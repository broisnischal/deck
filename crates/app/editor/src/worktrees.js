// Worktrees: the list (for the branch menu and the Worktrees panel), opening
// one in Deck, adding and removing them.

import { api } from "./api.js";
import { S, emit } from "./store.js";
import { h, clear, basename, ltr, tildify, validateBranch } from "./util.js";
import { icon } from "./icons.js";
import { toast, toastError, confirmDialog, openModal, segmented } from "./ui.js";
import { gitOp } from "./git.js";

let cache = null;

export async function loadWorktrees(force = false) {
  if (!force && cache && Date.now() - cache.at < 5000) return cache.list;
  const r = await api("worktrees");
  cache = { list: r.worktrees || [], at: Date.now() };
  return cache.list;
}
export const cachedWorktrees = () => cache?.list || null;
export function invalidateWorktrees() {
  cache = null;
}

// Ask Deck to open a folder as an editor or a terminal. Outside Deck there is
// no IPC channel, so say what would have happened.
export function openInDeck(kind, path) {
  const msg = JSON.stringify({ deck: "open", kind, path });
  if (window.ipc && typeof window.ipc.postMessage === "function") {
    try {
      window.ipc.postMessage(msg);
      return;
    } catch {}
  }
  toast(tildify(path), { title: kind === "terminal" ? "Would open a terminal in" : "Would open in Deck" });
}

export const worktreeName = (w) => basename(String(w.path).replace(/\/+$/, "")) || w.path;
export const worktreeBranch = (w) => (w.branch ? w.branch : `detached at ${String(w.head || "").slice(0, 7) || "?"}`);

export function worktreeBadges(w) {
  return [
    w.current && h("span", { class: "badge-chip accent" }, "current"),
    w.locked && h("span", { class: "badge-chip", title: "Locked: git won't prune or remove it without force" }, icon("lockSmall"), "locked"),
    w.prunable && h("span", { class: "badge-chip warn", title: "Its folder is gone; `git worktree prune` would drop it" }, "prunable"),
  ];
}

// ------------------------------------------------------------------ remove

export async function removeWorktree(w) {
  const name = worktreeName(w);
  const ok = await confirmDialog({
    title: `Remove the worktree “${name}”?`,
    message: `Its folder, ${tildify(w.path)}, is deleted.${w.branch ? ` The branch “${w.branch}” is kept.` : ""}`,
    confirm: "Remove",
    danger: true,
  });
  if (!ok) return false;
  try {
    await gitOp("Removing worktree…", "worktree_remove", { path: w.path, force: false }, { quiet: true, refs: true, success: `Removed ${name}` });
  } catch (e) {
    const msg = String(e.message || e);
    if (!/force|-f -f|modified or untracked|locked|unlock/i.test(msg)) {
      toastError(e, "Couldn't remove the worktree");
      return false;
    }
    const locked = /locked/i.test(msg);
    const again = await confirmDialog({
      title: locked ? `“${name}” is locked` : `“${name}” has uncommitted changes`,
      message: locked
        ? "Someone locked this worktree so it isn't removed by accident. Remove it anyway?"
        : "Removing it deletes those changes for good. Remove it anyway?",
      items: [msg.split("\n")[0]],
      confirm: "Force Remove",
      danger: true,
    });
    if (!again) return false;
    const r = await gitOp("Removing worktree…", "worktree_remove", { path: w.path, force: true }, { refs: true, success: `Removed ${name}` });
    if (!r) return false;
  } finally {
    invalidateWorktrees();
  }
  return true;
}

// ------------------------------------------------------------------ panel

export function openWorktreesPanel() {
  return openModal(
    (box, done) => {
      const list = h("div", { class: "wt-list" }, h("div", { class: "wt-empty" }, h("span", { class: "spinner" })));
      const add = h(
        "button",
        {
          class: "btn btn-sm",
          onClick: async () => {
            const made = await newWorktreeDialog();
            if (made) refresh(true);
          },
        },
        icon("plus"),
        "New Worktree…",
      );
      box.append(
        h("div", { class: "dialog-head" }, h("div", { class: "dialog-title" }, "Worktrees"), h("span", { class: "grow" }), add),
        h("div", { class: "dialog-msg" }, "Each worktree is its own checkout of this repository, on its own branch, in its own folder."),
        list,
        h("div", { class: "dialog-actions" }, h("span", { class: "grow" }), h("button", { class: "btn", onClick: () => done(null) }, "Done")),
      );
      const row = (w) => {
        const acts = h(
          "span",
          { class: "wt-acts" },
          h("button", { class: "icon-btn", title: "Open in Deck", disabled: w.current || w.prunable, onClick: () => (done(null), openInDeck("editor", w.path)) }, icon("external")),
          h("button", { class: "icon-btn", title: "Open Terminal", disabled: w.prunable, onClick: () => (done(null), openInDeck("terminal", w.path)) }, icon("terminal")),
          !w.current &&
            h(
              "button",
              {
                class: "icon-btn danger",
                title: "Remove Worktree…",
                onClick: async () => {
                  if (await removeWorktree(w)) refresh(true);
                },
              },
              icon("trash"),
            ),
        );
        return h(
          "div",
          { class: "wt-row" + (w.current ? " current" : ""), title: w.path },
          icon("worktree", "wt-ic"),
          h(
            "div",
            { class: "wt-main" },
            h("div", { class: "wt-line" }, h("span", { class: "wt-name" }, worktreeName(w)), worktreeBadges(w)),
            h(
              "div",
              { class: "wt-line wt-sub" },
              h("span", { class: "wt-branch" + (w.branch ? "" : " detached") }, icon(w.branch ? "branch" : "commit"), worktreeBranch(w)),
              h("span", { class: "wt-path" }, ltr(tildify(w.path))),
            ),
          ),
          acts,
        );
      };
      const refresh = async (force) => {
        try {
          const all = await loadWorktrees(force);
          clear(list);
          for (const w of all) list.append(row(w));
          if (!all.length) list.append(h("div", { class: "wt-empty" }, "No worktrees"));
        } catch (e) {
          clear(list);
          list.append(h("div", { class: "wt-empty err" }, e.message));
        }
      };
      refresh(true);
      return { submit: () => done(null), focus: () => add.focus() };
    },
    { cls: "dialog-wide" },
  );
}

// ------------------------------------------------------------------ new

const slug = (b) => b.replace(/^refs\/heads\//, "").replace(/[/\s]+/g, "-").replace(/[^\w.-]/g, "");

// An input with a suggestion list under it (branch names).
function suggestField({ placeholder, value = "", suggest, onChange }) {
  const input = h("input", { class: "field", placeholder, value, spellcheck: false, autocomplete: "off" });
  const pop = h("div", { class: "suggest" });
  const wrap = h("div", { class: "suggest-wrap" }, input, pop);
  let items = [];
  let cur = -1;
  let open = false;
  const close = () => {
    open = false;
    pop.classList.remove("on");
  };
  const choose = (i) => {
    const it = items[i];
    if (!it) return;
    input.value = it.value;
    close();
    onChange?.(input.value);
  };
  const render = () => {
    items = suggest(input.value.trim()).slice(0, 8);
    clear(pop);
    cur = Math.min(cur, items.length - 1);
    items.forEach((it, i) =>
      pop.append(
        h(
          "div",
          {
            class: "suggest-item" + (i === cur ? " cur" : ""),
            onMouseDown: (e) => {
              e.preventDefault();
              choose(i);
            },
          },
          icon(it.remote ? "cloud" : "branch"),
          h("span", { class: "suggest-label" }, it.label),
          it.detail && h("span", { class: "suggest-detail" }, it.detail),
        ),
      ),
    );
    open = items.length > 0 && document.activeElement === input;
    pop.classList.toggle("on", open);
  };
  input.addEventListener("input", () => {
    cur = -1;
    render();
    onChange?.(input.value);
  });
  input.addEventListener("focus", render);
  input.addEventListener("blur", () => setTimeout(close, 80));
  return {
    wrap,
    input,
    refresh: render,
    // Arrow keys and Enter while the list is open.
    onKey(e) {
      if (!open || document.activeElement !== input) return false;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        const n = items.length;
        cur = cur < 0 ? (e.key === "ArrowDown" ? 0 : n - 1) : (cur + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
        [...pop.children].forEach((c, j) => c.classList.toggle("cur", j === cur));
        return true;
      }
      if ((e.key === "Enter" || e.key === "Tab") && cur >= 0) {
        e.preventDefault();
        e.stopPropagation();
        choose(cur);
        return true;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
        return true;
      }
      return false;
    },
  };
}

// Resolves to the new worktree's path, or null.
export async function newWorktreeDialog({ branch = "", create } = {}) {
  let branches = [];
  let worktrees = [];
  try {
    [branches, worktrees] = await Promise.all([api("branches").then((r) => r.branches || []), loadWorktrees()]);
  } catch (e) {
    toastError(e, "Couldn't list branches");
  }
  const local = branches.filter((b) => !b.remote);
  const used = new Set(worktrees.map((w) => w.branch).filter(Boolean));
  const current = worktrees.find((w) => w.current);
  const repo = current ? worktreeName(current) : S.name || "repo";
  const top = current?.path || S.root;
  const result = await openModal(
    (box, done) => {
      let mode = (create ?? !branch) ? "new" : "existing";
      let pathTouched = false;
      const err = h("div", { class: "dialog-err" });
      const ok = h("button", { class: "btn btn-primary", onClick: () => submit() }, "Create Worktree");
      const segHost = h("div", { class: "seg-row" });
      const name = suggestField({
        placeholder: "feature/name",
        value: branch,
        suggest: (q) => {
          const ql = q.toLowerCase();
          if (mode === "new") return [];
          const out = [];
          for (const b of local) if (!used.has(b.name) && b.name.toLowerCase().includes(ql)) out.push({ value: b.name, label: b.name, detail: b.upstream || "" });
          for (const b of branches.filter((x) => x.remote)) {
            const short = b.name.replace(/^[^/]+\//, "");
            if (local.some((l) => l.name === short) || !b.name.toLowerCase().includes(ql)) continue;
            out.push({ value: short, label: b.name, detail: "remote", remote: true });
          }
          return out;
        },
        onChange: () => sync(),
      });
      const base = suggestField({
        placeholder: S.status?.branch || "HEAD",
        value: S.status?.branch || "",
        suggest: (q) => {
          const ql = q.toLowerCase();
          return branches.filter((b) => b.name.toLowerCase().includes(ql)).map((b) => ({ value: b.name, label: b.name, remote: b.remote }));
        },
        onChange: () => check(),
      });
      const path = h("input", { class: "field", spellcheck: false, autocomplete: "off", placeholder: `../${repo}-branch` });
      path.addEventListener("input", () => {
        pathTouched = true;
        check();
      });
      const baseRow = h("label", { class: "form-row" }, h("span", { class: "form-label" }, "Based on"), base.wrap);
      const check = () => {
        const b = name.input.value.trim();
        let msg = null;
        if (!b) msg = "";
        else if (validateBranch(b)) msg = validateBranch(b);
        else if (mode === "new" && branches.some((x) => !x.remote && x.name === b)) msg = `A branch named “${b}” already exists. Use Existing branch.`;
        else if (mode === "existing" && used.has(b)) msg = `“${b}” is already checked out in another worktree.`;
        else if (mode === "existing" && !local.some((x) => x.name === b) && !branches.some((x) => x.remote && x.name.replace(/^[^/]+\//, "") === b)) msg = `There is no branch named “${b}”.`;
        else if (!path.value.trim()) msg = "Choose a folder.";
        err.textContent = msg || "";
        ok.disabled = msg !== null;
        return msg === null;
      };
      const sync = () => {
        if (!pathTouched) path.value = name.input.value.trim() ? `../${repo}-${slug(name.input.value.trim())}` : "";
        check();
      };
      const renderSeg = () => {
        clear(segHost);
        segHost.append(
          segmented(
            [
              ["new", "New branch"],
              ["existing", "Existing branch"],
            ],
            mode,
            (v) => {
              mode = v;
              renderSeg();
              baseRow.style.display = mode === "new" ? "" : "none";
              name.input.placeholder = mode === "new" ? "feature/name" : "Branch to check out";
              check();
              name.input.focus();
            },
          ),
        );
      };
      renderSeg();
      baseRow.style.display = mode === "new" ? "" : "none";
      const submit = () => {
        if (!check()) return;
        done({ branch: name.input.value.trim(), create: mode === "new", base: mode === "new" ? base.input.value.trim() || null : null, path: path.value.trim() });
      };
      box.append(
        h("div", { class: "dialog-title" }, "New Worktree"),
        h("div", { class: "dialog-msg" }, "Check out a branch in a folder of its own, so you can work on it next to this one."),
        segHost,
        h("label", { class: "form-row" }, h("span", { class: "form-label" }, "Branch"), name.wrap),
        baseRow,
        h(
          "label",
          { class: "form-row" },
          h("span", { class: "form-label" }, "Folder"),
          path,
          h("span", { class: "form-hint" }, "Relative to ", h("span", { class: "mono" }, tildify(top))),
        ),
        err,
        h("div", { class: "dialog-actions" }, h("span", { class: "grow" }), h("button", { class: "btn", onClick: () => done(null) }, "Cancel"), ok),
      );
      sync();
      return { submit, focus: () => name.input.focus(), onKey: (e) => name.onKey(e) || base.onKey(e) };
    },
    { cls: "dialog-form" },
  );
  if (!result) return null;
  const r = await gitOp("Creating worktree…", "worktree_add", result, { refs: true });
  invalidateWorktrees();
  if (!r) return null;
  const where = r.path || result.path;
  toast(tildify(where), {
    kind: "success",
    title: `Created a worktree for ${result.branch}`,
    action: { label: "Open in Deck", run: () => openInDeck("editor", where) },
  });
  emit("worktrees");
  return where;
}
