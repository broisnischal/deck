# Deck editor

The code editor Deck shows in a WKWebView pane: a file tree, tabs with
CodeMirror 6, inline and side-by-side diffs, a git panel (stage, commit,
branches, push/pull, stash, worktrees) and a commit graph with a commit view.
Plain JS and CSS, bundled by esbuild into three files. It makes no network
requests beyond its own `/api`.

```sh
npm install && npm run build   # writes dist/index.html, dist/app.js, dist/app.css
npm run watch                  # rebuild on change
```

`dist/` is committed, so building Deck does not need Node.

## How Deck hosts it

Deck serves `dist/` from a custom scheme and loads
`deckcode://localhost/index.html?...`. Everything else comes from a JSON API on
the same origin:

```js
fetch("/api/<op>", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(args) })
```

Ops without arguments still send `{}`. Replies are JSON. A non-2xx reply is
`{"error": "message"}`, and the page shows it in a toast. Paths are relative to
the project root and use `/`.

| op | args | reply |
| --- | --- | --- |
| `hello` | | `{root, name, git, theme, font: {family, size}, ui_font_size, open: path\|null, view: "files"\|"changes"\|"history", commit: hash\|null}` |
| `ls` | `{dir}` (`""` = root) | `{entries: [{name, dir}]}`, dirs first, sorted, `.git` hidden |
| `read` | `{path}` | `{text, size}` or `{binary: true, size}` or `{too_large: true, size}` |
| `write` | `{path, text}` | `{ok: true}` |
| `status` | | `{branch\|null, detached, upstream\|null, ahead, behind, files: [{path, orig\|null, staged, unstaged}]}` |
| `base` | `{path, against: "head"\|"index"}` | `{text, exists}` (`exists: false` for a new file) |
| `stage` / `unstage` / `discard` | `{paths}` | `{ok: true}` |
| `commit` | `{message, amend, push}` | `{ok: true, output}`; `push: true` pushes (or publishes) after committing |
| `last_message` | | `{message}` |
| `branches` | | `{current, branches: [{name, upstream\|null, current, remote, merged, ahead, behind, date, subject}]}` |
| `switch` | `{name, create}` | `{ok, output}` |
| `merge` | `{name}` | `{ok, output}`; fast-forward when possible, else a merge commit. Conflicts are an error reply, and `status` then shows `U` files |
| `delete_branch` | `{name, force}` | `{ok, output}` |
| `fetch`, `pull`, `push`, `stash`, `unstash` | | `{ok, output}` |
| `log` | `{limit, skip, all}` | `{commits: [{hash, short, parents: [hash], subject, author, email, date, refs: ["HEAD -> main", "origin/main", "tag: v1.0"]}]}`, topological order, newest first |
| `show` | `{hash}` | `{hash, short, parents, subject, body, author, email, date, refs, files: [{path, orig\|null, status: "M"\|"A"\|"D"\|"R"\|"C"\|"T", added\|null, removed\|null}]}` |
| `commit_file` | `{hash, path, orig?}` | `{before, after, binary}`; `before` is the first parent's version (`""` when added), `after` is `""` when deleted |
| `worktrees` | | `{worktrees: [{path, branch\|null, head, detached, locked, prunable, current}]}` |
| `worktree_add` | `{path, branch, create, base\|null}` | `{ok, output, path}`; relative paths are relative to the repository's top folder, `path` in the reply is absolute |
| `worktree_remove` | `{path, force}` | `{ok, output}` |

- `theme` is `{dark, bg, panel, raise, line, text, text2, dim, accent, red, green, yellow, blue, purple, cyan, selection}`, with colours as `#rrggbb`.
  Syntax colours never use `accent` (Deck's is often white or black): keywords are `purple`, strings `green`, numbers and
  constants `yellow`, types `cyan`, functions `blue`, tags `red`, attributes `yellow`, comments `dim`. Without `purple` or
  `cyan` the page mixes them from `blue`/`red` and `blue`/`green`. The graph lanes and avatars use the same eight hues.
- `font.size` is used as given; Deck zooms the whole page itself.
- Status letters: `staged` is one of `M A D R C U` or `""`. `unstaged` is one of `M D ? U` or `""`, where `?` means untracked.
  A path that ends in `/` (an untracked directory) is shown as a folder and is passed back as-is to `stage` and `discard`.
  Any `U` puts the file under **Merge conflicts**; staging it marks it resolved.
- `unstage` of a rename sends both the new path and `orig`.
- The page always asks for confirmation, naming the files, before it calls `discard`, `delete_branch`, `worktree_remove`
  or `merge`. `force: true` is only sent after a second confirmation, when the first reply said the branch is not fully
  merged or the worktree has changes or is locked.
- `push` with no upstream is shown as **Publish**. Deck sets the upstream.
- Checking out a remote branch from the branch menu calls `switch {name: "<branch without the remote>", create: false}`;
  git's guess mode creates the tracking branch. A branch that is checked out in another worktree opens that worktree instead.
- The History view loads 200 commits per `log` call and more when you scroll to the end (`skip` = commits loaded so far).
  Refs tell local branches from remote ones by the remote names in `branches`.

### Messages from the page to Deck

`window.ipc.postMessage(JSON.stringify({deck: "open", kind: "editor"|"terminal", path: "/abs/path"}))` asks Deck to open a
folder (a worktree) as an editor or a terminal. Without `window.ipc` (the mock) the page shows a toast instead.

### Calls from Deck into the page

`window.deck` exists as soon as the script runs. Calls made before `hello`
returns wait for it.

- `deck.open(path, {line?, diff?})` opens a file, optionally at a line, with the diff on or off.
- `deck.show("files" | "changes" | "history")`
- `deck.commit(hash)` switches to History and opens that commit's view. It loads more history (up to ten pages) to select
  it in the list; the view itself comes straight from `show`, so it opens even when the commit isn't found. `hello.commit`
  does the same once the page is ready.
- `deck.setTheme(theme)` takes the same shape as `hello.theme`.
- `deck.setFont({family, size})`. An optional `ui_font_size` also resizes the UI.
- `deck.refresh()` re-reads git status, the tree, the history and every open file that changed on disk. A file with
  unsaved edits gets a "changed on disk" banner instead of being reloaded.
- `deck.key("quickopen" | "find" | "save")` runs a shortcut Deck took before the page saw it.

The page also polls `status` every 3 s while visible, and refreshes on window focus. When the branch or its ahead/behind
counts change, the history and branch lists are re-read.

### Typed control characters

WebKit can deliver arrow keys as text (U+001C–U+001F, the old Mac arrow characters). Every CodeMirror view drops C0
control characters from typed text and turns a lone U+001C–U+001F into the cursor move it stands for; plain inputs and
the commit box strip them as they arrive. `?mockcheckinput=1` runs a self-check (it sets `data-input-check="pass"` on
`<html>` and shows a toast).

## Keyboard

| | |
| --- | --- |
| ⌘P | Quick Open (fuzzy, `file:line` jumps to a line) |
| ⌘S | Save |
| ⌘F | Find / replace in the file (in History: filter commits) |
| ⌘⇧E / ⌘⇧G / ⌘⇧H | Files / Changes / History |
| ⌘⇧D | Toggle the diff for the current file |
| ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌃W | Close tab (⌘W is left to Deck) |
| ⌘↵ / ⌘⇧↵ | Commit / commit and push, from the message box |
| ↑ ↓ ← → | Move through Changes rows; fold sections and folders |
| Space · ↵ · ⌫ | Stage or unstage · open the diff · discard (asks first) |
| ↑ ↓ · ↵ | Move through History (opens each commit) · focus the commit |

Deck has to let these key equivalents reach the web view while the pane is
focused.

## Developing without Deck

Any origin other than `deckcode:` uses an in-memory mock repo (`src/mock.js`),
so `python3 -m http.server -d dist 8000` is enough to work on the UI. The mock has a
branchy history (about 220 commits with merges, tags, remote and merged branches),
three worktrees, staged/unstaged/untracked changes and a rename. URL switches put
the UI into a given state:

- views and files: `mockview=files|changes|history`, `mockopen=src/main.rs`, `mockdiff=1`, `mocksplit=1`,
  `mockfold=1`, `mockagainst=index`, `mocktabs=a,b`, `mockactivate=path`, `mockexpand=dir,dir`, `mockfilter=rs`,
  `mockquick=query`, `mockline=n`, `mockfind=1`, `mockdirty=1&mockdisk=1` (the changed-on-disk banner)
- look: `mocklight=1`, `mocktheme=black|graphite|midnight|light`, `mockaccent=7aa2f7`, `mockuisize=14`
  (or `mockui=14`), `mockfont=14`, `mocksidebar=300`
- repo state: `mocknogit=1`, `mockempty=1` (no commits yet), `mockdetached=1`, `mockconflict=1` (a merge stopped
  on conflicts), `mockbehind=n` (origin/main ahead by n), `mockprunable=1`, `mockmergeconflict=1` (any merge conflicts)
- history: `mockcommit=n` (n-th newest commit) or `mockcommit=<hash prefix>`, `mockcommitfile=i`,
  `mockhello=<hash prefix>` (as if `hello.commit` were set)
- popovers and dialogs: `mockmenu=branch|more|commit|worktrees|newworktree`, `mockmsg=text` (`\n` for new lines),
  `mockconfirm=discard`, `mocktoast=1`, `mockcheckinput=1`

## Layout

```
src/main.js        boot, layout, keyboard, polling, window.deck
src/editor.js      tabs (files and the commit view), CodeMirror views, save, disk sync, diffs, empty state
src/tree.js        lazy file tree and filter
src/branchbar.js   branch button, sync button and ⋯ menu above the git views
src/changes.js     commit box, Merge conflicts / Staged / Changes / Untracked as list or tree, keyboard
src/branches.js    branch menu, switch / create / merge / delete
src/worktrees.js   worktree list, Worktrees panel, New Worktree dialog, open in Deck
src/history.js     History view: virtualised commit list, filter, All / Current
src/graph.js       commit graph lanes and their SVG rows
src/refs.js        ref pills and author avatars
src/commitview.js  the commit tab: header, changed files, read-only diff
src/inputguard.js  drops typed control characters (WebKit arrow-key text)
src/quickopen.js   ⌘P index and fuzzy matcher
src/git.js         status polling, git op wrapper
src/theme.js       theme → CSS variables, CodeMirror theme and highlighting
src/languages.js   language by file name
src/ui.js          toasts, dialogs, popovers, menus, segmented controls
src/api.js         transport (real or mock)
src/mock.js        the in-memory repo used outside Deck
```
