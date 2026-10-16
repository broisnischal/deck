// The commit view: an editor tab with the commit's message, author, hashes
// and refs, its changed files, and a read-only diff of the selected file.
// One tab is reused as you click through the history.

import { EditorState, Compartment } from "@codemirror/state";
import { EditorView, lineNumbers, highlightSpecialChars, drawSelection, keymap } from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { unifiedMergeView, MergeView, goToNextChunk, goToPreviousChunk, getChunks } from "@codemirror/merge";

import { api } from "./api.js";
import { emit } from "./store.js";
import { h, append, clear, basename, dirname, prefs, ltr, plural, relTimeLong, fullDate, initials, copyText, formatSize } from "./util.js";
import { icon, fileIcon } from "./icons.js";
import { segmented } from "./ui.js";
import { editorTheme, highlight, currentTheme } from "./theme.js";
import { languageFor, WRAP } from "./languages.js";
import { controlGuard } from "./inputguard.js";
import { pillsHtml, avatarClass } from "./refs.js";
import { openCustomTab, openFile } from "./editor.js";

const DIFF_CONF = { scanLimit: 5000, timeout: 400 };
const TINT = { M: "modified", A: "added", D: "deleted", R: "renamed", C: "renamed", T: "modified", U: "conflict" };
const dark = new Compartment();
let shown = null;
let gen = 0;

export const shownCommit = () => shown;

export function openCommit(hash, { preview = null, focus = false } = {}) {
  if (!hash) return;
  const short = preview?.short || String(hash).slice(0, 7);
  const subject = preview?.subject || "";
  openCustomTab({
    path: "commit:" + hash,
    reuse: "commit",
    name: subject ? `${short} ${subject}` : short,
    title: subject ? `${short} · ${subject}` : `Commit ${short}`,
    icon: "commit",
    label: () => [h("span", { class: "tab-hash" }, short), subject && h("span", { class: "tab-subj" }, subject)],
    mount: (host, tab) => mount(host, tab, hash, preview, focus),
  });
}

function mount(host, tab, hash, preview, focusAfter) {
  const my = ++gen;
  shown = hash;
  const st = {
    data: null,
    sel: -1,
    mode: prefs.get("cv:mode", "inline"),
    fold: prefs.get("cv:fold", true),
    views: [],
    split: null,
    fileGen: 0,
  };
  const els = {};
  els.head = h("div", { class: "cv-head" });
  els.filesHead = h("div", { class: "cv-files-head" });
  els.files = h("div", { class: "cv-files", tabIndex: 0, role: "listbox", "aria-label": "Changed files", onKeyDown: onFilesKey });
  els.bar = h("div", { class: "cv-bar" });
  els.diff = h("div", { class: "cv-diffhost" });
  els.root = h(
    "div",
    { class: "cv" },
    els.head,
    h("div", { class: "cv-main" }, h("div", { class: "cv-side" }, els.filesHead, els.files), h("div", { class: "cv-diff" }, els.bar, els.diff)),
  );
  host.append(els.root);
  if (preview) renderHead(preview, true);
  else renderHead({ hash, subject: "" }, true);
  els.files.append(h("div", { class: "cv-loading" }, h("span", { class: "spinner" })));
  emit("commit-shown", hash);

  api("show", { hash })
    .then((d) => {
      if (my !== gen) return;
      st.data = d;
      shown = d.hash;
      const short = d.short || d.hash.slice(0, 7);
      tab.update?.({
        name: `${short} ${d.subject}`,
        title: `${short} · ${d.subject}`,
        label: () => [h("span", { class: "tab-hash" }, short), h("span", { class: "tab-subj" }, d.subject)],
      });
      renderHead(d, false);
      renderFiles();
      if (d.files?.length) selectFile(0, { focus: focusAfter });
      else showMessage("commit", "No file changes", d.parents?.length > 1 ? "This merge took one side as is." : "This commit changes no files.");
    })
    .catch((e) => {
      if (my !== gen) return;
      clear(els.files);
      clear(els.filesHead);
      showMessage("warning", "Couldn't load this commit", e.message || String(e));
    });

  // ---------------------------------------------------------------- header

  function hashButton(full) {
    const label = h("span", { class: "mono" }, full.slice(0, 7));
    const ic = icon("copy");
    const b = h(
      "button",
      {
        class: "hash-pill",
        title: `Copy ${full}`,
        onClick: async () => {
          if (await copyText(full)) {
            b.classList.add("done");
            ic.replaceWith((b._check ||= icon("check")));
            setTimeout(() => {
              b.classList.remove("done");
              b._check?.replaceWith(ic);
            }, 1200);
          }
        },
      },
      label,
      ic,
    );
    return b;
  }

  function renderHead(d, partial) {
    clear(els.head);
    const parents = d.parents || [];
    const body = String(d.body || "").replace(/\s+$/, "");
    let bodyEl = null;
    if (body) {
      const long = body.split("\n").length > 6 || body.length > 480;
      bodyEl = h("div", { class: "cv-body" + (long ? " clamped" : "") }, body);
      if (long) {
        const more = h(
          "button",
          {
            class: "link-btn",
            onClick: () => {
              const c = bodyEl.classList.toggle("clamped");
              more.textContent = c ? "Show more" : "Show less";
            },
          },
          "Show more",
        );
        bodyEl = h("div", null, bodyEl, more);
      }
    }
    const refs = h("span", { class: "cv-refs" });
    if (d.refs?.length) refs.innerHTML = pillsHtml(d.refs, 0);
    append(els.head, [
      h("div", { class: "cv-subject" }, d.subject || (partial ? "" : "(no subject)")),
      bodyEl,
      !partial || d.author
        ? h(
            "div",
            { class: "cv-meta" },
            d.author && h("span", { class: avatarClass(d.email, d.author) + " lg" }, initials(d.author)),
            d.author && h("span", { class: "cv-author", title: d.email ? `${d.author} <${d.email}>` : d.author }, d.author),
            d.date && h("span", { class: "cv-date", title: fullDate(d.date) }, `committed ${relTimeLong(d.date)}`),
          )
        : null,
      h(
        "div",
        { class: "cv-ids" },
        d.hash && hashButton(d.hash),
        parents.length > 0 && h("span", { class: "cv-label" }, parents.length > 1 ? "Parents" : "Parent"),
        parents.map((p) => h("button", { class: "hash-link mono", title: `Open ${p.slice(0, 7)}`, onClick: () => openCommit(p) }, p.slice(0, 7))),
        !partial && !parents.length && h("span", { class: "cv-label" }, "Root commit"),
        refs,
      ),
      parents.length > 1 &&
        h(
          "div",
          { class: "cv-note" },
          icon("merge"),
          h("span", null, `Merge of ${parents.length} parents. Changes are shown against the first parent, `, h("span", { class: "mono" }, parents[0].slice(0, 7)), "."),
        ),
    ]);
  }

  // ---------------------------------------------------------------- files

  function renderFiles() {
    const files = st.data.files || [];
    let add = 0, del = 0;
    for (const f of files) {
      add += f.added || 0;
      del += f.removed || 0;
    }
    clear(els.filesHead);
    els.filesHead.append(
      h("span", { class: "cv-files-title" }, plural(files.length, "file") + " changed"),
      h("span", { class: "grow" }),
      h("span", { class: "stats" }, h("span", { class: "add" }, `+${add}`), h("span", { class: "del" }, `−${del}`)),
    );
    clear(els.files);
    files.forEach((f, i) => {
      const name = basename(f.path);
      const dir = dirname(f.path);
      const renamed = f.orig && f.orig !== f.path;
      const nameEl = renamed && basename(f.orig) !== name ? [h("span", { class: "crow-old" }, basename(f.orig)), h("span", { class: "crow-arrow" }, " → "), name] : name;
      const dirText = renamed && dirname(f.orig) !== dir ? `${dirname(f.orig) || "."} → ${dir || "."}` : dir;
      els.files.append(
        h(
          "div",
          {
            class: "crow file cv-file" + (i === st.sel ? " open" : "") + (f.status === "D" ? " deleted" : ""),
            role: "option",
            title: renamed ? `${f.orig} → ${f.path}` : f.path,
            onClick: () => selectFile(i),
          },
          h("span", { class: "letter tint-" + (TINT[f.status] || "modified") }, f.status),
          icon(fileIcon(name), "crow-ic"),
          h("span", { class: "crow-label" }, h("span", { class: "crow-name" }, nameEl), dirText && h("span", { class: "crow-dir" }, ltr(dirText))),
          h(
            "span",
            { class: "stats crow-stats" },
            f.added == null && f.removed == null
              ? h("span", { class: "bin" }, "bin")
              : [f.added ? h("span", { class: "add" }, `+${f.added}`) : null, f.removed ? h("span", { class: "del" }, `−${f.removed}`) : null],
          ),
        ),
      );
    });
  }

  function onFilesKey(e) {
    const n = st.data?.files?.length || 0;
    if (!n) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      selectFile(Math.max(0, Math.min(n - 1, st.sel + (e.key === "ArrowDown" ? 1 : -1))));
      els.files.children[st.sel]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      ctl.view()?.focus();
    }
  }

  // ---------------------------------------------------------------- diff

  function destroyViews() {
    st.split?.destroy();
    st.split = null;
    for (const v of st.views) v.destroy();
    st.views = [];
    clear(els.diff);
  }

  function showMessage(ic, title, detail) {
    destroyViews();
    els.diff.append(
      h("div", { class: "message" }, h("div", { class: "message-ic" }, icon(ic)), h("div", { class: "message-title" }, title), detail && h("div", { class: "message-detail" }, detail)),
    );
  }

  function renderBar(f) {
    clear(els.bar);
    if (!f) return;
    const renamed = f.orig && f.orig !== f.path;
    append(els.bar, [
      icon(fileIcon(basename(f.path)), "cv-bar-ic"),
      h("span", { class: "cv-bar-path", title: renamed ? `${f.orig} → ${f.path}` : f.path }, ltr(renamed ? `${f.orig} → ${f.path}` : f.path)),
      f.added != null || f.removed != null ? h("span", { class: "stats" }, h("span", { class: "add" }, `+${f.added || 0}`), h("span", { class: "del" }, `−${f.removed || 0}`)) : null,
      h("span", { class: "grow" }),
      h("button", { class: "tool-btn icon-only nav", title: "Previous change", onClick: () => step(-1) }, icon("arrowUp")),
      h("button", { class: "tool-btn icon-only nav", title: "Next change", onClick: () => step(1) }, icon("arrowDown")),
      h(
        "button",
        {
          class: "tool-btn icon-only nav" + (st.fold ? " on" : ""),
          title: st.fold ? "Show all lines" : "Collapse unchanged lines",
          onClick: () => {
            st.fold = !st.fold;
            prefs.set("cv:fold", st.fold);
            selectFile(st.sel, { keep: true });
          },
        },
        icon("fold"),
      ),
      segmented(
        [
          ["inline", "", "inline", "Inline"],
          ["split", "", "split", "Side by side"],
        ],
        st.mode,
        (v) => {
          st.mode = v;
          prefs.set("cv:mode", v);
          selectFile(st.sel, { keep: true });
        },
      ),
      f.status !== "D" && h("span", { class: "tool-sep" }),
      f.status !== "D" && h("button", { class: "tool-btn icon-only", title: "Open the working copy", onClick: () => openFile(f.path, { diff: false }) }, icon("openFile")),
    ]);
  }

  function step(dir) {
    const v = ctl.view();
    if (!v) return;
    (dir > 0 ? goToNextChunk : goToPreviousChunk)(v);
    v.dispatch({ effects: EditorView.scrollIntoView(v.state.selection.main.head, { y: "center" }) });
    v.focus();
  }

  const cache = new Map();
  async function selectFile(i, { keep = false, focus = false } = {}) {
    const f = st.data?.files?.[i];
    if (!f) return;
    st.sel = i;
    [...els.files.children].forEach((el, j) => el.classList.toggle("open", j === i));
    renderBar(f);
    const my2 = ++st.fileGen;
    const key = f.path + "\0" + (f.orig || "");
    let r = cache.get(key);
    if (!r) {
      if (!keep) {
        destroyViews();
        els.diff.append(h("div", { class: "cv-loading" }, h("span", { class: "spinner" })));
      }
      try {
        r = await api("commit_file", { hash: st.data.hash, path: f.path, orig: f.orig || undefined });
        cache.set(key, r);
      } catch (e) {
        if (my2 === st.fileGen) showMessage("warning", "Couldn't load this file", e.message || String(e));
        return;
      }
      if (my2 !== st.fileGen || gen !== my) return;
    }
    if (r.binary) return showMessage("binary", "Binary file", f.status === "A" ? "Added" : f.status === "D" ? "Deleted" : "Changed");
    const size = (r.before || "").length + (r.after || "").length;
    if (size > 6_000_000) return showMessage("warning", "Too large to show", `${formatSize(size)} of text`);
    if ((r.before || "") === (r.after || "") && f.status !== "A" && f.status !== "D")
      return showMessage("check", f.orig ? "Renamed, contents unchanged" : "No changes in the text", f.orig ? `From ${f.orig}` : "Only the file mode or type changed.");
    build(f, r.before || "", r.after || "");
    if (focus) ctl.view()?.focus();
  }

  function extensions(f) {
    const lang = languageFor(f.path);
    return [
      lineNumbers(),
      highlightSpecialChars(),
      drawSelection(),
      highlight,
      editorTheme,
      EditorState.readOnly.of(true),
      search({ top: true }),
      keymap.of([...searchKeymap, ...defaultKeymap]),
      controlGuard,
      dark.of(EditorView.darkTheme.of(!!currentTheme.dark)),
      lang.support(),
      WRAP.has(lang.key) ? EditorView.lineWrapping : [],
    ];
  }

  function build(f, before, after) {
    destroyViews();
    const fold = st.fold ? { margin: 3, minSize: 4 } : undefined;
    if (st.mode === "split") {
      const wrap = h("div", { class: "split-host" });
      els.diff.append(wrap);
      const parent = (st.data.parents || [])[0];
      wrap.append(
        h(
          "div",
          { class: "split-labels" },
          h("span", null, f.status === "A" ? "Not in parent" : parent ? `Parent ${parent.slice(0, 7)}` : "Before"),
          h("span", null, f.status === "D" ? "Deleted" : `This commit ${st.data.hash.slice(0, 7)}`),
        ),
      );
      st.split = new MergeView({
        a: { doc: before, extensions: extensions(f) },
        b: { doc: after, extensions: extensions(f) },
        parent: wrap,
        gutter: true,
        highlightChanges: true,
        collapseUnchanged: fold,
        diffConfig: DIFF_CONF,
      });
    } else {
      st.views.push(
        new EditorView({
          state: EditorState.create({
            doc: after,
            extensions: [
              extensions(f),
              unifiedMergeView({
                original: before,
                mergeControls: false,
                gutter: true,
                highlightChanges: true,
                syntaxHighlightDeletions: true,
                collapseUnchanged: fold,
                diffConfig: DIFF_CONF,
              }),
            ],
          }),
          parent: els.diff,
        }),
      );
    }
    const v = ctl.view();
    if (v && !st.fold) {
      const chunks = getChunks(v.state)?.chunks;
      if (chunks?.length) v.dispatch({ effects: EditorView.scrollIntoView(Math.min(chunks[0].fromB, v.state.doc.length), { y: "start", yMargin: 48 }) });
    }
  }

  const ctl = {
    destroy() {
      if (gen === my) gen++;
      destroyViews();
      if (shown === hash || shown === st.data?.hash) shown = null;
    },
    view: () => (st.split ? st.split.b : st.views[0]) || null,
    setDark(d) {
      for (const v of [...st.views, st.split?.a, st.split?.b]) v?.dispatch({ effects: dark.reconfigure(EditorView.darkTheme.of(!!d)) });
    },
    remeasure() {
      for (const v of [...st.views, st.split?.a, st.split?.b]) v?.requestMeasure();
    },
    focus() {
      (st.data?.files?.length ? els.files : els.root).focus?.();
    },
  };
  return ctl;
}
