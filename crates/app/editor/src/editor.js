// Tabs, the CodeMirror editors behind them, saving, disk sync and diffs.

import { EditorState, Compartment, Text, EditorSelection } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLineGutter,
  highlightSpecialChars,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
  highlightActiveLine,
  scrollPastEnd,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { search, searchKeymap, highlightSelectionMatches, openSearchPanel } from "@codemirror/search";
import { indentOnInput, bracketMatching, foldGutter, foldKeymap, indentUnit } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { unifiedMergeView, MergeView, Chunk, goToNextChunk, goToPreviousChunk, getChunks } from "@codemirror/merge";

import { api } from "./api.js";
import { S, emit, tintOf } from "./store.js";
import { h, clear, basename, dirname, debounce, formatSize, prefs, ltr } from "./util.js";
import { icon, fileIcon } from "./icons.js";
import { toast, toastError, confirmDialog } from "./ui.js";
import { editorTheme, highlight, currentTheme } from "./theme.js";
import { languageFor, WRAP } from "./languages.js";
import { controlGuard } from "./inputguard.js";

const tabs = [];
let active = null;
let mru = []; // most recently activated paths, newest first
const els = {};

const DIFF_CONF = { scanLimit: 5000, timeout: 400 };

const phrases = EditorState.phrases.of({
  Find: "Find",
  Replace: "Replace",
  next: "↓",
  previous: "↑",
  all: "All",
  "match case": "Aa",
  regexp: ".*",
  "by word": "W",
  replace: "Replace",
  "replace all": "Replace All",
  close: "Close",
});

// ------------------------------------------------------------------ mount

export function mountEditor(root) {
  els.tabs = h("div", { class: "tabs", role: "tablist" });
  els.tabbar = h("div", { class: "tabbar" }, els.tabs);
  els.crumbs = h("div", { class: "crumbs" });
  els.tools = h("div", { class: "tools" });
  els.crumbbar = h("div", { class: "crumbbar" }, els.crumbs, els.tools);
  els.banner = h("div", { class: "banner-slot" });
  els.empty = emptyState();
  els.stack = h("div", { class: "stack" }, els.empty);
  root.append(els.tabbar, els.crumbbar, els.banner, els.stack);
  els.tabs.addEventListener("wheel", (e) => {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
      els.tabs.scrollLeft += e.deltaY;
      e.preventDefault();
    }
  }, { passive: false });
  render();
}

function emptyState() {
  els.recent = h("div", { class: "recent" });
  const row = (label, ...keys) => h("div", { class: "hint-row" }, h("span", null, label), h("span", { class: "keys" }, keys.map((k) => h("kbd", null, k))));
  return h(
    "div",
    { class: "empty" },
    h(
      "div",
      { class: "empty-inner" },
      els.recent,
      h(
        "div",
        { class: "hints" },
        row("Quick Open", "⌘", "P"),
        row("Files", "⌘", "⇧", "E"),
        row("Changes", "⌘", "⇧", "G"),
        row("History", "⌘", "⇧", "H"),
        row("Find in File", "⌘", "F"),
      ),
    ),
  );
}

// Files opened lately in this project, newest first (kept across reloads).
const recentKey = () => "recent:" + (S.root || "");
function rememberRecent(path) {
  const list = prefs.get(recentKey(), []).filter((p) => typeof p === "string" && p !== path);
  prefs.set(recentKey(), [path, ...list].slice(0, 12));
}

function renderEmpty() {
  if (!els.recent) return;
  clear(els.recent);
  const open = new Set(tabs.map((t) => t.path));
  const list = prefs.get(recentKey(), []).filter((p) => typeof p === "string" && !open.has(p)).slice(0, 6);
  if (!list.length) {
    els.recent.append(
      h("div", { class: "empty-mark" }, icon("code")),
      h("div", { class: "empty-title" }, "No file open"),
      h("div", { class: "empty-sub" }, "Pick a file from the sidebar, or jump straight to one."),
    );
    return;
  }
  els.recent.append(
    h("div", { class: "recent-title" }, "Recent files"),
    h(
      "div",
      { class: "recent-list" },
      list.map((p) =>
        h(
          "button",
          { class: "recent-row", title: p, onClick: () => openFile(p) },
          icon(fileIcon(basename(p)), "recent-ic"),
          h("span", { class: "recent-name" }, basename(p)),
          dirname(p) && h("span", { class: "recent-dir" }, ltr(dirname(p))),
        ),
      ),
    ),
  );
}

// ------------------------------------------------------------------ queries

export const activeTab = () => active;
export const allTabs = () => tabs;
export const recentPaths = () => mru.filter((p) => !p.startsWith("commit:"));
export const dirtyTabs = () => tabs.filter((t) => t.dirty);
const cur = (tab) => (tab.split ? tab.split.b : tab.view);
export function activeView() {
  if (active?.kind === "custom") return active.ctl?.view?.() || null;
  return active && active.view ? cur(active) : null;
}

function statusSig(path) {
  const e = S.byPath.get(path);
  return e ? `${e.staged}|${e.unstaged}|${e.orig || ""}` : "";
}
function diffAvailable(tab) {
  return S.git && (tab.kind === "text" || tab.kind === "deleted") && (S.byPath.has(tab.path) || tab.kind === "deleted");
}
function againstOptions(tab) {
  const e = S.byPath.get(tab.path);
  if (e && e.staged && e.unstaged && e.unstaged !== "?") return ["head", "index"];
  return ["head"];
}

// ------------------------------------------------------------------ text helpers

const splitLines = (s) => s.split(/\r\n?|\n/);
function toText(str, sep) {
  return Text.of(sep === "\r\n" ? str.split("\r\n") : splitLines(str));
}

function detectIndent(text) {
  let tabsN = 0, spaced = 0, prev = 0;
  const widths = { 2: 0, 4: 0, 8: 0 };
  const lines = text.split("\n", 3000);
  for (const line of lines) {
    if (!line.trim()) continue;
    if (line[0] === "\t") {
      tabsN++;
      continue;
    }
    const n = line.length - line.trimStart().length;
    if (n) spaced++;
    const d = Math.abs(n - prev);
    if (d in widths) widths[d]++;
    prev = n;
  }
  if (tabsN > spaced) return "\t";
  const best = Object.entries(widths).sort((a, b) => b[1] - a[1])[0];
  return best && best[1] > 0 ? " ".repeat(+best[0]) : "    ";
}

// Replace a view's document with `doc`, touching only the span that differs
// so the cursor and scroll position survive (and the change is undoable).
function syncDoc(view, doc, selection) {
  const old = view.state.doc;
  if (old.eq(doc)) {
    if (selection) view.dispatch({ selection });
    return;
  }
  const a = old.toString(), b = doc.toString();
  let s = 0;
  const max = Math.min(a.length, b.length);
  while (s < max && a.charCodeAt(s) === b.charCodeAt(s)) s++;
  let e = 0;
  while (e < max - s && a.charCodeAt(a.length - 1 - e) === b.charCodeAt(b.length - 1 - e)) e++;
  view.dispatch({
    changes: { from: s, to: a.length - e, insert: doc.slice(s, b.length - e) },
    selection,
    userEvent: "input.reload",
  });
}

// ------------------------------------------------------------------ extensions

function editingExtensions(tab, readOnly, inSplit = false) {
  const c = tab.comp;
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    history(),
    foldGutter({ markerDOM: (open) => icon(open ? "chevronDown" : "chevron", "fold-ic") }),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    highlight,
    bracketMatching(),
    closeBrackets(),
    rectangularSelection(),
    crosshairCursor(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    search({ top: true }),
    inSplit ? [] : scrollPastEnd(),
    keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...foldKeymap, indentWithTab]),
    controlGuard,
    editorTheme,
    phrases,
    EditorState.tabSize.of(4),
    tab.lineSep === "\r\n" ? EditorState.lineSeparator.of("\r\n") : [],
    c.dark.of(EditorView.darkTheme.of(!!currentTheme.dark)),
    c.lang.of(tab.lang.support()),
    c.wrap.of(WRAP.has(tab.lang.key) ? EditorView.lineWrapping : []),
    c.indent.of(indentUnit.of(tab.indent)),
    c.ro.of(readOnly ? [EditorState.readOnly.of(true)] : []),
    EditorView.updateListener.of((u) => onUpdate(tab, u)),
  ];
}

function baseExtensions(tab) {
  const c = tab.comp;
  return [
    lineNumbers(),
    highlightSpecialChars(),
    drawSelection(),
    highlight,
    editorTheme,
    phrases,
    search({ top: true }),
    keymap.of([...defaultKeymap, ...searchKeymap]),
    controlGuard,
    EditorState.tabSize.of(4),
    EditorState.readOnly.of(true),
    c.dark.of(EditorView.darkTheme.of(!!currentTheme.dark)),
    c.lang.of(tab.lang.support()),
    c.wrap.of(WRAP.has(tab.lang.key) ? EditorView.lineWrapping : []),
  ];
}

function onUpdate(tab, u) {
  if (u.docChanged && tab.disk) {
    const dirty = !u.state.doc.eq(tab.disk);
    if (dirty !== tab.dirty) {
      tab.dirty = dirty;
      renderTabs();
      emit("tabs");
    }
    tab.scheduleStats();
  }
  if (tab === active && (u.selectionSet || u.docChanged || u.focusChanged)) renderCursor();
}

// ------------------------------------------------------------------ tabs

function createTab(path) {
  const tab = {
    path,
    name: basename(path),
    kind: "loading",
    size: 0,
    error: null,
    host: h("div", { class: "host" }),
    view: null,
    split: null,
    disk: null,
    lineSep: "\n",
    indent: "    ",
    lang: languageFor(path),
    dirty: false,
    pendingDisk: null,
    diff: { on: false, mode: "inline", against: "head", fold: false },
    bases: {},
    baseReq: {},
    baseGen: 0,
    statusSig: statusSig(path),
    applied: null, // what the view currently shows: {base, mode, fold}
    stats: null,
    comp: { dark: new Compartment(), lang: new Compartment(), wrap: new Compartment(), indent: new Compartment(), ro: new Compartment(), diff: new Compartment() },
    loading: null,
  };
  tab.scheduleStats = debounce(() => computeStats(tab), 250);
  els.stack.append(tab.host);
  return tab;
}

function destroyViews(tab) {
  tab.ctl?.destroy?.();
  tab.ctl = null;
  if (tab.split) {
    tab.split.destroy();
    tab.split = null;
  }
  tab.view?.destroy();
  tab.view = null;
  tab.applied = null;
  tab.host.classList.remove("splitting");
  clear(tab.host);
}

async function loadTab(tab) {
  const entry = S.byPath.get(tab.path);
  let res = null, err = null;
  try {
    res = await api("read", { path: tab.path });
  } catch (e) {
    err = e;
  }
  if (!tabs.includes(tab)) return;
  destroyViews(tab);
  tab.dirty = false;
  tab.pendingDisk = null;
  tab.statusSig = statusSig(tab.path);
  if (err) {
    const deleted = entry && (entry.unstaged === "D" || (entry.staged === "D" && !entry.unstaged));
    if (deleted && S.git) {
      tab.kind = "deleted";
      tab.disk = Text.empty;
      tab.indent = "    ";
      tab.diff = { ...tab.diff, on: true, against: entry.unstaged === "D" ? "index" : "head" };
      createView(tab, "", true);
    } else {
      tab.kind = "error";
      tab.error = err.message;
      tab.host.append(messageView("warning", "Couldn't open this file", err.message, h("button", { class: "btn", onClick: () => reload(tab) }, "Try Again")));
    }
  } else if (res.binary) {
    tab.kind = "binary";
    tab.size = res.size || 0;
    tab.host.append(messageView("binary", "Binary file", `${formatSize(tab.size)} · not shown in the editor`));
  } else if (res.too_large) {
    tab.kind = "too_large";
    tab.size = res.size || 0;
    tab.host.append(messageView("warning", "File is too large to open", `${formatSize(tab.size)} is over the editor's limit`));
  } else {
    const text = res.text ?? "";
    tab.kind = "text";
    tab.size = res.size || text.length;
    tab.lineSep = text.includes("\r\n") ? "\r\n" : "\n";
    tab.indent = detectIndent(text);
    tab.disk = toText(text, tab.lineSep);
    createView(tab, tab.disk, false);
  }
  resetBases(tab);
  tab.stats = null;
  if (tab.kind !== "deleted") tab.diff.on = tab.diff.on && diffAvailable(tab);
  if (tab.diff.on || diffAvailable(tab)) await applyDiff(tab);
  render();
}

function reload(tab) {
  tab.kind = "loading";
  render();
  tab.loading = loadTab(tab);
  return tab.loading;
}

function createView(tab, doc, readOnly) {
  tab.view = new EditorView({
    state: EditorState.create({ doc, extensions: [editingExtensions(tab, readOnly), tab.comp.diff.of([])] }),
    parent: tab.host,
  });
}

function messageView(ic, title, detail, action) {
  return h(
    "div",
    { class: "message" },
    h("div", { class: "message-ic" }, icon(ic)),
    h("div", { class: "message-title" }, title),
    detail && h("div", { class: "message-detail" }, detail),
    action,
  );
}

// A tab that isn't a file (the commit view). spec: {path, reuse, name, title,
// icon, label(), mount(host, tab) -> {destroy, view, setDark, remeasure,
// focus}}. A tab with the same `reuse` key is recycled instead of adding one.
export function openCustomTab(spec) {
  let tab = tabs.find((t) => t.path === spec.path) || (spec.reuse && tabs.find((t) => t.reuse === spec.reuse));
  if (tab && tab.path === spec.path && tab.ctl) {
    activate(tab);
    tab.ctl.focus?.();
    return tab;
  }
  if (tab) {
    destroyViews(tab);
    mru = mru.filter((p) => p !== tab.path);
  } else {
    tab = createTab(spec.path);
    const i = active ? tabs.indexOf(active) + 1 : tabs.length;
    tabs.splice(i, 0, tab);
  }
  Object.assign(tab, { path: spec.path, reuse: spec.reuse || null, kind: "custom", name: spec.name, title: spec.title, icon: spec.icon, label: spec.label, dirty: false, statusSig: "" });
  tab.update = (u) => {
    Object.assign(tab, u);
    renderTabs();
    emit("title");
  };
  tab.ctl = spec.mount(tab.host, tab);
  activate(tab, { focus: false });
  return tab;
}

export async function openFile(path, opts = {}) {
  path = String(path || "").replace(/^\/+/, "").replace(/\/+$/, "");
  if (!path) return;
  let tab = tabs.find((t) => t.path === path);
  if (!tab) {
    tab = createTab(path);
    const i = active ? tabs.indexOf(active) + 1 : tabs.length;
    tabs.splice(i, 0, tab);
    activate(tab, { focus: false });
    tab.loading = loadTab(tab);
  } else activate(tab, { focus: false });
  await tab.loading;
  if (!tabs.includes(tab)) return;
  if (opts.against && againstOptions(tab).includes(opts.against)) tab.diff.against = opts.against;
  if (opts.diff === true && diffAvailable(tab) && !tab.diff.on) {
    tab.diff.on = true;
    await applyDiff(tab);
    jumpToFirstChange(tab);
  } else if (opts.diff === true && tab.diff.on) {
    await applyDiff(tab);
    jumpToFirstChange(tab);
  } else if (opts.diff === false && tab.diff.on && tab.kind !== "deleted") {
    tab.diff.on = false;
    await applyDiff(tab);
  }
  if (opts.line) gotoLine(tab, opts.line, opts.col);
  render();
  if (opts.focus !== false && active === tab) focusEditor();
}

export function activate(tab, { focus = true } = {}) {
  if (!tab) return;
  active = tab;
  if (tab.kind !== "custom") rememberRecent(tab.path);
  mru = [tab.path, ...mru.filter((p) => p !== tab.path)].slice(0, 50);
  for (const t of tabs) t.host.classList.toggle("on", t === tab);
  render();
  emit("active", tab.path);
  if (focus) focusEditor();
  if (diffAvailable(tab) && !tab.bases[tab.diff.against]) applyDiff(tab);
}

export async function closeTab(tab) {
  if (!tab) return;
  if (tab.dirty) {
    const choice = await confirmDialog({
      title: `Save changes to “${tab.name}”?`,
      message: "Your changes will be lost if you don't save them.",
      confirm: "Save",
      alt: "Don't Save",
    });
    if (!choice) return;
    if (choice === "confirm" && !(await save(tab))) return;
  }
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  destroyViews(tab);
  tab.host.remove();
  mru = mru.filter((p) => p !== tab.path);
  if (active === tab) {
    active = null;
    const next = tabs.find((t) => t.path === mru[0]) || tabs[Math.min(i, tabs.length - 1)];
    if (next) activate(next);
    else {
      render();
      emit("active", null);
    }
  } else render();
  emit("tabs");
}

export function closeActive() {
  if (active) closeTab(active);
}

export function cycleTab(dir) {
  if (tabs.length < 2 || !active) return;
  const i = tabs.indexOf(active);
  activate(tabs[(i + dir + tabs.length) % tabs.length]);
}

export function focusEditor() {
  const v = activeView();
  if (v) requestAnimationFrame(() => v.focus());
}

export function openFind() {
  const v = activeView();
  if (!v) return false;
  v.focus();
  openSearchPanel(v);
  return true;
}

function gotoLine(tab, line, col) {
  const v = tab.view && cur(tab);
  if (!v) return;
  const n = Math.max(1, Math.min(v.state.doc.lines, Math.floor(line)));
  const l = v.state.doc.line(n);
  const pos = Math.min(l.to, l.from + Math.max(0, (col || 1) - 1));
  v.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
}

function jumpToFirstChange(tab) {
  const v = tab.view && cur(tab);
  if (!v) return;
  const chunks = getChunks(v.state)?.chunks;
  if (!chunks || !chunks.length) return;
  const pos = Math.min(chunks[0].fromB, v.state.doc.length);
  v.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
}

// ------------------------------------------------------------------ save & disk sync

export async function save(tab = active) {
  if (!tab || tab.kind !== "text" || !tab.view) return false;
  const v = cur(tab);
  const doc = v.state.doc;
  try {
    await api("write", { path: tab.path, text: v.state.sliceDoc() });
  } catch (e) {
    toastError(e, `Couldn't save ${tab.name}`);
    return false;
  }
  tab.disk = doc;
  tab.pendingDisk = null;
  tab.dirty = !cur(tab).state.doc.eq(doc);
  render();
  emit("saved", tab.path);
  return true;
}

export async function saveAll() {
  for (const t of dirtyTabs()) if (!(await save(t))) return false;
  return true;
}

async function checkDisk(tab) {
  if (tab.kind === "loading") return;
  if (tab.kind === "deleted") {
    const e = S.byPath.get(tab.path);
    if (!(e && (e.unstaged === "D" || (e.staged === "D" && !e.unstaged)))) await reload(tab);
    return;
  }
  if (tab.kind === "error") {
    // Retry quietly; only swap the view once the file can be read again.
    try {
      await api("read", { path: tab.path });
    } catch {
      return;
    }
    await reload(tab);
    return;
  }
  if (tab.kind !== "text") return;
  let r;
  try {
    r = await api("read", { path: tab.path });
  } catch {
    if (!tab.dirty && S.byPath.get(tab.path)?.unstaged === "D") await reload(tab);
    return;
  }
  if (!tabs.includes(tab) || tab.kind !== "text" || r.binary || r.too_large || typeof r.text !== "string") return;
  const disk = toText(r.text, tab.lineSep);
  if (disk.eq(tab.disk)) {
    if (tab.pendingDisk) {
      tab.pendingDisk = null;
      renderBanner();
    }
    return;
  }
  if (!tab.dirty) {
    tab.disk = disk;
    syncDoc(cur(tab), disk);
    tab.scheduleStats();
  } else if (!tab.pendingDisk || !tab.pendingDisk.eq(disk)) {
    tab.pendingDisk = disk;
    renderBanner();
  }
}

export async function refreshOpenFiles({ dropBases = false } = {}) {
  await Promise.all(
    tabs.map(async (t) => {
      if (dropBases) resetBases(t);
      await checkDisk(t);
      if (dropBases && tabs.includes(t) && diffAvailable(t)) await applyDiff(t);
    }),
  );
  render();
}

// Called when git status changed: refresh bases and toolbar for affected tabs.
export async function onStatus() {
  await Promise.all(
    tabs.map(async (t) => {
      const sig = statusSig(t.path);
      if (sig === t.statusSig) return;
      t.statusSig = sig;
      resetBases(t);
      if (!againstOptions(t).includes(t.diff.against)) t.diff.against = "head";
      await checkDisk(t);
      if (!tabs.includes(t) || t.kind === "loading") return;
      if (!diffAvailable(t)) {
        t.stats = null;
        if (t.diff.on) {
          t.diff.on = false;
          await applyDiff(t);
        }
      } else if (t.diff.on || t === active) await applyDiff(t);
    }),
  );
  render();
}

// ------------------------------------------------------------------ diffs

function resetBases(tab) {
  tab.bases = {};
  tab.baseReq = {};
  tab.baseGen = (tab.baseGen || 0) + 1;
}

// Fetch (once) the committed or staged text a tab is compared against.
async function ensureBase(tab, against) {
  if (tab.bases[against]) return tab.bases[against];
  const gen = tab.baseGen;
  if (!tab.baseReq[against]) {
    tab.baseReq[against] = api("base", { path: tab.path, against }).then((r) => ({
      text: Text.of(splitLines(r.text || "")),
      exists: r.exists !== false,
    }));
  }
  try {
    const base = await tab.baseReq[against];
    if (tab.baseGen === gen) tab.bases[against] = base;
    return base;
  } finally {
    if (tab.baseGen === gen) delete tab.baseReq[against];
  }
}

function revertControl(type, action) {
  if (type === "accept") return h("span", { hidden: true });
  const b = h("button", { class: "chunk-revert", title: "Revert this change", type: "button" }, icon("discard"), "Revert");
  b.onmousedown = action;
  return b;
}

// Bring the tab's views in line with tab.diff (on/off, inline/split, base, fold).
export async function applyDiff(tab) {
  if (!tab.view) return;
  const d = tab.diff;
  let base = null;
  if (diffAvailable(tab)) {
    try {
      base = await ensureBase(tab, d.against);
    } catch (e) {
      if (d.on) toastError(e, "Couldn't load the diff base");
      d.on = tab.kind === "deleted";
    }
  }
  if (!tabs.includes(tab) || !tab.view) return;
  const want = d.on && base ? { base, mode: d.mode, fold: d.fold } : null;
  const have = tab.applied;
  const same = want && have && have.base === want.base && have.mode === want.mode && have.fold === want.fold;
  if (!same && (want || have)) {
    const readOnly = tab.kind !== "text";
    const fold = d.fold ? { margin: 3, minSize: 4 } : undefined;
    if (tab.split && (!want || want.mode !== "split")) exitSplit(tab);
    if (want && want.mode === "inline") {
      tab.view.dispatch({
        effects: tab.comp.diff.reconfigure(
          unifiedMergeView({
            original: base.text,
            mergeControls: base.exists && !readOnly ? revertControl : false,
            gutter: true,
            highlightChanges: true,
            syntaxHighlightDeletions: true,
            collapseUnchanged: fold,
            diffConfig: DIFF_CONF,
          }),
        ),
      });
    } else if (have && have.mode === "inline") {
      tab.view.dispatch({ effects: tab.comp.diff.reconfigure([]) });
    }
    if (want && want.mode === "split") {
      if (tab.split && have && have.base !== base) {
        tab.split.a.dispatch({ changes: { from: 0, to: tab.split.a.state.doc.length, insert: base.text } });
      }
      if (tab.split) {
        tab.split.reconfigure({ collapseUnchanged: fold, revertControls: base.exists && !readOnly ? "a-to-b" : undefined });
        if (tab.splitLabel) tab.splitLabel.textContent = d.against === "index" ? "Staged" : "HEAD";
      }
      else enterSplit(tab, base, readOnly, fold);
    }
    tab.applied = want;
  }
  computeStats(tab);
  if (tab === active) render();
}

function enterSplit(tab, base, readOnly, fold) {
  const v = tab.view;
  const wrap = h("div", { class: "split-host" });
  tab.host.append(wrap);
  tab.host.classList.add("splitting");
  tab.split = new MergeView({
    a: { doc: base.text, extensions: baseExtensions(tab) },
    b: { doc: v.state.doc, selection: v.state.selection, extensions: editingExtensions(tab, readOnly, true) },
    parent: wrap,
    gutter: true,
    highlightChanges: true,
    collapseUnchanged: fold,
    revertControls: base.exists && !readOnly ? "a-to-b" : undefined,
    renderRevertControl: () => {
      const b = h("button", { class: "merge-revert", title: "Revert this change" });
      b.append(icon("chevron"));
      return b;
    },
    diffConfig: DIFF_CONF,
  });
  const revert = base.exists && !readOnly;
  wrap.prepend(
    h(
      "div",
      { class: "split-labels" },
      (tab.splitLabel = h("span", null, tab.diff.against === "index" ? "Staged" : "HEAD")),
      revert && h("i", { class: "split-gap" }),
      h("span", null, tab.kind === "deleted" ? "Deleted" : "Working Copy"),
    ),
  );
  if (tab === active) requestAnimationFrame(() => tab.split?.b.focus());
}

function exitSplit(tab) {
  const mv = tab.split;
  if (!mv) return;
  const b = mv.b;
  const doc = b.state.doc, sel = b.state.selection;
  const wrap = mv.dom.parentElement;
  tab.split = null;
  mv.destroy();
  wrap?.remove();
  tab.host.classList.remove("splitting");
  syncDoc(tab.view, doc, EditorSelection.create(sel.ranges.map((r) => EditorSelection.range(Math.min(r.anchor, doc.length), Math.min(r.head, doc.length))), sel.mainIndex));
  if (tab === active) requestAnimationFrame(() => tab.view?.focus());
}

function computeStats(tab) {
  const base = tab.bases[tab.diff.against];
  if (!base || !tab.view || !diffAvailable(tab)) {
    if (tab.stats) {
      tab.stats = null;
      if (tab === active) renderTools();
    }
    return;
  }
  const doc = cur(tab).state.doc;
  const chunks = (tab.diff.on && getChunks(cur(tab).state)?.chunks) || Chunk.build(base.text, doc, DIFF_CONF);
  let add = 0, del = 0;
  for (const c of chunks) {
    if (c.toA > c.fromA) del += base.text.lineAt(c.endA).number - base.text.lineAt(c.fromA).number + 1;
    if (c.toB > c.fromB) add += doc.lineAt(c.endB).number - doc.lineAt(c.fromB).number + 1;
  }
  const prev = tab.stats;
  tab.stats = { add, del, chunks: chunks.length };
  if (tab === active && (!prev || prev.add !== add || prev.del !== del)) renderTools();
}

export async function toggleDiff(tab = active, on = !tab?.diff.on) {
  if (!tab || !diffAvailable(tab) || tab.kind === "deleted") return;
  tab.diff.on = on;
  await applyDiff(tab);
  if (on) jumpToFirstChange(tab);
  focusEditor();
}

async function setDiffOption(tab, key, value) {
  if (tab.diff[key] === value) return;
  tab.diff[key] = value;
  await applyDiff(tab);
  focusEditor();
}

function stepChunk(dir) {
  const v = activeView();
  if (!v) return;
  (dir > 0 ? goToNextChunk : goToPreviousChunk)(v);
  v.dispatch({ effects: EditorView.scrollIntoView(v.state.selection.main.head, { y: "center" }) });
  v.focus();
}

// ------------------------------------------------------------------ theme hooks

export function setDark(dark) {
  for (const t of tabs) {
    t.ctl?.setDark?.(dark);
    for (const v of [t.view, t.split?.a, t.split?.b]) {
      if (v) v.dispatch({ effects: t.comp.dark.reconfigure(EditorView.darkTheme.of(!!dark)) });
    }
  }
}

export function remeasure() {
  for (const t of tabs) {
    t.ctl?.remeasure?.();
    for (const v of [t.view, t.split?.a, t.split?.b]) v?.requestMeasure();
  }
}

// ------------------------------------------------------------------ rendering

function render() {
  renderTabs();
  renderCrumbs();
  renderTools();
  renderBanner();
  els.empty.style.display = active ? "none" : "";
  els.crumbbar.style.display = active && active.kind !== "custom" ? "" : "none";
  if (!active) renderEmpty();
  emit("title");
}

function renderTabs() {
  if (!els.tabs) return;
  clear(els.tabs);
  const counts = new Map();
  for (const t of tabs) counts.set(t.name, (counts.get(t.name) || 0) + 1);
  for (const t of tabs) {
    const custom = t.kind === "custom";
    const tint = custom ? null : tintOf(S.byPath.get(t.path));
    const dir = custom ? "" : dirname(t.path);
    const el = h(
      "div",
      {
        class: "tab" + (t === active ? " active" : "") + (t.dirty ? " dirty" : "") + (tint ? " tint-" + tint : "") + (custom ? " custom" : ""),
        role: "tab",
        title: custom ? t.title || t.name : t.path,
        "aria-selected": t === active ? "true" : "false",
        onMouseDown: (e) => {
          if (e.button === 1) e.preventDefault();
        },
        onAuxClick: (e) => {
          if (e.button === 1) {
            e.preventDefault();
            closeTab(t);
          }
        },
        onClick: () => activate(t),
      },
      icon(custom ? t.icon || "file" : fileIcon(t.name), "tab-ic"),
      h("span", { class: "tab-name" }, custom && t.label ? t.label() : t.name),
      !custom && counts.get(t.name) > 1 && dir && h("span", { class: "tab-dir" }, basename(dir)),
      h(
        "button",
        {
          class: "tab-close",
          title: "Close (⌃W)",
          tabIndex: -1,
          onClick: (e) => {
            e.stopPropagation();
            closeTab(t);
          },
        },
        h("span", { class: "dot" }),
        icon("close"),
      ),
    );
    els.tabs.append(el);
    if (t === active) requestAnimationFrame(() => el.scrollIntoView({ block: "nearest", inline: "nearest" }));
  }
}

function renderCrumbs() {
  if (!els.crumbs) return;
  clear(els.crumbs);
  if (!active) return;
  const parts = active.path.split("/");
  parts.forEach((p, i) => {
    const last = i === parts.length - 1;
    const sub = parts.slice(0, i + 1).join("/");
    if (i) els.crumbs.append(h("span", { class: "crumb-sep" }, icon("chevron")));
    els.crumbs.append(
      h(
        "button",
        { class: "crumb" + (last ? " last" : ""), title: last ? "Reveal in Files" : sub, tabIndex: -1, onClick: () => emit("reveal", sub) },
        last && icon(fileIcon(p), "crumb-ic"),
        p,
      ),
    );
  });
}

function seg(options, value, onPick) {
  return h(
    "div",
    { class: "seg seg-sm" },
    options.map(([v, label, ic, title]) =>
      h("button", { class: "seg-btn" + (v === value ? " active" : "") + (label ? "" : " icon-only"), title: title || label, onClick: () => onPick(v) }, ic && icon(ic), label || null),
    ),
  );
}

function renderTools() {
  if (!els.tools) return;
  clear(els.tools);
  const t = active;
  if (!t) return;
  if (diffAvailable(t)) {
    const d = t.diff;
    const st = t.stats;
    const stats =
      st && h("span", { class: "stats" }, t.kind !== "deleted" && h("span", { class: "add" }, `+${st.add}`), h("span", { class: "del" }, `−${st.del}`));
    if (t.kind === "deleted") els.tools.append(h("span", { class: "tool-label del" }, "Deleted"));
    else
      els.tools.append(
        h("button", { class: "tool-btn diff-toggle" + (d.on ? " on" : ""), title: d.on ? "Hide changes" : "Show changes", onClick: () => toggleDiff(t) }, icon("diff"), h("span", { class: "tool-text" }, "Diff"), stats),
      );
    if (d.on || t.kind === "deleted") {
      if (t.kind === "deleted" && stats) els.tools.append(stats);
      els.tools.append(
        h("button", { class: "tool-btn icon-only nav", title: "Previous change", onClick: () => stepChunk(-1) }, icon("arrowUp")),
        h("button", { class: "tool-btn icon-only nav", title: "Next change", onClick: () => stepChunk(1) }, icon("arrowDown")),
        h("button", { class: "tool-btn icon-only nav" + (d.fold ? " on" : ""), title: d.fold ? "Show all lines" : "Collapse unchanged lines", onClick: () => setDiffOption(t, "fold", !d.fold) }, icon("fold")),
      );
      const ag = againstOptions(t);
      if (ag.length > 1)
        els.tools.append(seg([["head", "HEAD", null, "Compare with the last commit"], ["index", "Staged", null, "Compare with the staged version"]], d.against, (v) => setDiffOption(t, "against", v)));
      els.tools.append(seg([["inline", "", "inline", "Inline"], ["split", "", "split", "Side by side"]], d.mode, (v) => setDiffOption(t, "mode", v)));
    }
    els.tools.append(h("span", { class: "tool-sep" }));
  }
  if (t.kind === "text" || t.kind === "deleted") {
    els.tools.append(h("span", { class: "tool-info" }, t.lang.name));
    els.cursor = h("span", { class: "tool-info pos" });
    els.tools.append(els.cursor);
    renderCursor();
  } else els.cursor = null;
}

function renderCursor() {
  const v = activeView();
  if (!els.cursor || !v) return;
  const sel = v.state.selection.main;
  const line = v.state.doc.lineAt(sel.head);
  const col = sel.head - line.from + 1;
  const n = sel.empty ? "" : ` (${Math.abs(sel.to - sel.from)} selected)`;
  els.cursor.textContent = `Ln ${line.number}, Col ${col}${n}`;
}

function renderBanner() {
  if (!els.banner) return;
  clear(els.banner);
  const t = active;
  if (!t || !t.pendingDisk) return;
  els.banner.append(
    h(
      "div",
      { class: "banner" },
      icon("warning"),
      h("span", { class: "banner-text" }, h("b", null, "This file changed on disk."), " You have unsaved edits here."),
      h("span", { class: "grow" }),
      h(
        "button",
        {
          class: "btn btn-sm",
          onClick: () => {
            t.disk = t.pendingDisk;
            t.pendingDisk = null;
            t.dirty = !cur(t).state.doc.eq(t.disk);
            render();
          },
        },
        "Keep Mine",
      ),
      h(
        "button",
        {
          class: "btn btn-sm btn-primary",
          onClick: () => {
            t.disk = t.pendingDisk;
            t.pendingDisk = null;
            syncDoc(cur(t), t.disk);
            t.dirty = false;
            render();
            toast(`Reloaded ${t.name} from disk`, { kind: "success" });
          },
        },
        "Reload",
      ),
    ),
  );
}

// Re-render tab tints and toolbar (status changed elsewhere).
export function refreshChrome() {
  render();
}
