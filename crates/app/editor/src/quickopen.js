// ⌘P Quick Open: fuzzy search over a file index built with repeated `ls`.

import { api } from "./api.js";
import { h, clear, basename, join } from "./util.js";
import { icon, fileIcon } from "./icons.js";
import { closeMenu } from "./ui.js";
import { openFile, recentPaths } from "./editor.js";

const SKIP = new Set(["node_modules", "target", "dist", ".next", "build", "vendor", ".git"]);
const CAP = 5000;
const MAX_RESULTS = 80;

const index = { files: [], done: false, building: null, stale: true, truncated: false };
let ui = null;

export function invalidateIndex() {
  index.stale = true;
}

function buildIndex(onProgress) {
  if (index.building) return index.building;
  const files = [];
  // A rebuild keeps showing the previous (stale) list until it finishes.
  const progressive = !index.files.length;
  index.done = false;
  index.truncated = false;
  const queue = [""];
  let active = 0;
  index.building = new Promise((resolve) => {
    let last = 0;
    const pump = () => {
      if (files.length >= CAP) queue.length = 0;
      while (active < 6 && queue.length) {
        const dir = queue.shift();
        active++;
        api("ls", { dir })
          .then((r) => {
            for (const e of r.entries || []) {
              if (e.dir) {
                if (!SKIP.has(e.name)) queue.push(join(dir, e.name));
              } else if (files.length < CAP) files.push(join(dir, e.name));
              else index.truncated = true;
            }
          })
          .catch(() => {})
          .finally(() => {
            active--;
            const now = performance.now();
            if (progressive && now - last > 120) {
              last = now;
              index.files = files.slice();
              onProgress?.();
            }
            if (!queue.length && !active) {
              index.files = files;
              index.done = true;
              index.stale = false;
              index.building = null;
              onProgress?.();
              resolve();
            } else pump();
          });
      }
    };
    pump();
  });
  return index.building;
}

// ------------------------------------------------------------------ matching

const SEP = new Set(["/", ".", "_", "-", " "]);
function isBoundary(s, i) {
  if (i === 0) return true;
  const p = s[i - 1], c = s[i];
  return SEP.has(p) || (p === p.toLowerCase() && c !== c.toLowerCase());
}

function canMatch(q, qi, sl, from) {
  for (; qi < q.length; qi++) {
    from = sl.indexOf(q[qi], from);
    if (from < 0) return false;
    from++;
  }
  return true;
}

// Greedy subsequence match from `start`, preferring word starts when the
// rest of the query still fits after them.
function matchFrom(q, s, sl, start) {
  let si = start, prev = -2, score = 0;
  const pos = [];
  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi];
    let at = sl.indexOf(ch, si);
    if (at < 0) return null;
    if (at !== prev + 1) {
      for (let j = at; j < sl.length && j < at + 24; j++) {
        if (sl[j] === ch && isBoundary(s, j)) {
          if (canMatch(q, qi + 1, sl, j + 1)) at = j;
          break;
        }
      }
    }
    score += 1;
    if (at === prev + 1) score += 5;
    if (isBoundary(s, at)) score += 6;
    score -= Math.min(at - si, 12) * 0.15;
    pos.push(at);
    prev = at;
    si = at + 1;
  }
  return { score, pos };
}

export function fuzzy(q, s) {
  const sl = s.toLowerCase();
  const slash = s.lastIndexOf("/") + 1;
  const inName = matchFrom(q, s, sl, slash);
  let best = null;
  if (inName) {
    inName.score += 12;
    if (inName.pos[0] === slash) inName.score += 8;
    const name = sl.slice(slash);
    const sub = name.indexOf(q);
    if (sub >= 0) inName.score += 10 + (isBoundary(s, slash + sub) ? 4 : 0);
    if (name.startsWith(q)) inName.score += 6;
    if (name === q || name.replace(/\.[^.]*$/, "") === q) inName.score += 10;
    best = inName;
  }
  const whole = matchFrom(q, s, sl, 0);
  if (whole && (!best || whole.score > best.score)) best = whole;
  if (!best) return null;
  best.score -= s.length * 0.01;
  return best;
}

function search(query) {
  const q = query.replace(/\s+/g, "").toLowerCase();
  if (!q) {
    const recent = recentPaths();
    const set = new Set(recent);
    const rest = index.files.filter((f) => !set.has(f)).slice(0, MAX_RESULTS - recent.length);
    return [...recent.map((path) => ({ path, pos: [], recent: true })), ...rest.map((path) => ({ path, pos: [] }))];
  }
  const out = [];
  const recent = new Set(recentPaths().slice(0, 10));
  for (const path of index.files) {
    const m = fuzzy(q, path);
    if (m) out.push({ path, pos: m.pos, score: m.score + (recent.has(path) ? 3 : 0) });
  }
  out.sort((a, b) => b.score - a.score || a.path.length - b.path.length);
  return out.slice(0, MAX_RESULTS);
}

// ------------------------------------------------------------------ UI

function marked(text, offset, pos) {
  if (!pos.length) return text;
  const set = new Set(pos.map((p) => p - offset).filter((p) => p >= 0 && p < text.length));
  if (!set.size) return text;
  const out = [];
  let run = "", inMark = false;
  const flush = () => {
    if (run) out.push(inMark ? h("mark", null, run) : run);
    run = "";
  };
  for (let i = 0; i < text.length; i++) {
    const m = set.has(i);
    if (m !== inMark) {
      flush();
      inMark = m;
    }
    run += text[i];
  }
  flush();
  return out;
}

export function isQuickOpenOpen() {
  return !!ui;
}

export function closeQuickOpen() {
  if (!ui) return;
  const { scrim, prevFocus } = ui;
  ui = null;
  scrim.remove();
  if (prevFocus?.isConnected) prevFocus.focus?.();
}

export function openQuickOpen(initial = "") {
  if (ui) {
    ui.input.focus();
    ui.input.select();
    return;
  }
  closeMenu();
  const input = h("input", { class: "qo-input", placeholder: "Search files by name", spellcheck: false, autocomplete: "off", value: initial });
  const list = h("div", { class: "qo-list", role: "listbox" });
  const foot = h("div", { class: "qo-foot" });
  const box = h("div", { class: "qo", role: "dialog", "aria-label": "Quick Open" }, h("div", { class: "qo-head" }, icon("search", "qo-ic"), input), list, foot);
  const scrim = h("div", { class: "qo-scrim", onMouseDown: (e) => e.target === scrim && closeQuickOpen() }, box);
  ui = { scrim, input, list, foot, results: [], cur: 0, prevFocus: document.activeElement };
  document.body.append(scrim);

  const parseQuery = () => {
    const m = /^(.*?):(\d+)(?::(\d+))?$/.exec(input.value.trim());
    return m ? { q: m[1], line: +m[2], col: m[3] ? +m[3] : undefined } : { q: input.value.trim() };
  };

  const render = () => {
    if (!ui) return;
    const { q } = parseQuery();
    ui.results = search(q);
    ui.cur = Math.min(ui.cur, Math.max(0, ui.results.length - 1));
    clear(list);
    if (!ui.results.length) {
      list.append(h("div", { class: "qo-empty" }, index.done ? (q ? "No matching files" : "No files") : "Indexing…"));
    }
    ui.results.forEach((r, i) => {
      const slash = r.path.lastIndexOf("/") + 1;
      const dir = r.path.slice(0, Math.max(0, slash - 1));
      list.append(
        h(
          "div",
          {
            class: "qo-row" + (i === ui.cur ? " cur" : ""),
            role: "option",
            onMouseMove: () => setCur(i, false),
            onMouseDown: (e) => {
              e.preventDefault();
              choose(i);
            },
          },
          icon(fileIcon(basename(r.path)), "qo-file-ic"),
          h("span", { class: "qo-name" }, marked(r.path.slice(slash), slash, r.pos)),
          h("span", { class: "qo-dir" }, marked(dir, 0, r.pos)),
          r.recent && h("span", { class: "qo-tag" }, "open"),
        ),
      );
    });
    const n = index.files.length;
    clear(foot);
    foot.append(
      h("span", null, index.done ? `${n.toLocaleString()} file${n === 1 ? "" : "s"}${index.truncated ? " (capped)" : ""}` : `Indexing… ${n.toLocaleString()} files`),
      h("span", { class: "grow" }),
      h("span", { class: "qo-keys" }, h("kbd", null, "↑↓"), " navigate ", h("kbd", null, "↵"), " open ", h("kbd", null, "esc"), " close"),
    );
  };

  const setCur = (i, scroll = true) => {
    if (!ui || i === ui.cur) return;
    list.children[ui.cur]?.classList.remove("cur");
    ui.cur = i;
    const el = list.children[i];
    el?.classList.add("cur");
    if (scroll) el?.scrollIntoView({ block: "nearest" });
  };

  const choose = (i) => {
    const r = ui?.results[i];
    if (!r) return;
    const { line, col } = parseQuery();
    closeQuickOpen();
    openFile(r.path, { line, col });
  };

  input.addEventListener("input", () => {
    ui.cur = 0;
    render();
    list.scrollTop = 0;
  });
  input.addEventListener("keydown", (e) => {
    const n = ui.results.length;
    const down = e.key === "ArrowDown" || (e.ctrlKey && e.key === "n");
    const up = e.key === "ArrowUp" || (e.ctrlKey && e.key === "p");
    if (down || up) {
      e.preventDefault();
      if (n) setCur((ui.cur + (down ? 1 : -1) + n) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(ui.cur);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeQuickOpen();
    } else if (e.key === "Tab") e.preventDefault();
  });

  render();
  input.focus();
  input.select();
  if (index.stale || !index.done) buildIndex(render);
}
