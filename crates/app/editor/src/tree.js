// Lazy file tree with a filter over the names that are already loaded.

import { api } from "./api.js";
import { S, tintOf, letterOf, emit } from "./store.js";
import { h, clear, join, ancestors, debounce, prefs } from "./util.js";
import { icon, fileIcon } from "./icons.js";
import { toastError } from "./ui.js";
import { openFile } from "./editor.js";

const cache = new Map(); // dir -> [{name, dir}]
const inflight = new Map(); // dir -> Promise
const expanded = new Set();
let filter = "";
let activePath = null;
let cursor = null; // keyboard focus row path
let rows = []; // visible rows in order
const els = {};

export function mountTree(root) {
  els.filter = h("input", {
    class: "filter-input",
    placeholder: "Filter files",
    spellcheck: false,
    autocomplete: "off",
    onInput: () => {
      filter = els.filter.value.trim().toLowerCase();
      els.filterBox.classList.toggle("has-value", !!filter);
      render();
    },
    onKeyDown: (e) => {
      if (e.key === "Escape") {
        if (els.filter.value) {
          e.stopPropagation();
          clearFilter();
        } else els.list.focus();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        els.list.focus();
        moveCursor(rows.length ? 0 : -1, true);
      } else if (e.key === "Enter" && rows.length) {
        const first = rows.find((r) => !r.dir) || rows[0];
        activateRow(first);
      }
    },
  });
  els.filterBox = h(
    "div",
    { class: "filter" },
    icon("search", "filter-ic"),
    els.filter,
    h("button", { class: "filter-clear", title: "Clear", tabIndex: -1, onClick: () => clearFilter() }, icon("close")),
  );
  els.title = h("span", { class: "section-title" });
  els.head = h(
    "div",
    { class: "section-head" },
    els.title,
    h("span", { class: "grow" }),
    h("button", { class: "icon-btn", title: "Refresh", onClick: () => reloadTree() }, icon("refresh")),
    h("button", { class: "icon-btn", title: "Collapse All", onClick: () => collapseAll() }, icon("collapse")),
  );
  els.list = h("div", { class: "tree", tabIndex: 0, role: "tree", onKeyDown: onKey, onClick: onClick, onMouseDown: onMouseDown });
  root.append(els.filterBox, els.head, els.list);
}

export function focusFilter() {
  els.filter?.focus();
  els.filter?.select();
}

function clearFilter() {
  els.filter.value = "";
  filter = "";
  els.filterBox.classList.remove("has-value");
  render();
}

export function loadDir(dir) {
  if (inflight.has(dir)) return inflight.get(dir);
  const p = api("ls", { dir })
    .then((r) => {
      cache.set(dir, (r.entries || []).map((e) => ({ name: e.name, dir: !!e.dir })));
    })
    .catch((e) => {
      cache.delete(dir);
      expanded.delete(dir);
      if (dir === "") toastError(e, "Couldn't list the project");
      throw e;
    })
    .finally(() => inflight.delete(dir));
  inflight.set(dir, p);
  return p;
}

// Expanded folders are remembered per project root.
export function restoreExpanded() {
  expanded.clear();
  for (const p of prefs.get("expanded:" + S.root, [])) if (typeof p === "string" && p) expanded.add(p);
}
const saveExpanded = debounce(() => prefs.set("expanded:" + S.root, [...expanded].slice(0, 200)), 400);

async function toggle(path, open = !expanded.has(path)) {
  if (open) {
    expanded.add(path);
    if (!cache.has(path)) {
      render();
      try {
        await loadDir(path);
      } catch (e) {
        toastError(e);
      }
    }
  } else expanded.delete(path);
  saveExpanded();
  render();
}

function collapseAll() {
  expanded.clear();
  saveExpanded();
  render();
}

// Expand every ancestor of `path` (loading as needed) and scroll it into view.
export async function reveal(path, { select = true } = {}) {
  if (!path) return;
  for (const d of ["", ...ancestors(path)]) {
    if (d) expanded.add(d);
    if (!cache.has(d)) {
      try {
        await loadDir(d);
      } catch {
        return;
      }
    }
  }
  if (select) cursor = path;
  saveExpanded();
  render();
  const el = els.list?.querySelector(`[data-path="${CSS.escape(path)}"]`);
  el?.scrollIntoView({ block: "nearest" });
}

export function setActive(path) {
  activePath = path;
  if (path) cursor = path;
  render();
}

// Re-list every loaded directory that is visible, drop the hidden ones.
export async function reloadTree() {
  const dirs = [...cache.keys()];
  const visible = (d) => d === "" || ancestors(d + "/x").every((a) => expanded.has(a));
  for (const d of dirs) if (!visible(d)) cache.delete(d);
  await Promise.all([...cache.keys()].map((d) => loadDir(d).catch(() => {})));
  emit("tree");
  render();
}

// After a status change: if a changed path's parent is loaded but doesn't
// list it (new or deleted file), re-list the tree.
export function syncWithStatus() {
  let stale = false;
  for (const f of S.status?.files || []) {
    const p = f.path.replace(/\/$/, "");
    const parts = p.split("/");
    for (let i = 0; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      const entries = cache.get(dir);
      if (!entries) break;
      const has = entries.some((e) => e.name === parts[i]);
      const gone = f.unstaged === "D" && i === parts.length - 1;
      if (has === gone) {
        stale = true;
        break;
      }
    }
    if (stale) break;
  }
  if (stale) reloadTree();
  else render();
}

export function loadedFiles() {
  const out = [];
  for (const [dir, entries] of cache) for (const e of entries) if (!e.dir) out.push(join(dir, e.name));
  return out;
}

// ------------------------------------------------------------------ render

// Paths to show while filtering: every loaded match plus its ancestors
// (which are shown expanded).
function matchSet() {
  const show = new Set(), open = new Set();
  for (const [dir, entries] of cache) {
    for (const e of entries) {
      if (!e.name.toLowerCase().includes(filter)) continue;
      const p = join(dir, e.name);
      show.add(p);
      for (const a of ancestors(p)) {
        show.add(a);
        open.add(a);
      }
    }
  }
  return { show, open };
}

export function render() {
  if (!els.list) return;
  els.title.textContent = S.name || "Project";
  const scroll = els.list.scrollTop;
  clear(els.list);
  rows = [];
  const set = filter ? matchSet() : null;
  const walk = (dir, depth) => {
    const entries = cache.get(dir);
    if (!entries) {
      if (depth > 0 && !set) {
        // Expanded but not listed yet (e.g. restored from a previous session).
        if (!inflight.has(dir)) loadDir(dir).then(render, render);
        rows.push({ loading: true, depth, path: dir + "/…" });
      }
      return;
    }
    for (const e of entries) {
      const p = join(dir, e.name);
      if (set && !set.show.has(p)) continue;
      const open = e.dir && (set ? set.open.has(p) : expanded.has(p));
      rows.push({ path: p, name: e.name, dir: e.dir, depth, open });
      if (open) walk(p, depth + 1);
    }
  };
  walk("", 0);
  const frag = document.createDocumentFragment();
  for (const r of rows) frag.append(rowEl(r));
  if (!rows.length) {
    frag.append(
      h(
        "div",
        { class: "tree-empty" },
        filter ? `No loaded files match “${els.filter.value.trim()}”` : cache.has("") ? "This folder is empty" : inflight.has("") ? "" : "Couldn't list this folder",
      ),
    );
  }
  els.list.append(frag);
  els.list.scrollTop = scroll;
}

function highlightName(name) {
  if (!filter) return name;
  const i = name.toLowerCase().indexOf(filter);
  if (i < 0) return name;
  return [name.slice(0, i), h("mark", null, name.slice(i, i + filter.length)), name.slice(i + filter.length)];
}

function rowEl(r) {
  if (r.loading) return h("div", { class: "row loading", style: `--depth:${r.depth}` }, h("span", { class: "twisty" }), h("span", { class: "spinner" }));
  const entry = r.dir ? null : S.byPath.get(r.path) || S.byPath.get(r.path + "/");
  const tint = r.dir ? S.dirTints.get(r.path) || (S.byPath.get(r.path + "/") ? "untracked" : null) : tintOf(entry);
  const letter = entry ? letterOf(entry) : "";
  return h(
    "div",
    {
      class:
        "row" +
        (r.dir ? " dir" : " file") +
        (r.open ? " open" : "") +
        (r.path === activePath ? " active" : "") +
        (r.path === cursor ? " cursor" : "") +
        (tint ? " tint-" + tint : ""),
      style: `--depth:${r.depth}`,
      "data-path": r.path,
      role: "treeitem",
      title: r.path,
    },
    h("span", { class: "twisty" }, r.dir && icon("chevron")),
    icon(r.dir ? (r.open ? "folderOpen" : "folder") : fileIcon(r.name), "row-ic"),
    h("span", { class: "row-name" }, highlightName(r.name)),
    r.dir ? tint && h("span", { class: "row-dot" }) : letter && h("span", { class: "row-letter" }, letter),
  );
}

// ------------------------------------------------------------------ interaction

function rowAt(target) {
  const el = target.closest?.(".row");
  if (!el || !el.dataset.path) return null;
  return rows.find((r) => r.path === el.dataset.path) || null;
}

function onMouseDown(e) {
  if (e.button === 1) e.preventDefault();
}

function onClick(e) {
  const r = rowAt(e.target);
  if (!r) return;
  cursor = r.path;
  activateRow(r, { focus: true });
}

function activateRow(r, { focus = true } = {}) {
  if (r.dir) toggle(r.path);
  else {
    cursor = r.path;
    openFile(r.path, { focus });
  }
}

function moveCursor(i, scroll) {
  if (!rows.length) return;
  const idx = Math.max(0, Math.min(rows.length - 1, i));
  cursor = rows[idx].path;
  render();
  if (scroll) els.list.querySelector(".row.cursor")?.scrollIntoView({ block: "nearest" });
}

function onKey(e) {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  const i = rows.findIndex((r) => r.path === cursor);
  const r = rows[i];
  switch (e.key) {
    case "ArrowDown":
      e.preventDefault();
      moveCursor(i + 1, true);
      break;
    case "ArrowUp":
      e.preventDefault();
      moveCursor(i < 0 ? 0 : i - 1, true);
      break;
    case "ArrowRight":
      e.preventDefault();
      if (r?.dir && !r.open) toggle(r.path, true);
      else if (r?.dir) moveCursor(i + 1, true);
      break;
    case "ArrowLeft": {
      e.preventDefault();
      if (r?.dir && r.open && !filter) toggle(r.path, false);
      else if (r) {
        const parent = r.path.includes("/") ? r.path.slice(0, r.path.lastIndexOf("/")) : null;
        const pi = rows.findIndex((x) => x.path === parent);
        if (pi >= 0) moveCursor(pi, true);
      }
      break;
    }
    case "Enter":
    case " ":
      if (r) {
        e.preventDefault();
        activateRow(r, { focus: e.key === "Enter" });
      }
      break;
    case "Home":
      e.preventDefault();
      moveCursor(0, true);
      break;
    case "End":
      e.preventDefault();
      moveCursor(rows.length - 1, true);
      break;
    default:
      if (e.key.length === 1 && /\S/.test(e.key)) {
        els.filter.focus();
      }
  }
}
