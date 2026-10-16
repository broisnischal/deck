// Deck editor: boot, layout, keyboard, polling and the window.deck bridge.

import "./styles.css";
import { api, isMock, params } from "./api.js";
import { S, on } from "./store.js";
import { h, prefs, isMac } from "./util.js";
import { applyTheme, applyFont, initialTheme } from "./theme.js";
import { toast, toastError, isModalOpen } from "./ui.js";
import {
  mountEditor,
  openFile,
  save,
  activeTab,
  closeActive,
  cycleTab,
  openFind,
  setDark,
  remeasure,
  onStatus,
  refreshOpenFiles,
  toggleDiff,
  applyDiff,
  activeView,
} from "./editor.js";
import { mountTree, loadDir, reveal, setActive, reloadTree, syncWithStatus, focusFilter, restoreExpanded, render as renderTree } from "./tree.js";
import { mountChanges, renderChanges, focusChanges } from "./changes.js";
import { mountBranchBar, renderBranchBar } from "./branchbar.js";
import { mountHistory, setHistoryVisible, focusHistory, focusHistoryFilter, revealCommit, reloadHistory } from "./history.js";
import { openCommit } from "./commitview.js";
import { refreshStatus } from "./git.js";
import { openQuickOpen, invalidateIndex, isQuickOpenOpen } from "./quickopen.js";
import { guardTextFields } from "./inputguard.js";

applyTheme(isMock ? undefined : initialTheme());
applyFont({ family: "JetBrains Mono", size: 13 }, 13);
guardTextFields(document);

// Motion is for the pointer. After a key press, popovers and dialogs appear
// without animating (styles.css keys off this attribute).
const modality = (m) => () => {
  if (document.documentElement.dataset.input !== m) document.documentElement.dataset.input = m;
};
window.addEventListener("keydown", modality("key"), true);
window.addEventListener("pointerdown", modality("pointer"), true);

const els = {};
let readyResolve;
const ready = new Promise((r) => (readyResolve = r));
const VIEWS = ["files", "changes", "history"];

// ------------------------------------------------------------------ layout

function layout() {
  const width = Math.max(180, Math.min(600, (isMock && +params.get("mocksidebar")) || prefs.get("sidebar", 260)));
  document.documentElement.style.setProperty("--sidebar-w", width + "px");

  const tab = (view, label, extra) =>
    h("button", { class: "side-tab", role: "tab", "data-view": view, onClick: () => setView(view, true) }, h("span", { class: "side-tab-label" }, label), extra);
  els.badge = h("span", { class: "badge" });
  els.tabs = {
    files: tab("files", "Files"),
    changes: tab("changes", "Changes", els.badge),
    history: tab("history", "History"),
  };
  els.panes = {
    files: h("div", { class: "pane files-pane" }),
    changes: h("div", { class: "pane changes-pane" }),
    history: h("div", { class: "pane history-pane" }),
  };
  els.gitbar = h("div", { class: "gitbar" });
  els.sidebar = h(
    "aside",
    { class: "sidebar" },
    h("div", { class: "side-head" }, h("div", { class: "side-tabs", role: "tablist" }, Object.values(els.tabs))),
    els.gitbar,
    h("div", { class: "side-body" }, Object.values(els.panes)),
  );
  els.resizer = h("div", { class: "resizer", title: "Drag to resize, double-click to reset" });
  els.main = h("main", { class: "main" });
  document.body.append(h("div", { class: "app" }, els.sidebar, els.resizer, els.main));

  mountTree(els.panes.files);
  mountChanges(els.panes.changes);
  mountHistory(els.panes.history);
  mountBranchBar(els.gitbar);
  mountEditor(els.main);
  setupResizer();
}

function setupResizer() {
  const r = els.resizer;
  r.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    r.setPointerCapture(e.pointerId);
    document.body.classList.add("resizing");
    const startX = e.clientX;
    const startW = els.sidebar.getBoundingClientRect().width;
    const move = (ev) => {
      const w = Math.round(Math.max(180, Math.min(Math.min(600, innerWidth - 320), startW + ev.clientX - startX)));
      document.documentElement.style.setProperty("--sidebar-w", w + "px");
    };
    const up = () => {
      r.removeEventListener("pointermove", move);
      r.removeEventListener("pointerup", up);
      r.removeEventListener("pointercancel", up);
      document.body.classList.remove("resizing");
      prefs.set("sidebar", Math.round(els.sidebar.getBoundingClientRect().width));
      remeasure();
    };
    r.addEventListener("pointermove", move);
    r.addEventListener("pointerup", up);
    r.addEventListener("pointercancel", up);
  });
  r.addEventListener("dblclick", () => {
    document.documentElement.style.setProperty("--sidebar-w", "260px");
    prefs.set("sidebar", 260);
    remeasure();
  });
}

function setView(view, focus = false) {
  S.view = VIEWS.includes(view) ? view : "files";
  for (const v of VIEWS) {
    const on = S.view === v;
    els.tabs[v].classList.toggle("active", on);
    els.tabs[v].setAttribute("aria-selected", String(on));
    els.panes[v].classList.toggle("on", on);
  }
  els.gitbar.style.display = S.git && S.view !== "files" ? "" : "none";
  setHistoryVisible(S.view === "history");
  const t = activeTab();
  if (S.view === "files" && t && t.kind !== "custom") reveal(t.path, { select: true });
  if (focus) {
    if (S.view === "changes") focusChanges();
    else if (S.view === "history") focusHistory();
    else els.panes.files.querySelector(".tree")?.focus();
  }
}

function updateBadge() {
  const n = S.status?.files?.length || 0;
  els.badge.textContent = n > 999 ? "999+" : n ? String(n) : "";
  els.badge.style.display = n ? "" : "none";
}

function updateTitle() {
  const t = activeTab();
  document.title = t ? `${t.name} · ${S.name || "Editor"}` : S.name || "Editor";
}

// ------------------------------------------------------------------ events

on("status", () => {
  updateBadge();
  renderBranchBar();
  renderChanges();
  syncWithStatus();
  onStatus();
  invalidateIndex();
});
on("busy", () => {
  renderBranchBar();
  renderChanges();
});
on("tabs", () => renderChanges());
on("title", updateTitle);
on("saved", () => setTimeout(refreshStatus, 60));
on("active", (path) => {
  const file = path && !path.startsWith("commit:");
  setActive(file ? path : null);
  if (file && S.view === "files") reveal(path, { select: true });
  renderChanges();
  updateTitle();
});
on("reveal", (path) => {
  setView("files");
  reveal(path, { select: true });
});
on("view", (view) => setView(view, true));
on("disk", () => {
  refreshOpenFiles({ dropBases: true });
  reloadTree();
});
on("refresh", () => window.deck.refresh());

// ------------------------------------------------------------------ keyboard

window.addEventListener(
  "keydown",
  (e) => {
    if (isModalOpen()) return;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const stop = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    if (mod && !e.altKey && !e.shiftKey && k === "p") {
      stop();
      openQuickOpen();
    } else if (mod && !e.altKey && !e.shiftKey && k === "s") {
      stop();
      save();
    } else if (mod && e.shiftKey && !e.altKey && k === "g") {
      stop();
      setView("changes", true);
    } else if (mod && e.shiftKey && !e.altKey && k === "h") {
      stop();
      if (S.git) setView("history", true);
    } else if (mod && e.shiftKey && !e.altKey && k === "e") {
      stop();
      setView("files", true);
    } else if (mod && !e.altKey && !e.shiftKey && k === "f") {
      if (isQuickOpenOpen()) return;
      const inEditor = document.activeElement?.closest?.(".cm-editor");
      if (inEditor) return; // CodeMirror opens its own panel
      stop();
      find();
    } else if (e.ctrlKey && !e.metaKey && e.key === "Tab") {
      stop();
      cycleTab(e.shiftKey ? -1 : 1);
    } else if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && k === "w") {
      stop();
      closeActive();
    } else if (mod && e.shiftKey && !e.altKey && k === "d") {
      stop();
      toggleDiff();
    }
  },
  true,
);

function find() {
  if (S.view === "history" && !document.activeElement?.closest?.(".main")) return focusHistoryFilter();
  if (!openFind() && S.view === "files") focusFilter();
}

// ------------------------------------------------------------------ polling

let lastFocusRefresh = 0;
async function refreshAll({ dropBases = false } = {}) {
  await refreshStatus();
  await refreshOpenFiles({ dropBases });
}

function startPolling() {
  setInterval(() => {
    if (document.visibilityState === "visible") refreshStatus();
  }, 3000);
  window.addEventListener("focus", () => {
    const now = Date.now();
    if (now - lastFocusRefresh < 1500) return;
    lastFocusRefresh = now;
    refreshAll();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") refreshStatus();
  });
}

// ------------------------------------------------------------------ bridge

async function showCommit(hash) {
  if (!S.git || !hash) return;
  setView("history");
  const c = await revealCommit(String(hash));
  openCommit(c ? c.hash : String(hash), { preview: c });
}

window.deck = {
  async open(path, opts = {}) {
    await ready;
    const o = opts || {};
    await openFile(path, { line: o.line, diff: o.diff === undefined ? undefined : !!o.diff });
  },
  async show(view) {
    await ready;
    setView(view);
  },
  // Switch to History and open a commit's view (loading more history to
  // find it; the view itself comes straight from `show`).
  async commit(hash) {
    await ready;
    await showCommit(hash);
  },
  setTheme(theme) {
    applyTheme(theme, !isMock);
    setDark(!!(theme && theme.dark));
  },
  setFont(font) {
    applyFont(font, font && (font.ui_font_size || font.uiSize));
    remeasure();
  },
  async refresh() {
    await ready;
    invalidateIndex();
    await Promise.all([refreshAll({ dropBases: true }), reloadTree()]);
    reloadHistory();
  },
  // Shortcuts Deck itself takes before the page sees them, handed on.
  key(name) {
    if (isModalOpen()) return;
    if (name === "quickopen") openQuickOpen();
    else if (name === "find") {
      if (isQuickOpenOpen()) return;
      find();
    } else if (name === "save") save();
  },
};

// ------------------------------------------------------------------ boot

async function boot() {
  layout();
  setView("files");
  let hello;
  try {
    hello = await api("hello");
  } catch (e) {
    toastError(e, "Couldn't connect to Deck");
    hello = { name: "Editor", git: false };
  }
  S.name = hello.name || (hello.root || "").split("/").pop() || "Editor";
  S.root = hello.root || "";
  S.git = !!hello.git;
  restoreExpanded();
  if (hello.theme) {
    applyTheme(hello.theme, !isMock);
    setDark(!!hello.theme.dark);
  }
  applyFont(hello.font, hello.ui_font_size);
  updateTitle();
  updateBadge();
  renderChanges();
  renderBranchBar();
  renderTree();

  const view = hello.commit && S.git ? "history" : hello.view;
  setView(VIEWS.includes(view) ? view : "files");

  await Promise.all([loadDir("").catch(() => {}), S.git ? refreshStatus() : null]);
  renderTree();
  readyResolve();

  if (hello.open) {
    const diff = isMock ? params.has("mockdiff") : hello.view === "changes" && S.byPath.has(hello.open);
    await openFile(hello.open, { diff: diff || undefined, line: isMock && params.get("mockline") ? +params.get("mockline") : undefined });
  }
  if (hello.commit) await showCommit(hello.commit);
  if (isMock) await mockScenario();
  startPolling();
}

// Mock-only URL switches that put the UI in a given state for screenshots.
async function mockScenario() {
  const t = activeTab();
  if (t && params.get("mocksplit")) {
    t.diff.mode = "split";
    t.diff.on = true;
    await applyDiff(t);
  }
  if (t && params.get("mockfold")) {
    t.diff.fold = true;
    await applyDiff(t);
  }
  if (t && params.get("mockagainst")) {
    t.diff.against = params.get("mockagainst");
    await applyDiff(t);
  }
  for (const p of (params.get("mocktabs") || "").split(",").filter(Boolean)) await openFile(p, { focus: false });
  if (params.get("mockactivate")) await openFile(params.get("mockactivate"));
  if (t && params.get("mockdirty")) {
    const v = activeView();
    v?.dispatch({ changes: { from: 0, insert: "// unsaved edit\n" } });
  }
  if (params.get("mockexpand")) for (const d of params.get("mockexpand").split(",")) await reveal(d + "/x", { select: false });
  if (params.get("mockfilter")) {
    const input = document.querySelector(".files-pane .filter-input");
    input.value = params.get("mockfilter");
    input.dispatchEvent(new Event("input"));
  }
  if (params.get("mockfind")) openFind();
  if (params.has("mockquick")) openQuickOpen(params.get("mockquick"));
  if (params.has("mockcommit")) {
    // A commit by position in the log ("0" is the newest) or by hash prefix.
    const q = params.get("mockcommit");
    let hash = q;
    if (/^\d+$/.test(q)) {
      const r = await api("log", { limit: +q + 1, skip: 0, all: true });
      hash = r.commits[+q]?.hash;
    }
    await showCommit(hash);
    if (params.get("mockcommitfile")) document.querySelectorAll(".cv-file")[+params.get("mockcommitfile")]?.click();
  }
  if (params.get("mockmsg")) {
    const m = document.querySelector(".commit-input");
    m.value = params.get("mockmsg").replace(/\\n/g, "\n");
    m.dispatchEvent(new Event("input", { bubbles: true }));
  }
  if (params.get("mockmenu") === "branch") document.querySelector(".branch-btn")?.click();
  if (params.get("mockmenu") === "more") document.querySelector(".gitbar .icon-btn:last-child")?.click();
  if (params.get("mockmenu") === "commit") document.querySelector(".split-more")?.click();
  if (params.get("mockmenu") === "worktrees") (await import("./worktrees.js")).openWorktreesPanel();
  if (params.get("mockmenu") === "newworktree") (await import("./worktrees.js")).newWorktreeDialog();
  if (params.get("mockconfirm") === "discard") document.querySelector(".gsec-changes .gsec-actions .icon-btn:nth-child(2)")?.click();
  // Typing U+001E (WebKit's "up arrow" character) must move the cursor up
  // and leave the document alone.
  if (params.has("mockcheckinput")) {
    await openFile("src/config.rs", { line: 3 });
    const v = activeView();
    v.focus();
    const before = v.state.doc.toString();
    const line = () => v.state.doc.lineAt(v.state.selection.main.head).number;
    const start = line();
    document.execCommand("insertText", false, "\x1e");
    await new Promise((r) => setTimeout(r, 50));
    const ok = v.state.doc.toString() === before && line() === start - 1;
    document.documentElement.dataset.inputCheck = ok ? "pass" : "fail";
    toast(ok ? `Typing U+001E moved the cursor from line ${start} to ${line()} and left the text alone.` : `Typing U+001E changed the text or didn't move the cursor (line ${start} → ${line()}).`, {
      kind: ok ? "success" : "error",
      title: ok ? "Input check passed" : "Input check failed",
      timeout: 60000,
    });
  }
  if (params.get("mocktoast")) {
    toast("To github.com:you/deck.git\n   3f92312..9be41a0  main -> main", { mono: true, title: "Pushed", timeout: 60000 });
    toast("fatal: Not possible to fast-forward, aborting.", { kind: "error", title: "Pull failed", timeout: 60000 });
  }
  if (params.get("mockdisk")) {
    const tab = activeTab();
    if (tab) {
      await api("write", { path: tab.path, text: "// changed elsewhere\n" + tab.disk.toString() });
      await refreshOpenFiles();
    }
  }
}

boot();
