// The bar on top of the git views: the branch button (opens the branch menu),
// one sync button that offers the next useful remote action, and a ⋯ menu.

import { S, emit } from "./store.js";
import { h, append, clear, plural } from "./util.js";
import { icon } from "./icons.js";
import { showMenu } from "./ui.js";
import { gitOp, gitBusy } from "./git.js";
import { openBranchMenu, newBranch } from "./branches.js";
import { openWorktreesPanel, loadWorktrees } from "./worktrees.js";

const els = {};
let detachedShort = null;

export function mountBranchBar(root) {
  els.root = root;
  renderBranchBar();
}

export function fetchRemote() {
  return gitOp("Fetching…", "fetch", {}, { output: true, success: "Fetched", refs: true });
}
export function pull() {
  return gitOp("Pulling…", "pull", {}, { output: true, success: "Pulled", refs: true, after: (r) => r && emit("disk") });
}
export function push() {
  const publish = !S.status?.upstream;
  return gitOp(publish ? "Publishing…" : "Pushing…", "push", {}, { output: true, success: publish ? "Published" : "Pushed", refs: true });
}
function stash() {
  return gitOp("Stashing…", "stash", {}, { output: true, success: "Stashed", after: (r) => r && emit("disk") });
}
function unstash() {
  return gitOp("Popping stash…", "unstash", {}, { output: true, success: "Stash applied", after: (r) => r && emit("disk") });
}

// The one remote action worth a button, like GitHub Desktop.
function syncAction(st) {
  if (!st) return null;
  if (st.detached || !st.branch) return { label: "Fetch", ic: "refresh", run: fetchRemote, title: "Fetch from the remote" };
  if (!st.upstream) return { label: "Publish", ic: "cloudUp", run: push, title: `Push ${st.branch} and track it on the remote` };
  if (st.behind)
    return {
      label: "Pull",
      count: `↓${st.behind}`,
      ic: "down",
      run: pull,
      title: `Pull ${plural(st.behind, "commit")} from ${st.upstream}${st.ahead ? ` (you also have ${st.ahead} to push)` : ""}`,
    };
  if (st.ahead) return { label: "Push", count: `↑${st.ahead}`, ic: "up", run: push, title: `Push ${plural(st.ahead, "commit")} to ${st.upstream}` };
  return { label: "Fetch", ic: "refresh", run: fetchRemote, title: `Up to date with ${st.upstream}. Fetch to check for new commits.` };
}

export function renderBranchBar() {
  if (!els.root) return;
  clear(els.root);
  const st = S.status;
  const busy = gitBusy();
  if (st?.detached && !detachedShort) {
    loadWorktrees()
      .then((list) => {
        const cur = list.find((w) => w.current);
        if (cur?.head && S.status?.detached) {
          detachedShort = cur.head.slice(0, 7);
          renderBranchBar();
        }
      })
      .catch(() => {});
  }
  if (!st?.detached) detachedShort = null;
  const name = st?.branch || (st?.detached ? (detachedShort ? detachedShort : "Detached HEAD") : st ? "No branch" : "…");
  const counts = [];
  if (st?.upstream && st.ahead) counts.push(h("span", { class: "ab" }, `↑${st.ahead}`));
  if (st?.upstream && st.behind) counts.push(h("span", { class: "ab" }, `↓${st.behind}`));
  const title = st?.detached
    ? `Detached HEAD${detachedShort ? ` at ${detachedShort}` : ""}: commits made here belong to no branch`
    : st?.upstream
      ? `${name} → ${st.upstream}${st.ahead || st.behind ? ` (${st.ahead} ahead, ${st.behind} behind)` : ""}`
      : `${name} (not published)`;
  els.branchBtn = h(
    "button",
    { class: "branch-btn" + (st?.detached ? " detached" : ""), title, onClick: (e) => openBranchMenu(e.currentTarget) },
    icon(st?.detached ? "commit" : "branch", "branch-ic"),
    h("span", { class: "branch-name" }, name),
    st?.detached && h("span", { class: "badge-chip warn" }, "detached"),
    counts.length ? h("span", { class: "branch-ab" }, counts) : null,
    icon("chevronDown", "branch-caret"),
  );
  const a = syncAction(st);
  const sync = busy
    ? h("span", { class: "sync-btn busy", title: busy }, h("span", { class: "spinner" }), h("span", { class: "sync-label" }, busy.replace(/…$/, "")))
    : a &&
      h(
        "button",
        { class: "sync-btn", title: a.title, onClick: () => a.run() },
        icon(a.ic),
        h("span", { class: "sync-label" }, a.label),
        a.count && h("span", { class: "sync-count" }, a.count),
      );
  append(els.root, [
    els.branchBtn,
    h("span", { class: "grow" }),
    sync,
    h("button", { class: "icon-btn", title: "More Git Actions", onClick: (e) => moreMenu(e.currentTarget) }, icon("more")),
  ]);
}

function moreMenu(anchor) {
  const st = S.status || {};
  const busy = !!gitBusy();
  const hasChanges = (st.files || []).some((f) => f.unstaged !== "?");
  showMenu(
    anchor,
    [
      { label: "Fetch", icon: "refresh", disabled: busy, onSelect: fetchRemote },
      { label: "Pull", icon: "down", hint: st.behind ? `↓${st.behind}` : null, disabled: busy || !st.upstream, onSelect: pull },
      st.upstream || !st.branch
        ? { label: "Push", icon: "up", hint: st.ahead ? `↑${st.ahead}` : null, disabled: busy || !st.branch, onSelect: push }
        : { label: "Publish Branch", icon: "cloudUp", disabled: busy, onSelect: push },
      "-",
      { label: "Stash Changes", icon: "stash", disabled: busy || !hasChanges, onSelect: stash },
      { label: "Pop Stash", icon: "unstash", disabled: busy, onSelect: unstash },
      "-",
      { label: "Switch Branch…", icon: "branch", onSelect: () => openBranchMenu(els.branchBtn) },
      { label: "New Branch…", icon: "newBranch", disabled: busy, onSelect: newBranch },
      { label: "Worktrees…", icon: "worktree", onSelect: openWorktreesPanel },
      "-",
      { label: "Refresh", icon: "sync", onSelect: () => emit("refresh") },
    ],
    { align: "right" },
  );
}

export function openBranchMenuFromBar() {
  if (els.branchBtn) openBranchMenu(els.branchBtn);
}
export function openMoreMenu() {
  const btn = els.root?.querySelector(".icon-btn:last-child");
  if (btn) moreMenu(btn);
}
