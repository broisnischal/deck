// The History view: a compact commit graph (virtualised list), a filter over
// the loaded commits and an All branches / Current branch switch.

import { api } from "./api.js";
import { S, emit, on } from "./store.js";
import { h, clear, prefs, relTime, fullDate, initials, escapeHtml, debounce, copyText } from "./util.js";
import { icon } from "./icons.js";
import { toast, toastError, segmented, showContextMenu } from "./ui.js";
import { createGraph, extendGraph, rowSvg, dotSvg, graphWidth } from "./graph.js";
import { parseRefs, pillsHtml, avatarClass } from "./refs.js";
import { openCommit } from "./commitview.js";

const PAGE = 200;
const MAX_LANES = 9;
const els = {};
const H = {
  commits: [],
  byHash: new Map(),
  graph: createGraph(),
  all: prefs.get("history:all", true),
  q: "",
  loading: false,
  more: true,
  error: null,
  stale: true,
  loaded: false,
  gen: 0,
  selected: null,
  view: [], // indices into commits that are shown (all, or the filter's matches)
};
let rowH = 24;
let visible = false;

export function mountHistory(root) {
  els.root = root;
  els.filter = h("input", {
    class: "filter-input",
    placeholder: "Filter commits",
    spellcheck: false,
    autocomplete: "off",
    "aria-label": "Filter commits by subject, author or hash",
    onInput: debounce(() => {
      H.q = els.filter.value.trim().toLowerCase();
      els.filterBox.classList.toggle("has-value", !!H.q);
      applyFilter();
      els.scroll.scrollTop = 0;
      renderRows();
    }, 60),
    onKeyDown: (e) => {
      if (e.key === "Escape" && els.filter.value) {
        e.stopPropagation();
        clearFilter();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        els.scroll.focus();
        if (H.view.length) select(H.commits[H.view[0]].hash, { open: true });
      } else if (e.key === "Enter" && H.view.length) {
        select(H.commits[H.view[0]].hash, { open: true });
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
  els.scope = h("div", { class: "hist-scope" });
  els.bar = h("div", { class: "hist-bar" }, els.filterBox, els.scope);
  els.spacer = h("div", { class: "hist-spacer" });
  els.probe = h("div", { class: "hrow probe", "aria-hidden": "true" }, "x");
  els.scroll = h(
    "div",
    {
      class: "hist-scroll",
      tabIndex: 0,
      role: "listbox",
      "aria-label": "Commits",
      onScroll: () => schedule(),
      onKeyDown: onKey,
      onMouseDown: onMouseDown,
      onClick: onClick,
      onContextMenu: onContext,
      onFocus: () => markFocus(),
      onBlur: () => markFocus(),
    },
    els.spacer,
  );
  els.state = h("div", { class: "hist-state" });
  root.append(els.bar, els.scroll, els.state, els.probe);
  renderScope();
  new ResizeObserver(() => visible && renderRows()).observe(els.scroll);
  on("refs", () => {
    H.stale = true;
    if (visible) reload();
  });
  on("commit-shown", (hash) => {
    if (hash && hash !== H.selected && H.byHash.has(hash)) {
      H.selected = hash;
      scrollToHash(hash);
      renderRows();
    }
  });
}

function renderScope() {
  clear(els.scope);
  els.scope.append(
    segmented(
      [
        [true, "All", null, "Commits on every branch"],
        [false, "Current", null, "Commits reachable from HEAD"],
      ],
      H.all,
      (v) => {
        H.all = v;
        prefs.set("history:all", v);
        renderScope();
        reset();
        load();
      },
    ),
  );
}

function clearFilter() {
  els.filter.value = "";
  H.q = "";
  els.filterBox.classList.remove("has-value");
  applyFilter();
  renderRows();
  if (H.selected) scrollToHash(H.selected);
}

export function setHistoryVisible(on) {
  visible = on;
  if (!on) return;
  if (!H.loaded) load();
  else if (H.stale) reload();
  else renderRows();
}

export function focusHistory() {
  els.scroll?.focus();
}
export function focusHistoryFilter() {
  els.filter?.focus();
  els.filter?.select();
}

export function markHistoryStale() {
  H.stale = true;
  if (visible) reload();
}

// ------------------------------------------------------------------ data

function reset() {
  H.gen++;
  H.commits = [];
  H.byHash = new Map();
  H.graph = createGraph();
  H.more = true;
  H.loaded = false;
  H.error = null;
  H.loading = false;
  H.view = [];
  if (els.scroll) els.scroll.scrollTop = 0;
}

function ingest(list) {
  const fresh = [];
  for (const c of list) {
    if (H.byHash.has(c.hash)) continue;
    H.byHash.set(c.hash, c);
    H.commits.push(c);
    fresh.push(c);
  }
  extendGraph(H.graph, fresh);
}

async function loadRemotes() {
  try {
    const r = await api("branches");
    const set = new Set();
    for (const b of r.branches || []) if (b.remote) set.add(b.name.split("/")[0]);
    if (set.size) S.remotes = set;
  } catch {}
}

let inflight = null;
function track(p) {
  const t = p.finally(() => {
    if (inflight === t) inflight = null;
  });
  inflight = t;
  return t;
}

// Load the next page. Calls while a page is on its way share it.
function load() {
  if (inflight) return inflight;
  if (!S.git || !H.more) return Promise.resolve();
  return track(loadPage());
}

async function loadPage() {
  const gen = H.gen;
  H.loading = true;
  H.error = null;
  renderRows();
  try {
    const [r] = await Promise.all([api("log", { limit: PAGE, skip: H.commits.length, all: H.all }), H.loaded ? null : loadRemotes()]);
    if (gen !== H.gen) return;
    const list = r.commits || [];
    ingest(list);
    H.more = list.length >= PAGE;
    H.loaded = true;
    H.stale = false;
  } catch (e) {
    if (gen !== H.gen) return;
    H.error = e.message || String(e);
  } finally {
    if (gen === H.gen) {
      H.loading = false;
      applyFilter();
      renderRows();
    }
  }
}

// Re-read what is loaded (at least a page), keeping the scroll position.
function reload() {
  if (!S.git) return Promise.resolve();
  return track((inflight || Promise.resolve()).then(reloadNow));
}

async function reloadNow() {
  const keep = Math.max(PAGE, Math.ceil(H.commits.length / PAGE) * PAGE);
  const gen = ++H.gen;
  H.loading = true;
  try {
    const r = await api("log", { limit: keep, skip: 0, all: H.all });
    if (gen !== H.gen) return;
    const list = r.commits || [];
    H.commits = [];
    H.byHash = new Map();
    H.graph = createGraph();
    ingest(list);
    H.more = list.length >= keep;
    H.loaded = true;
    H.stale = false;
    H.error = null;
  } catch (e) {
    if (gen !== H.gen) return;
    H.error = e.message || String(e);
  } finally {
    if (gen === H.gen) {
      H.loading = false;
      applyFilter();
      renderRows();
    }
  }
}

export function reloadHistory() {
  H.stale = true;
  if (visible || H.loaded) reload();
}

function applyFilter() {
  const q = H.q;
  H.view = [];
  for (let i = 0; i < H.commits.length; i++) {
    const c = H.commits[i];
    if (!q || c.subject.toLowerCase().includes(q) || (c.author || "").toLowerCase().includes(q) || (c.email || "").toLowerCase().includes(q) || c.hash.startsWith(q))
      H.view.push(i);
  }
}

function findCommit(hash) {
  if (!hash) return null;
  const hl = String(hash).toLowerCase();
  return H.byHash.get(hl) || H.commits.find((c) => c.hash.startsWith(hl)) || null;
}

// Select a commit by hash, loading more history until it shows up (at most a
// few pages). Resolves to the commit when it is in the list.
export async function revealCommit(hash) {
  if (inflight) await inflight;
  if (!H.loaded) await load();
  let c = findCommit(hash);
  for (let i = 0; !c && H.more && i < 10; i++) {
    await load();
    c = findCommit(hash);
  }
  if (c) {
    if (H.q && !H.view.includes(H.commits.indexOf(c))) clearFilter();
    H.selected = c.hash;
    scrollToHash(c.hash, true);
    renderRows();
  }
  return c;
}

// ------------------------------------------------------------------ render

let raf = 0;
function schedule() {
  if (raf) return;
  raf = requestAnimationFrame(() => {
    raf = 0;
    renderRows();
  });
}

function measure() {
  const hgt = els.probe.offsetHeight;
  if (hgt > 0) rowH = hgt;
  return rowH;
}

function stateView() {
  clear(els.state);
  const show = (ic, title, sub, action) => {
    els.state.append(
      h(
        "div",
        { class: "panel-empty" },
        h("div", { class: "panel-empty-ic" }, icon(ic)),
        h("div", { class: "panel-empty-title" }, title),
        sub && h("div", { class: "panel-empty-sub" }, sub),
        action,
      ),
    );
    els.state.style.display = "";
  };
  els.state.style.display = "none";
  if (!S.git) return show("git", "Not a Git repository", "This folder isn't tracked by git, so there is no history to show."), true;
  if (H.error && !H.commits.length)
    return show("warning", "Couldn't load the history", H.error, h("button", { class: "btn btn-sm", onClick: () => (reset(), load()) }, "Try Again")), true;
  if (H.loaded && !H.commits.length)
    return (
      show(
        "commit",
        "No commits yet",
        H.all ? "Commit from the Changes tab to start the history." : "The current branch has no commits yet.",
        h("button", { class: "btn btn-sm", onClick: () => emit("view", "changes") }, "Go to Changes"),
      ),
      true
    );
  return false;
}

function renderRows() {
  if (!els.scroll) return;
  const blocked = stateView();
  els.scroll.style.display = blocked ? "none" : "";
  els.bar.style.display = S.git && (H.commits.length || H.q || (!blocked && !H.loaded)) ? "" : "none";
  if (blocked) return;
  const H_ = measure();
  const n = H.view.length;
  const filtering = !!H.q;
  const footer = 1;
  els.spacer.style.height = (n + footer) * H_ + "px";
  const top = els.scroll.scrollTop;
  const height = els.scroll.clientHeight || 600;
  const from = Math.max(0, Math.floor(top / H_) - 6);
  const to = Math.min(n + footer, Math.ceil((top + height) / H_) + 6);
  const lanes = Math.min(els.scroll.clientWidth < 320 ? 6 : MAX_LANES, H.graph.width);
  const gw = filtering ? graphWidth(1) : graphWidth(lanes);
  const focused = document.activeElement === els.scroll;
  const now = Date.now() / 1000;
  const maxPills = els.scroll.clientWidth < 320 ? 1 : 2;
  let html = "";
  for (let v = from; v < to; v++) {
    const y = v * H_;
    if (v === n) {
      html += footerHtml(y, n, filtering);
      continue;
    }
    const i = H.view[v];
    const c = H.commits[i];
    const g = H.graph.rows[i];
    const pills = parseRefs(c.refs);
    const isHead = pills.some((p) => p.head);
    const sel = c.hash === H.selected;
    const svg = filtering ? dotSvg(g?.color || 0, H_, c.parents.length > 1) : g ? rowSvg(g, H_, lanes) : "";
    html +=
      `<div class="hrow${sel ? " sel" : ""}${sel && focused ? " cursor" : ""}${isHead ? " head" : ""}${c.parents.length > 1 ? " merge" : ""}" role="option" aria-selected="${sel}" data-v="${v}" style="transform:translateY(${y}px)" title="${escapeHtml(`${c.short || c.hash.slice(0, 7)} · ${c.author} · ${fullDate(c.date)}\n${c.subject}`)}">` +
      `<span class="hgraph" style="width:${gw}px">${svg}</span>` +
      `<span class="hmain">${pillsHtml(c.refs, maxPills)}<span class="hsubj">${escapeHtml(c.subject)}</span></span>` +
      `<span class="${avatarClass(c.email, c.author)}" title="${escapeHtml(`${c.author} <${c.email}>`)}">${escapeHtml(initials(c.author))}</span>` +
      `<span class="htime">${relTime(c.date, now)}</span>` +
      `</div>`;
  }
  els.spacer.innerHTML = html;
  // Scrolling near the end loads the next page (not while filtering: that
  // could walk the whole history; the footer offers "Load more" instead).
  if (!filtering && H.more && !H.loading && !H.error && to >= n - 4 && H.loaded) load();
}

function footerHtml(y, n, filtering) {
  let inner;
  if (H.loading) inner = `<span class="spinner"></span><span>Loading commits…</span>`;
  else if (H.error) inner = `<span class="hfoot-err">${escapeHtml(H.error)}</span><button class="link-btn" data-act="more">Retry</button>`;
  else if (filtering) {
    inner = `<span>${n ? `${n} of ${H.commits.length} loaded commits` : `No loaded commits match “${escapeHtml(els.filter.value.trim())}”`}</span>`;
    if (H.more) inner += `<button class="link-btn" data-act="more">Load more</button>`;
  } else if (H.more) inner = `<button class="link-btn" data-act="more">Load more</button>`;
  else inner = `<span>${H.commits.length} commit${H.commits.length === 1 ? "" : "s"}</span>`;
  return `<div class="hrow hfoot" style="transform:translateY(${y}px)">${inner}</div>`;
}

function scrollToHash(hash, center = false) {
  const i = H.commits.findIndex((c) => c.hash === hash);
  const v = H.view.indexOf(i);
  if (v < 0 || !els.scroll) return;
  const H_ = measure();
  const y = v * H_;
  const s = els.scroll;
  if (center) s.scrollTop = Math.max(0, y - s.clientHeight / 2 + H_ / 2);
  else if (y < s.scrollTop) s.scrollTop = y;
  else if (y + H_ > s.scrollTop + s.clientHeight) s.scrollTop = y + H_ - s.clientHeight;
}

// ------------------------------------------------------------------ interaction

const openSoon = debounce((hash) => openCommit(hash, { preview: H.byHash.get(hash) }), 110);

function select(hash, { open = true, now = false } = {}) {
  if (!hash) return;
  H.selected = hash;
  scrollToHash(hash);
  renderRows();
  if (open) {
    if (now) {
      openSoon.cancel();
      openCommit(hash, { preview: H.byHash.get(hash) });
    } else openSoon(hash);
  }
}

function rowAt(e) {
  const el = e.target.closest?.(".hrow");
  if (!el || el.dataset.v == null) return null;
  const c = H.commits[H.view[+el.dataset.v]];
  return c || null;
}

// Like a native list, a row is chosen on mouse down. (Re-rendering on focus
// would replace the row under the pointer before the click lands.)
function onMouseDown(e) {
  if (e.button !== 0 || e.target.closest?.("[data-act]")) return;
  const c = rowAt(e);
  if (c) select(c.hash, { open: true, now: true });
}

function onClick(e) {
  if (e.target.closest?.("[data-act=more]")) {
    if (H.error) H.error = null;
    load();
  }
}

function markFocus() {
  const focused = document.activeElement === els.scroll;
  els.spacer.querySelector(".hrow.sel")?.classList.toggle("cursor", focused);
}

function onContext(e) {
  const c = rowAt(e);
  if (!c) return;
  select(c.hash, { open: false });
  showContextMenu(e, [
    { label: "Open Commit", icon: "commit", onSelect: () => openCommit(c.hash, { preview: c }) },
    "-",
    { label: "Copy Hash", icon: "copy", hint: c.short || c.hash.slice(0, 7), onSelect: () => copyText(c.hash).then(() => toast(`Copied ${c.hash.slice(0, 7)}`)) },
    { label: "Copy Subject", icon: "copy", onSelect: () => copyText(c.subject) },
  ]);
}

function onKey(e) {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  const v = H.view.findIndex((i) => H.commits[i].hash === H.selected);
  const go = (nv) => {
    e.preventDefault();
    if (!H.view.length) return;
    const k = Math.max(0, Math.min(H.view.length - 1, nv));
    select(H.commits[H.view[k]].hash, { open: true });
  };
  const page = Math.max(1, Math.floor((els.scroll.clientHeight || 400) / measure()) - 1);
  switch (e.key) {
    case "ArrowDown":
      return go(v < 0 ? 0 : v + 1);
    case "ArrowUp":
      return go(v < 0 ? 0 : v - 1);
    case "PageDown":
      return go(v + page);
    case "PageUp":
      return go(v - page);
    case "Home":
      return go(0);
    case "End":
      return go(H.view.length - 1);
    case "Enter":
      e.preventDefault();
      if (H.selected) openCommit(H.selected, { preview: H.byHash.get(H.selected), focus: true });
      return;
    default:
      if (e.key.length === 1 && /\S/.test(e.key)) els.filter.focus();
  }
}

export function historyCommit(hash) {
  return H.byHash.get(hash) || null;
}
export const historySelected = () => H.selected;
