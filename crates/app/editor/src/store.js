// Shared app state and a tiny event bus.

import { ancestors } from "./util.js";

export const S = {
  name: "",
  root: "",
  git: false,
  view: "files",
  status: null,
  statusSig: "",
  byPath: new Map(), // path -> status entry
  dirTints: new Map(), // dir -> strongest tint among its changed descendants
  headSig: "", // branch/upstream/ahead/behind: changes when refs probably moved
};

const subs = new Map();
export function on(evt, fn) {
  if (!subs.has(evt)) subs.set(evt, []);
  subs.get(evt).push(fn);
}
export function emit(evt, ...args) {
  for (const fn of subs.get(evt) || []) fn(...args);
}

// Colour bucket for a status entry: drives tints in the tree and tabs.
export function tintOf(e) {
  if (!e) return null;
  if (e.staged === "U" || e.unstaged === "U") return "conflict";
  if (e.unstaged === "?") return "untracked";
  if (e.unstaged === "D" || (e.staged === "D" && !e.unstaged)) return "deleted";
  if (e.staged === "A" && !e.unstaged) return "added";
  if (e.staged === "R" && !e.unstaged) return "renamed";
  return "modified";
}

const RANK = { conflict: 5, deleted: 2, modified: 4, renamed: 3, added: 1, untracked: 1 };

export function letterOf(e) {
  if (!e) return "";
  if (e.unstaged === "U" || e.staged === "U") return "U";
  if (e.unstaged === "?") return "?";
  return e.unstaged || e.staged;
}

// Returns true when the status actually changed.
export function setStatus(st) {
  const sig = JSON.stringify(st);
  if (sig === S.statusSig) return false;
  S.statusSig = sig;
  S.status = st;
  S.byPath = new Map();
  S.dirTints = new Map();
  for (const f of st?.files || []) {
    S.byPath.set(f.path, f);
    const t = tintOf(f);
    for (const d of ancestors(f.path)) {
      const cur = S.dirTints.get(d);
      if (!cur || RANK[t] > RANK[cur]) S.dirTints.set(d, t);
    }
  }
  return true;
}

export const isConflict = (f) => f.staged === "U" || f.unstaged === "U";

// The Changes panel's groups. A file with staged and unstaged edits is in
// both Staged and Changes; conflicts and untracked files only in their own.
export function sections() {
  const out = { conflicts: [], staged: [], changes: [], untracked: [] };
  for (const f of S.status?.files || []) {
    if (isConflict(f)) out.conflicts.push(f);
    else if (f.unstaged === "?") out.untracked.push(f);
    else {
      if (f.staged) out.staged.push(f);
      if (f.unstaged) out.changes.push(f);
    }
  }
  return out;
}

// What the branch bar shows; a change means the history may be out of date.
export function headSignature(st) {
  return st ? [st.branch, st.detached, st.upstream, st.ahead, st.behind].join("|") : "";
}
