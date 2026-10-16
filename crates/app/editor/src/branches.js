// The branch menu (search, recent / local / remote branches, worktrees) and
// the branch actions behind it: switch, create, merge, delete.

import { api } from "./api.js";
import { S, emit } from "./store.js";
import { h, clear, relTime, validateBranch, ltr, tildify } from "./util.js";
import { icon } from "./icons.js";
import { toast, toastError, confirmDialog, promptDialog, popover, closeMenu, menuOpenFor } from "./ui.js";
import { gitOp } from "./git.js";
import { loadWorktrees, openInDeck, worktreeName, worktreeBranch, openWorktreesPanel, newWorktreeDialog, invalidateWorktrees } from "./worktrees.js";

const RECENT = 4;

// ------------------------------------------------------------------ actions

export function switchBranch(name, create = false) {
  return gitOp(create ? "Creating branch…" : "Switching…", "switch", { name, create }, {
    success: create ? `Created and switched to ${name}` : `Switched to ${name}`,
    refs: true,
    after: (r) => r && emit("disk"),
  });
}

export async function newBranch() {
  const name = await promptDialog({
    title: "New Branch",
    message: `Create a branch from ${S.status?.branch || "HEAD"} and switch to it.`,
    placeholder: "feature/name",
    confirm: "Create Branch",
    validate: validateBranch,
  });
  if (name) await switchBranch(name, true);
}

export async function mergeBranch(name) {
  const into = S.status?.branch || "HEAD";
  const ok = await confirmDialog({
    title: `Merge “${name}” into “${into}”?`,
    message: "Fast-forwards when it can, otherwise makes a merge commit.",
    confirm: "Merge",
  });
  if (!ok) return;
  try {
    await gitOp("Merging…", "merge", { name }, { quiet: true, refs: true, output: true, success: `Merged ${name} into ${into}`, after: () => emit("disk") });
  } catch (e) {
    const conflicted = (S.status?.files || []).some((f) => f.staged === "U" || f.unstaged === "U");
    if (conflicted) {
      toast("Resolve the conflicted files, stage them, then commit to finish the merge.", {
        kind: "error",
        title: `Merging ${name} stopped on conflicts`,
        action: { label: "Show Conflicts", run: () => emit("view", "changes") },
        timeout: 12000,
      });
      emit("view", "changes");
    } else toastError(e, "Merge failed");
  }
}

export async function deleteBranch(b) {
  const ok = await confirmDialog({
    title: `Delete the branch “${b.name}”?`,
    message: b.merged ? `It's merged into ${S.status?.branch || "the current branch"}, so no commits are lost.` : "Only the local branch is deleted.",
    confirm: "Delete",
    danger: true,
  });
  if (!ok) return;
  try {
    await gitOp("Deleting branch…", "delete_branch", { name: b.name, force: false }, { quiet: true, refs: true, success: `Deleted ${b.name}` });
  } catch (e) {
    if (!/not fully merged/i.test(e.message)) {
      toastError(e, "Couldn't delete the branch");
      return;
    }
    const again = await confirmDialog({
      title: `“${b.name}” is not fully merged`,
      message: "Its commits that aren't on any other branch will be lost. Delete it anyway?",
      confirm: "Delete Anyway",
      danger: true,
    });
    if (again) await gitOp("Deleting branch…", "delete_branch", { name: b.name, force: true }, { refs: true, success: `Deleted ${b.name}` });
  }
}

// ------------------------------------------------------------------ menu

export function openBranchMenu(anchor) {
  if (menuOpenFor(anchor)) return closeMenu();
  const input = h("input", { class: "bm-input", placeholder: "Filter branches", spellcheck: false, autocomplete: "off", "aria-label": "Filter branches" });
  const list = h("div", { class: "bm-list", role: "listbox" });
  const el = h("div", { class: "menu bm" }, h("div", { class: "bm-head" }, icon("search", "bm-search"), input), list);
  let data = null;
  let error = null;
  let rows = []; // actionable rows: {el, run}
  let cur = 0;

  const run = (fn) => {
    closeMenu();
    fn();
  };

  const branchRow = (b, wts) => {
    const wt = !b.remote && wts.find((w) => !w.current && w.branch === b.name);
    const go = b.current
      ? () => {}
      : wt
        ? () => openInDeck("editor", wt.path)
        : () => switchBranch(b.remote ? b.name.replace(/^[^/]+\//, "") : b.name, false);
    const acts = [];
    if (!b.current) {
      acts.push(
        h(
          "button",
          {
            class: "icon-btn sm",
            title: wt ? "Open Its Worktree in Deck" : b.remote ? `Check Out ${b.name.replace(/^[^/]+\//, "")}` : "Switch to Branch",
            onClick: (e) => {
              e.stopPropagation();
              run(go);
            },
          },
          icon(wt ? "external" : "switchTo"),
        ),
      );
      acts.push(
        h(
          "button",
          {
            class: "icon-btn sm",
            title: `Merge into ${S.status?.branch || "HEAD"}`,
            onClick: (e) => {
              e.stopPropagation();
              run(() => mergeBranch(b.name));
            },
          },
          icon("merge"),
        ),
      );
      if (!b.remote && !wt)
        acts.push(
          h(
            "button",
            {
              class: "icon-btn sm danger",
              title: "Delete Branch…",
              onClick: (e) => {
                e.stopPropagation();
                run(() => deleteBranch(b));
              },
            },
            icon("trash"),
          ),
        );
    }
    const counts = [];
    if (b.ahead) counts.push(h("span", { class: "ab" }, `↑${b.ahead}`));
    if (b.behind) counts.push(h("span", { class: "ab" }, `↓${b.behind}`));
    const title = [
      b.name,
      b.subject && `“${b.subject}”`,
      b.upstream && `tracks ${b.upstream}`,
      wt && `checked out in ${tildify(wt.path)}`,
      b.current ? "current branch" : wt ? "click to open that worktree in Deck" : null,
    ]
      .filter(Boolean)
      .join("\n");
    const el = h(
      "div",
      {
        class: "bm-row" + (b.current ? " current" : ""),
        role: "option",
        title,
        onClick: () => run(go),
      },
      icon(b.current ? "check" : b.remote ? "cloud" : "branch", "bm-ic"),
      h("span", { class: "bm-name" }, b.name),
      h(
        "span",
        { class: "bm-meta" },
        counts,
        b.merged && !b.current && h("span", { class: "badge-chip" }, "merged"),
        wt && h("span", { class: "badge-chip", title: tildify(wt.path) }, icon("worktree"), "worktree"),
        b.date ? h("span", { class: "bm-time" }, relTime(b.date)) : null,
      ),
      acts.length ? h("span", { class: "bm-acts" }, acts) : null,
    );
    rows.push({ el, run: () => run(go) });
    return el;
  };

  const worktreeRow = (w) => {
    const el = h(
      "div",
      {
        class: "bm-row wt" + (w.current ? " current" : ""),
        role: "option",
        title: w.path + (w.current ? "\nthis worktree" : "\nopen in Deck"),
        onClick: () => run(() => !w.current && openInDeck("editor", w.path)),
      },
      icon(w.current ? "check" : "worktree", "bm-ic"),
      h("span", { class: "bm-name" }, worktreeName(w)),
      h("span", { class: "bm-dir" }, ltr(worktreeBranch(w))),
      h(
        "span",
        { class: "bm-meta" },
        w.locked && h("span", { class: "badge-chip" }, "locked"),
        w.prunable && h("span", { class: "badge-chip warn" }, "prunable"),
      ),
      !w.current &&
        h(
          "span",
          { class: "bm-acts" },
          h(
            "button",
            {
              class: "icon-btn sm",
              title: "Open Terminal",
              onClick: (e) => {
                e.stopPropagation();
                run(() => openInDeck("terminal", w.path));
              },
            },
            icon("terminal"),
          ),
        ),
    );
    rows.push({ el, run: () => run(() => !w.current && openInDeck("editor", w.path)) });
    return el;
  };

  const actionRow = (ic, label, fn, hint) => {
    const el = h("div", { class: "bm-row action", role: "option", onClick: () => run(fn) }, icon(ic, "bm-ic"), h("span", { class: "bm-name" }, label), hint && h("span", { class: "bm-meta" }, hint));
    rows.push({ el, run: () => run(fn) });
    return el;
  };

  const header = (label, n) => h("div", { class: "bm-header" }, label, n != null && h("span", { class: "bm-count" }, String(n)));

  const render = () => {
    clear(list);
    rows = [];
    const q = input.value.trim();
    const ql = q.toLowerCase();
    const match = (s) => !ql || String(s).toLowerCase().includes(ql);
    if (q) {
      const exists = data?.branches?.some((b) => !b.remote && b.name === q);
      if (!exists) {
        const bad = validateBranch(q);
        if (bad) list.append(h("div", { class: "bm-note" }, icon("warning"), `“${q}” isn't a valid branch name`));
        else list.append(actionRow("newBranch", `Create branch “${q}”`, () => switchBranch(q, true), `from ${S.status?.branch || "HEAD"}`));
      }
    } else list.append(actionRow("newBranch", "New Branch…", newBranch));
    if (error) list.append(h("div", { class: "bm-empty err" }, error));
    else if (!data) list.append(h("div", { class: "bm-empty" }, h("span", { class: "spinner" })));
    else {
      const wts = data.worktrees || [];
      const local = data.branches.filter((b) => !b.remote && match(b.name));
      const remote = data.branches.filter((b) => b.remote && !/\/HEAD$/.test(b.name) && match(b.name));
      let recent = [];
      let rest = local;
      if (!q && local.length > 3) {
        recent = local.slice().sort((a, b) => (b.current ? 1 : 0) - (a.current ? 1 : 0) || (b.date || 0) - (a.date || 0)).slice(0, RECENT);
        rest = local.filter((b) => !recent.includes(b));
      } else rest = local.slice().sort((a, b) => (b.current ? 1 : 0) - (a.current ? 1 : 0) || a.name.localeCompare(b.name));
      if (recent.length) {
        list.append(header("Recent"));
        for (const b of recent) list.append(branchRow(b, wts));
      }
      if (rest.length) {
        list.append(header("Local", rest.length));
        for (const b of rest) list.append(branchRow(b, wts));
      }
      if (remote.length) {
        list.append(header("Remote", remote.length));
        for (const b of remote) list.append(branchRow(b, wts));
      }
      const wtShown = wts.filter((w) => match(worktreeName(w)) || match(w.branch || ""));
      if (wts.length > 1 || !q) {
        list.append(header("Worktrees", wts.length || null));
        if (wts.length > 1) for (const w of wtShown) list.append(worktreeRow(w));
        if (!q) {
          list.append(actionRow("plus", "New Worktree…", () => newWorktreeDialog()));
          if (wts.length) list.append(actionRow("worktree", "Manage Worktrees…", () => openWorktreesPanel()));
        }
      }
      if (q && !local.length && !remote.length && !wtShown.length && list.childElementCount <= 1) list.append(h("div", { class: "bm-empty" }, "No branches match"));
    }
    cur = Math.min(cur, rows.length - 1);
    if (cur < 0) cur = 0;
    // With a query, start on the first branch rather than “Create branch”.
    if (q && rows.length > 1 && cur === 0 && rows[0].el.classList.contains("action")) cur = 1;
    rows[cur]?.el.classList.add("cur");
    for (const [i, r] of rows.entries()) r.el.addEventListener("mousemove", () => setCur(i, false));
  };
  const setCur = (i, scroll = true) => {
    if (i === cur || !rows[i]) return;
    rows[cur]?.el.classList.remove("cur");
    cur = i;
    rows[cur].el.classList.add("cur");
    if (scroll) rows[cur].el.scrollIntoView({ block: "nearest" });
  };
  input.addEventListener("input", () => {
    cur = 0;
    render();
  });
  const replace = popover(anchor, el, {
    align: "left",
    onKey: (e) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = rows.length;
        if (n) setCur((cur + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
      } else if (e.key === "Enter") {
        e.preventDefault();
        rows[cur]?.run();
      }
    },
  });
  render();
  input.focus();
  invalidateWorktrees();
  Promise.all([api("branches"), loadWorktrees(true).catch(() => [])])
    .then(([b, w]) => {
      data = { branches: b.branches || [], worktrees: w };
      render();
      replace();
    })
    .catch((e) => {
      error = e.message;
      render();
    });
}
