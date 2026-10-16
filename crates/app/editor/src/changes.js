// The Changes panel: commit box, then Merge conflicts / Staged / Changes /
// Untracked, each as a flat list or a folder tree, with keyboard control.

import { api } from "./api.js";
import { S, emit, sections } from "./store.js";
import { h, clear, basename, dirname, prefs, ltr, plural, copyText } from "./util.js";
import { icon, fileIcon } from "./icons.js";
import { toast, toastError, confirmDialog, showMenu, showContextMenu } from "./ui.js";
import { gitOp, gitBusy, refreshStatus } from "./git.js";
import { openFile, activeTab, dirtyTabs, saveAll } from "./editor.js";

const els = {};
const state = { amend: false, amendPrefill: null, beforeAmend: "" };
const collapsed = {}; // section id -> folded (undefined: not decided yet)
const folded = new Set(); // "<section>:dir:<path>" folders folded in tree mode
let rows = []; // what the keyboard walks: {key, type: "head"|"dir"|"file", sec, files, file, el}
let cursor = null;
let cursorIndex = 0;
let cursorPlace = { sec: null, pos: 0 };

const SECTIONS = [
  { id: "conflicts", title: "Merge conflicts" },
  { id: "staged", title: "Staged" },
  { id: "changes", title: "Changes" },
  { id: "untracked", title: "Untracked" },
];
const TINT = { M: "modified", A: "added", "?": "untracked", D: "deleted", R: "renamed", C: "renamed", U: "conflict", T: "modified" };
const modeOf = (id) => prefs.get("gitview:" + id, id === "untracked" ? "tree" : "list");

// ------------------------------------------------------------------ mount

export function mountChanges(root) {
  els.root = root;
  els.msg = h("textarea", {
    class: "commit-input",
    rows: 1,
    spellcheck: true,
    "aria-label": "Commit message",
    onInput: () => {
      autosize();
      updateCommit();
    },
    onKeyDown: (e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commit({ push: e.shiftKey });
      } else if (e.key === "Escape") {
        e.preventDefault();
        els.list.focus();
      }
    },
  });
  els.amend = h("button", { class: "toggle", "aria-pressed": "false", title: "Replace the last commit (prefills its message)", onClick: () => setAmend(!state.amend) }, icon("pencil"), "Amend");
  els.commitMain = h("button", { class: "split-main", onClick: () => commit() }, "Commit");
  els.commitMore = h("button", { class: "split-more", title: "More Commit Options", onClick: (e) => commitMenu(e.currentTarget) }, icon("chevronDown"));
  els.split = h("div", { class: "split-btn" }, els.commitMain, els.commitMore);
  els.commit = h(
    "div",
    { class: "commit" },
    h("div", { class: "commit-field", onMouseDown: (e) => e.target === e.currentTarget && (e.preventDefault(), els.msg.focus()) }, els.msg),
    h("div", { class: "commit-row" }, els.amend, h("span", { class: "grow" }), els.split),
  );
  els.list = h("div", {
    class: "glist",
    tabIndex: 0,
    role: "tree",
    "aria-label": "Changes",
    onKeyDown: onKey,
    onFocus: () => markCursor(),
    onBlur: () => markCursor(),
  });
  els.empty = h(
    "div",
    { class: "panel-empty small" },
    h("div", { class: "panel-empty-ic" }, icon("check")),
    h("div", { class: "panel-empty-title" }, "No changes"),
    h("div", { class: "panel-empty-sub" }, "The working tree matches the last commit."),
  );
  els.nogit = h(
    "div",
    { class: "panel-empty" },
    h("div", { class: "panel-empty-ic" }, icon("git")),
    h("div", { class: "panel-empty-title" }, "Not a Git repository"),
    h("div", { class: "panel-empty-sub" }, "This folder isn't tracked by git, so there are no changes to show."),
  );
  root.append(els.commit, els.list, els.empty, els.nogit);
  new ResizeObserver(() => updateCommit()).observe(els.msg);
  renderChanges();
  requestAnimationFrame(autosize);
}

function autosize() {
  const m = els.msg;
  if (!m || !m.isConnected) return;
  const cs = getComputedStyle(m);
  const line = parseFloat(cs.lineHeight) || 18;
  const pad = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
  const max = Math.round(line * 8 + pad);
  m.style.height = "0px";
  const want = Math.max(Math.round(line + pad), m.scrollHeight);
  m.style.height = Math.min(want, max) + "px";
  m.style.overflowY = want > max + 1 ? "auto" : "hidden";
}

export function focusCommit() {
  els.msg?.focus();
}
export function focusChanges() {
  if (rows.length) els.list.focus();
  else els.msg?.focus();
}

// ------------------------------------------------------------------ commit button

function plan() {
  const { conflicts, staged, changes, untracked } = sections();
  const all = !state.amend && !staged.length;
  return { all, count: all ? changes.length + untracked.length : staged.length, conflicts: conflicts.length, staged: staged.length };
}

function updateCommit() {
  if (!els.commitMain) return;
  const p = plan();
  const branch = S.status?.branch;
  const busy = !!gitBusy();
  const msg = els.msg.value.trim();
  els.commitMain.textContent = state.amend ? "Amend" : p.all ? "Commit All" : "Commit";
  let why = null;
  if (p.conflicts) why = "Resolve the merge conflicts first";
  else if (!state.amend && !p.count) why = "Nothing to commit";
  else if (!msg) why = "Write a commit message first";
  els.commitMain.disabled = busy || !!why;
  els.split.classList.toggle("disabled", els.commitMain.disabled);
  els.commitMain.title = why || (p.all ? `Stage all ${plural(p.count, "change")} and commit (⌘↵)` : state.amend ? "Replace the last commit (⌘↵)" : `Commit ${plural(p.count, "staged file")} (⌘↵)`);
  els.commitMore.disabled = busy;
  els.amend.classList.toggle("on", state.amend);
  els.amend.setAttribute("aria-pressed", String(state.amend));
  // The placeholder must fit on one line, so it shortens with the panel.
  const w = els.msg.clientWidth || 240;
  const verb = state.amend ? "amend" : "commit";
  const full = `Message (⌘↵ to ${verb}${branch ? ` on ${branch}` : ""})`;
  els.msg.placeholder = full.length * 6.4 < w - 16 ? full : `Message (⌘↵ to ${verb})`.length * 6.4 < w - 16 ? `Message (⌘↵ to ${verb})` : "Message";
}

function commitMenu(anchor) {
  const p = plan();
  const verb = state.amend ? "Amend" : p.all ? "Commit All" : "Commit";
  const canPush = !!S.status?.branch;
  showMenu(
    anchor,
    [
      { label: `${verb} & ${S.status?.upstream || !canPush ? "Push" : "Publish"}`, icon: "up", hint: "⌘⇧↵", disabled: !canPush || els.commitMain.disabled, onSelect: () => commit({ push: true }) },
      "-",
      { label: "Amend Last Commit", icon: "pencil", checked: state.amend, onSelect: () => setAmend(!state.amend) },
    ],
    { align: "right" },
  );
}

async function setAmend(on) {
  if (on === state.amend) return;
  state.amend = on;
  if (on) {
    state.beforeAmend = els.msg.value;
    state.amendPrefill = null;
    if (!els.msg.value.trim()) {
      try {
        const r = await api("last_message");
        if (state.amend && !els.msg.value.trim()) {
          els.msg.value = String(r.message || "").replace(/\s+$/, "");
          state.amendPrefill = els.msg.value;
        }
      } catch (e) {
        toastError(e, "Couldn't read the last commit message");
      }
    }
  } else {
    if (state.amendPrefill !== null && els.msg.value === state.amendPrefill) els.msg.value = state.beforeAmend;
    state.amendPrefill = null;
  }
  autosize();
  updateCommit();
  els.msg.focus();
}

async function commit({ push = false } = {}) {
  if (gitBusy()) return;
  const amend = state.amend;
  const message = els.msg.value.replace(/\s+$/, "");
  if (!message.trim()) {
    toast("Write a commit message first.");
    els.msg.focus();
    return;
  }
  const dirty = dirtyTabs();
  if (dirty.length) {
    const c = await confirmDialog({
      title: dirty.length === 1 ? `Save “${dirty[0].name}” before committing?` : `Save ${dirty.length} files before committing?`,
      message: "Unsaved edits are not part of the commit.",
      items: dirty.length > 1 ? dirty.map((t) => t.path) : null,
      confirm: "Save",
      alt: "Don't Save",
    });
    if (!c) return;
    if (c === "confirm") {
      if (!(await saveAll())) return;
      await refreshStatus();
    }
  }
  const { conflicts, staged, changes, untracked } = sections();
  if (conflicts.length) {
    toast("Stage each conflicted file once it's resolved, then commit.", { kind: "error", title: "Unresolved conflicts" });
    return;
  }
  if (!staged.length && !amend) {
    const all = [...changes, ...untracked];
    if (!all.length) {
      toast("Nothing to commit.");
      return;
    }
    if (!prefs.get("commitAllOk", false)) {
      const c = await confirmDialog({
        title: "Commit all changes?",
        message: `Nothing is staged, so ${plural(all.length, "change")}${untracked.length ? `, including ${plural(untracked.length, "untracked file")},` : ""} will be staged and committed. You won't be asked again.`,
        items: all.map((f) => f.path),
        confirm: "Commit All",
      });
      if (!c) return;
      prefs.set("commitAllOk", true);
    }
    if (!(await gitOp("Staging…", "stage", { paths: all.map((f) => f.path) }))) return;
  }
  const label = push ? (amend ? "Amending & pushing…" : "Committing & pushing…") : amend ? "Amending…" : "Committing…";
  const r = await gitOp(label, "commit", { message, amend, push }, { refs: true });
  if (!r) return;
  els.msg.value = "";
  state.amend = false;
  state.amendPrefill = null;
  autosize();
  updateCommit();
  const first = String(r.output || "").trim().split("\n")[0];
  toast(first || message.split("\n")[0], { kind: "success", title: push ? (amend ? "Amended and pushed" : "Committed and pushed") : amend ? "Amended" : "Committed" });
}

// ------------------------------------------------------------------ render

export function renderChanges() {
  if (!els.root) return;
  const git = S.git;
  els.commit.style.display = git ? "" : "none";
  els.nogit.style.display = git ? "none" : "";
  if (!git) {
    els.list.style.display = "none";
    els.empty.style.display = "none";
    return;
  }
  renderList();
  updateCommit();
}

function renderList() {
  const secs = sections();
  const total = secs.conflicts.length + secs.staged.length + secs.changes.length + secs.untracked.length;
  const scroll = els.list.scrollTop;
  clear(els.list);
  rows = [];
  els.list.style.display = S.status && total ? "" : "none";
  els.empty.style.display = S.status && !total ? "" : "none";
  if (!S.status || !total) return;
  for (const s of SECTIONS) {
    const files = secs[s.id];
    if (!files.length) continue;
    if (collapsed[s.id] === undefined) collapsed[s.id] = s.id === "untracked" && files.length > 10;
    const sec = h("div", { class: "gsec gsec-" + s.id + (collapsed[s.id] ? " folded" : ""), role: "group" });
    sec.append(sectionHead(s, files));
    if (!collapsed[s.id]) {
      if (modeOf(s.id) === "tree") renderTree(sec, s.id, files);
      else for (const f of files) sec.append(fileRow(f, s.id, 0, false));
    }
    els.list.append(sec);
  }
  els.list.scrollTop = scroll;
  if (cursor && !rows.some((r) => r.key === cursor)) {
    // The row left (staged, discarded): stay at the same place in its section.
    const same = rows.filter((r) => r.sec === cursorPlace.sec);
    const next = same.length > 1 ? same[Math.max(1, Math.min(cursorPlace.pos, same.length - 1))] : rows[Math.min(cursorIndex, rows.length - 1)];
    cursor = next?.key || null;
  }
  markCursor();
}

function markCursor() {
  const focused = document.activeElement === els.list;
  rows.forEach((r, i) => {
    const on = r.key === cursor;
    if (on) {
      cursorIndex = i;
      cursorPlace = { sec: r.sec, pos: rows.filter((x) => x.sec === r.sec).indexOf(r) };
    }
    r.el.classList.toggle("cursor", on && focused);
  });
}

function act(ic, title, fn, cls = "") {
  return h(
    "button",
    {
      class: "icon-btn sm" + (cls ? " " + cls : ""),
      title,
      tabIndex: -1,
      disabled: !!gitBusy(),
      onClick: (e) => {
        e.stopPropagation();
        fn();
      },
    },
    icon(ic),
  );
}

function sectionActions(id, files) {
  const mode = modeOf(id);
  const out = [
    act(mode === "tree" ? "list" : "tree", mode === "tree" ? "View as List" : "View as Tree", () => {
      prefs.set("gitview:" + id, mode === "tree" ? "list" : "tree");
      renderList();
    }),
  ];
  if (id === "staged") out.push(act("minus", "Unstage All", () => unstage(files)));
  if (id === "changes") out.push(act("discard", "Discard All Changes…", () => discard(files)), act("plus", "Stage All Changes", () => stage(files)));
  if (id === "untracked") out.push(act("trash", "Delete All Untracked Files…", () => discard(files)), act("plus", "Stage All Untracked Files", () => stage(files)));
  if (id === "conflicts") out.push(act("check", "Mark All as Resolved", () => stage(files)));
  return out;
}

function sectionHead(s, files) {
  const key = s.id + ":";
  const el = h(
    "div",
    {
      class: "gsec-head",
      role: "treeitem",
      "aria-expanded": String(!collapsed[s.id]),
      "data-key": key,
      onClick: () => {
        setCursor(key);
        toggleSection(s.id);
      },
    },
    h("span", { class: "twisty" }, icon("chevron")),
    h("span", { class: "gsec-title" }, s.title),
    h("span", { class: "gsec-count" }, String(files.length)),
    h("span", { class: "grow" }),
    h("span", { class: "gsec-actions" }, sectionActions(s.id, files)),
  );
  rows.push({ key, type: "head", sec: s.id, files, el });
  return el;
}

function toggleSection(id, open = !!collapsed[id]) {
  collapsed[id] = !open;
  renderList();
}

// Folder tree with single-child chains joined ("crates/app/src").
function buildTree(files) {
  const mk = (name, path) => ({ name, path, dirs: new Map(), files: [], entry: null });
  const root = mk("", "");
  for (const f of files) {
    const isDir = f.path.endsWith("/");
    const parts = f.path.replace(/\/$/, "").split("/");
    let node = root;
    const last = parts.length - 1;
    for (let i = 0; i < last; i++) {
      const p = parts.slice(0, i + 1).join("/");
      if (!node.dirs.has(parts[i])) node.dirs.set(parts[i], mk(parts[i], p));
      node = node.dirs.get(parts[i]);
    }
    if (isDir) {
      if (!node.dirs.has(parts[last])) node.dirs.set(parts[last], mk(parts[last], parts.join("/")));
      node.dirs.get(parts[last]).entry = f;
    } else node.files.push(f);
  }
  const finish = (n) => {
    for (const d of n.dirs.values()) {
      while (d.dirs.size === 1 && !d.files.length && !d.entry) {
        const [only] = d.dirs.values();
        if (only.entry && !only.dirs.size) break;
        d.name += "/" + only.name;
        d.path = only.path;
        d.dirs = only.dirs;
        d.files = only.files;
        d.entry = only.entry;
      }
      finish(d);
    }
    n.all = [...n.files, ...[...n.dirs.values()].flatMap((d) => (d.entry ? [d.entry, ...d.all] : d.all))];
    n.dirList = [...n.dirs.values()].sort((a, b) => a.name.localeCompare(b.name));
    n.files.sort((a, b) => basename(a.path).localeCompare(basename(b.path)));
  };
  finish(root);
  return root;
}

function renderTree(box, sec, files) {
  const walk = (node, depth) => {
    for (const d of node.dirList) {
      if (d.entry && !d.dirs.size && !d.files.length) {
        box.append(fileRow(d.entry, sec, depth, true));
        continue;
      }
      const key = `${sec}:dir:${d.path}`;
      const isFolded = folded.has(key);
      box.append(dirRow(d, sec, depth, key, isFolded));
      if (!isFolded) walk(d, depth + 1);
    }
    for (const f of node.files) box.append(fileRow(f, sec, depth, true));
  };
  walk(buildTree(files), 0);
}

function fileActions(sec, files, { open = null } = {}) {
  const out = [];
  if (open) out.push(act("openFile", "Open File", open));
  if (sec === "changes") out.push(act("discard", "Discard Changes…", () => discard(files)));
  if (sec === "untracked") out.push(act("trash", "Delete…", () => discard(files)));
  if (sec === "staged") out.push(act("minus", "Unstage", () => unstage(files)));
  else if (sec === "conflicts") out.push(act("check", "Mark as Resolved", () => stage(files)));
  else out.push(act("plus", "Stage", () => stage(files)));
  return out;
}

function dirRow(d, sec, depth, key, isFolded) {
  const files = d.all;
  const el = h(
    "div",
    {
      class: "crow dir" + (isFolded ? "" : " expanded"),
      role: "treeitem",
      "aria-expanded": String(!isFolded),
      style: `--depth:${depth}`,
      "data-key": key,
      title: d.path,
      onClick: () => {
        setCursor(key);
        toggleFolder(key);
      },
      onContextMenu: (e) => rowMenu(e, sec, files, d.path + "/"),
    },
    h("span", { class: "twisty" }, icon("chevron")),
    icon(isFolded ? "folder" : "folderOpen", "crow-ic"),
    h("span", { class: "crow-label" }, h("span", { class: "crow-name" }, d.name)),
    h("span", { class: "crow-end" }, h("span", { class: "crow-actions" }, fileActions(sec, files)), h("span", { class: "crow-count" }, String(files.length))),
  );
  rows.push({ key, type: "dir", sec, files, el, path: d.path });
  return el;
}

function toggleFolder(key, open = folded.has(key)) {
  if (open) folded.delete(key);
  else folded.add(key);
  renderList();
}

function letterFor(f, sec) {
  if (sec === "conflicts") return "U";
  if (sec === "staged") return f.staged;
  return f.unstaged;
}

function fileRow(f, sec, depth, inTree) {
  const isDir = f.path.endsWith("/");
  const path = f.path.replace(/\/$/, "");
  const name = basename(path);
  const dir = dirname(path);
  const letter = letterFor(f, sec);
  const key = `${sec}:${f.path}`;
  const open = !isDir && activeTab()?.path === path;
  const renamed = f.orig && f.orig !== path;
  let nameEl = name + (isDir ? "/" : "");
  let dirText = dir;
  if (renamed) {
    const oldName = basename(f.orig), oldDir = dirname(f.orig);
    if (oldName !== name) nameEl = [h("span", { class: "crow-old" }, oldName), h("span", { class: "crow-arrow" }, " → "), name];
    if (oldDir !== dir) dirText = `${oldDir || "."} → ${dir || "."}`;
  }
  const canOpen = !isDir && letter !== "D";
  const el = h(
    "div",
    {
      class: "crow file" + (open ? " open" : "") + (letter === "D" ? " deleted" : "") + (inTree ? " in-tree" : ""),
      role: "treeitem",
      style: `--depth:${depth}`,
      "data-key": key,
      title: (f.orig ? `${f.orig} → ${path}` : f.path) + statusWord(letter),
      onClick: () => {
        setCursor(key);
        openChange(f, sec, false);
      },
      onDblClick: () => canOpen && openFile(path, { diff: false }),
      onContextMenu: (e) => rowMenu(e, sec, [f], f.path),
    },
    h("span", { class: "twisty" }),
    icon(isDir ? "folder" : fileIcon(name), "crow-ic"),
    h("span", { class: "crow-label" }, h("span", { class: "crow-name" }, nameEl), (!inTree || renamed) && dirText && h("span", { class: "crow-dir" }, ltr(dirText))),
    h(
      "span",
      { class: "crow-end" },
      h("span", { class: "crow-actions" }, fileActions(sec, [f], { open: canOpen ? () => openFile(path, { diff: false }) : null })),
      h("span", { class: "letter tint-" + (TINT[letter] || "modified") }, letter),
    ),
  );
  rows.push({ key, type: "file", sec, files: [f], file: f, el });
  return el;
}

function statusWord(letter) {
  const w = { M: "modified", A: "added", D: "deleted", R: "renamed", C: "copied", U: "conflicted", "?": "untracked", T: "type changed" }[letter];
  return w ? ` (${w})` : "";
}

function rowMenu(e, sec, files, path) {
  const one = files.length === 1 ? files[0] : null;
  const p = path.replace(/\/$/, "");
  const isDir = path.endsWith("/");
  const letter = one ? letterFor(one, sec) : "";
  showContextMenu(e, [
    one && !isDir && letter !== "D" && { label: "Open File", icon: "openFile", onSelect: () => openFile(p, { diff: false }) },
    one && !isDir && { label: "Open Changes", icon: "diff", onSelect: () => openChange(one, sec, true) },
    { label: "Reveal in Files", icon: "folder", onSelect: () => emit("reveal", p) },
    "-",
    sec === "staged"
      ? { label: "Unstage", icon: "minus", onSelect: () => unstage(files) }
      : { label: sec === "conflicts" ? "Mark as Resolved" : "Stage", icon: sec === "conflicts" ? "check" : "plus", onSelect: () => stage(files) },
    sec === "changes" && { label: "Discard Changes…", icon: "discard", danger: true, onSelect: () => discard(files) },
    sec === "untracked" && { label: "Delete…", icon: "trash", danger: true, onSelect: () => discard(files) },
    "-",
    { label: "Copy Path", icon: "copy", onSelect: () => copyText(p) },
  ]);
}

function openChange(f, sec, focus) {
  const path = f.path.replace(/\/$/, "");
  if (f.path.endsWith("/")) {
    emit("reveal", path);
    return;
  }
  const both = f.staged && f.unstaged && f.unstaged !== "?";
  openFile(path, { diff: sec !== "conflicts", against: sec === "changes" && both ? "index" : "head", focus });
}

// ------------------------------------------------------------------ keyboard

function setCursor(key, scroll = false) {
  cursor = key;
  markCursor();
  if (scroll) rows.find((r) => r.key === key)?.el.scrollIntoView({ block: "nearest" });
}

function move(i) {
  if (!rows.length) return;
  const r = rows[Math.max(0, Math.min(rows.length - 1, i))];
  setCursor(r.key, true);
}

function onKey(e) {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  let i = rows.findIndex((r) => r.key === cursor);
  const r = rows[i];
  const stop = () => {
    e.preventDefault();
    e.stopPropagation();
  };
  switch (e.key) {
    case "ArrowDown":
      stop();
      move(i < 0 ? 0 : i + 1);
      break;
    case "ArrowUp":
      stop();
      move(i < 0 ? 0 : i - 1);
      break;
    case "Home":
      stop();
      move(0);
      break;
    case "End":
      stop();
      move(rows.length - 1);
      break;
    case "ArrowRight":
      stop();
      if (r?.type === "head" && collapsed[r.sec]) toggleSection(r.sec, true);
      else if (r?.type === "dir" && folded.has(r.key)) toggleFolder(r.key, true);
      else if (r && r.type !== "file") move(i + 1);
      break;
    case "ArrowLeft": {
      stop();
      if (r?.type === "head" && !collapsed[r.sec]) toggleSection(r.sec, false);
      else if (r?.type === "dir" && !folded.has(r.key)) toggleFolder(r.key, false);
      else if (r) {
        // Up to the enclosing folder, or the section header.
        const depth = parseInt(r.el.style.getPropertyValue("--depth") || "0", 10);
        for (let j = i - 1; j >= 0; j--) {
          const x = rows[j];
          if (x.sec !== r.sec) break;
          const xd = parseInt(x.el.style.getPropertyValue("--depth") || "0", 10);
          if (x.type === "head" || (x.type === "dir" && xd < depth)) {
            move(j);
            break;
          }
        }
      }
      break;
    }
    case " ":
      stop();
      if (!r) break;
      if (r.sec === "staged") unstage(r.files);
      else stage(r.files);
      break;
    case "Enter":
      stop();
      if (!r) break;
      if (r.type === "head") toggleSection(r.sec);
      else if (r.type === "dir") toggleFolder(r.key);
      else openChange(r.file, r.sec, true);
      break;
    case "Backspace":
    case "Delete":
      stop();
      if (r && (r.sec === "changes" || r.sec === "untracked")) discard(r.files);
      break;
    case "Escape":
      if (document.activeElement === els.list) {
        stop();
        els.msg.focus();
      }
      break;
  }
}

// ------------------------------------------------------------------ actions

const pathsOf = (files, withOrig = false) => [...new Set(files.flatMap((f) => (withOrig && f.orig ? [f.path, f.orig] : [f.path])))];

function stage(files) {
  return gitOp("Staging…", "stage", { paths: pathsOf(files) });
}

function unstage(files) {
  return gitOp("Unstaging…", "unstage", { paths: pathsOf(files, true) });
}

async function discard(files) {
  const n = files.length;
  if (!n) return;
  const untracked = files.filter((f) => f.unstaged === "?").length;
  const one = basename(files[0].path.replace(/\/$/, ""));
  let message;
  if (untracked === n) message = n === 1 ? "This untracked file is deleted. This can't be undone." : `These ${n} untracked files are deleted. This can't be undone.`;
  else if (untracked) message = `Tracked files go back to their last committed or staged state; ${plural(untracked, "untracked file")} ${untracked === 1 ? "is" : "are"} deleted. This can't be undone.`;
  else message = n === 1 ? "The file goes back to its last committed or staged state. This can't be undone." : "These files go back to their last committed or staged state. This can't be undone.";
  const ok = await confirmDialog({
    title: n === 1 ? `${untracked ? "Delete" : "Discard changes to"} “${one}”?` : `${untracked === n ? "Delete" : "Discard changes to"} ${n} files?`,
    message,
    items: files.map((f) => f.path),
    confirm: untracked === n ? "Delete" : "Discard",
    danger: true,
  });
  if (!ok) return;
  await gitOp("Discarding…", "discard", { paths: pathsOf(files) }, { after: (r) => r && emit("disk") });
}
