# deck — what a proper terminal app needs

Status: `[x]` built and verified · `[~]` built, not yet checked by hand · `[ ]` not built.
Each item says how it was verified.

## Terminal

- [x] Colours: 16 / 256 / truecolor, bold, italic, underline, dim, inverse (screenshots: zsh prompt, OpenCode)
- [x] Wide characters (CJK, emoji) and fallback glyphs stay on the grid (engine test `a中b😀c`; glyphs placed by column)
- [x] Cursor shapes from DECSCUSR (engine tests); block cursor with the glyph redrawn under it (screenshot)
- [~] Trackpad and wheel scrolling: pixel deltas now accumulate into lines (unit test: 30 × 4px = 6 lines; the old code gave 0). A real gesture can't be posted to a background window, so this needs a hand check.
- [~] Mouse reporting to TUIs (click, drag, wheel; alt-screen wheel → arrows); the encoding is engine-tested, not yet tried in a live TUI
- [~] Selection: drag, double-click word, triple-click line, ⌘C; Shift bypasses mouse reporting
- [~] ⌘-click a URL opens it in a browser pane beside the terminal (URL detection tested, including after wide chars)
- [x] Keyboard: ctrl/alt/function keys, application cursor mode (unit tests)
- [~] Text input through the IME (dead keys, accents, emoji picker, non-Latin) via the platform input handler
- [~] ⌘V paste with bracketed paste
- [~] ⌘K clears screen and scrollback (engine test for the clear)
- [x] Font zoom ⌘+ / ⌘− / ⌘0, saved across launches (driven through the View menu: font 13 → 15, settings.json written, screenshot)
- [x] UTF-8 locale for the shell (new pane prints `lang=en_US.UTF-8`)
- [x] New terminal opens in the focused terminal's folder (File › New Terminal Tab after `cd /private/tmp` gave `cwd=/private/tmp`)
- [~] Titles from the shell; bell marks the tab
- [~] Shell exit closes its pane

## Workspace

- [x] Vertical tabs grouped Agents / Terminals / Browser; new, close, switch (⌘T, ⌘N, ⌘W, ⌘1–9, ⌃Tab)
- [~] Splits right / down, focus cycling (⌘[ ⌘])
- [ ] Drag a split divider to resize
- [ ] Restore tabs and splits after a relaunch
- [~] Find in scrollback: ⌘F bar, Enter / ⌘G up to older output, ⇧Enter / ⇧⌘G down, wraps, smart case, query taken literally; matches in view highlighted, the current one in white (engine test: order, wrap, case, `1+1`, focus follows output into history, viewport-only highlight). Not yet tried in the window.
- [x] Sidebar (⌘B) and inspector (⌘I); the inspector folds itself away when the window is too narrow (screenshot at 690 px)
- [x] Inspector: processes, project dir, ports, files, git, all native except `git status` (timings: tree 0.04 ms, ports 0.01 ms)
- [x] Black monochrome design, real SVG icons throughout (screenshots)
- [~] ⌘T opens a new tab page: Terminal (preselected, so ⌘T ⏎ is still a shell), Browser, and every agent deck knows with its mark, as a chat or in its own terminal (⌥⏎ or the card's terminal button); 1–9 pick, esc closes. ⌥⌘T is a plain terminal.
- [~] Splits draw a card per pane, a gutter apart; the focused card has the brighter edge and the others dim (a page dims itself, since gpui cannot paint over it)
- [~] Tab titles: a program's own title (Claude Code's task, `vim file`), else the command in front; a second line says which project (git root) and folder the pane is in, or which site a page is on; threads say "Claude · project"
- [~] Spinners: a comet for a busy terminal or a loading page, the agent's own mark turning while it works, and a light sweep across "Working…"
- [~] Right-click menus in deck's own look: terminal (links, copy, paste, select all, find, clear, splits, close), text fields, chat messages, sidebar rows
- [~] Terminal ⌘V pastes copied Finder files as paths and a copied image as a saved file's path, and files dropped on a terminal type their paths, so Claude Code and Codex attach them (path escaping unit-tested)
- [~] Settings tab (⌘,, Deck › Settings…, or the gear at the foot of the sidebar), one per window, applied live and saved to settings.json: zoom; terminal colour scheme (Deck, Tomorrow Night, Catppuccin Mocha, Tokyo Night, Gruvbox Dark, Solarized Dark, One Dark); font family (installed monospace faces only), size, line height; default cursor (block / bar / underline), blink, Option as Meta; search engine (Google by default); the agent and access new chats start with. An old settings.json still loads, and a bad value resets only itself (serde round-trip, legacy file, cursor default and Meta keys unit-tested). Not yet seen in the window.

## Browser

- [x] Toolbar: back, forward, reload / stop, address bar (URL or search), close (screenshot)
- [~] Back / forward dim when there is nowhere to go; two-finger swipe goes back and forward (WebKit's own gesture)
- [~] Real loading progress along the toolbar edge, from WebKit's estimate, polled only while a load runs; a failed load no longer leaves the bar spinning forever
- [~] A pane's first page shows 250 ms after it commits (was: up to 1.2 s, or until every subresource finished, on every address-bar navigation); later loads keep the old page up until the new one paints
- [~] Safari's user agent, so sites send their current code instead of fallback bundles
- [~] Search with Google by default (or the engine in Settings); `?text` forces a search; localhost, `*.localhost` and LAN IPs open over http (unit-tested)
- [~] Address bar: first click selects the whole address; inline completion from history (Safari-style host, then full address; ⇥ accepts, ⌫ drops it); suggestions from history, dev servers and the search engine, ↑↓ ⏎ esc; the page steps aside while suggestions show (ranking and completion unit-tested)
- [~] History kept in `browser-history.json`, frecency-ranked, search result pages left out (unit-tested)
- [~] Site icons fetched natively (ICO, PNG, SVG, WebP), kept per host under `favicons/`, shown in the address bar, the sidebar and the new tab page
- [~] New tab page: frequently visited sites as tiles, dev servers running in your terminals
- [~] Find in page ⌘F (⌘G / ⇧⌘G, ↑↓, esc) with a match count
- [~] Downloads go to ~/Downloads with a toolbar chip; click shows the file in Finder
- [~] ⌘-click or middle-click a link opens it in a new tab; pages report single-page-app navigation, and deck reads the real address from WebKit rather than trusting the page (message parsing unit-tested)
- [~] Links that open new windows (target=_blank, window.open) become tabs
- [~] Address bar looks focused only while editing (was verified by screenshot before the redesign)
- [~] Web Inspector: right-click › Inspect Element, or ⌥⌘I
- [~] Pages render in dark appearance, so sites that follow `prefers-color-scheme` match deck
- [~] Page zoom ⌘+ / ⌘− / ⌘0 when a page has focus
- [~] Rounded bottom corners that match the card

## Agents and control plane

- [x] Master chat with Claude / OpenCode / Codex / Gemini / Grok: streaming text, tool cards, thinking, interrupt, agent switch mid-thread, Edits only / Full access
- [x] Agents drive deck through the `deck` CLI (Claude ran `deck pane list` from a chat and answered correctly)
- [x] Chat threads in the sidebar with live status
- [x] `deck pane send` / `pane read` work on chats, so one agent can message another
- [~] Agent TUIs as tabs with working / idle / needs-you status
- [x] `deck` CLI: pane, tab, browser, open; automation token (used for all the checks above)
- [x] Control chat pinned first and opened at launch (⌘1)
- [x] T3-style thread view: centred column, work logs that fold to "Worked for 4s · Ran 2 commands" (screenshot), Copy on hover, diffs struck and highlighted
- [x] Composer card with agent picker, Auto-accept edits / Full access, tokens and cost, round send / stop (screenshot)
- [~] Messages sent mid-turn go straight to Claude Code, which queues them itself and reads them as soon as the current tool calls finish, inside the same turn; deck shows them as "Queued · after this step" and moves each into the thread where Claude read it (`--replay-user-messages`), so the rest of the turn reads as its answer. ⌘⏎ sends with `priority: "now"`. Esc stops the turn and the queue goes out next; a message Claude drops comes back to the composer. Agents that can't take input mid-turn keep deck's own queue (protocol checked against the real CLI: mid-turn delivery, and the queue surviving an interrupt; decoder unit-tested)
- [~] Inbox sidebar: threads newest first, two-line rows with Working timer / Needs you / Done / time ago
- [~] Attachments: ⌘V pastes screenshots and copied images (saved under `attachments/`, scaled to 1568 px) and files copied in Finder; drag files in; + opens a picker. Thumbnails and chips in the composer and in sent messages (click opens). Claude gets images inline as base64, other agents a path. File-URL decoding unit-tested; a thumbnail seen in a sent message (screenshot).
- [~] `@` file mentions: git's file list (or a bounded walk), ranked by name match; ↑↓ choose, ⏎/⇥ insert, esc closes (ranking unit-tested)
- [~] Composer editing: ⌘Z / ⇧⌘Z (typing and deleting undo in runs; ⌘Z after send brings the message back), ↑ / ↓ walk sent messages, ↑↓ follow soft-wrapped lines, double-click word, triple-click line, caret kept in view when the text scrolls, a click anywhere on the card focuses it; `deck pane send` no longer wipes the draft (word/line selection unit-tested)
- [~] Threads persist: transcript, draft, attachments, ↑ history and turn times saved per thread (`threads/<id>.json`, debounced, flushed on quit); relaunch reopens the control chat and other threads; ⌘W archives to `threads/archive/`. Claude resumes its own session with `--resume`, falling back to replaying the thread if that fails; a session that ended is picked up with the thread instead of starting blank (restore seen in a test instance's log; round-trip unit-tested)
- [x] Markdown: inline code in the mono face, links clickable (open in deck's browser) including bare URLs, code blocks scroll sideways instead of wrapping with a language label and Copy, task lists, multi-line quotes and list items, `snake_case` no longer italicised, bordered tables (unit tests + screenshot)
- [~] Work log: steps read as sentences ("Ran `cargo test`", "Edited src/chat.rs", "Searched fn main"), present tense while running; running steps and logs spin, with a live seconds counter, and steps over a second say how long they took; the header counts "Working for 12s"; logs and opened steps fade in; commands open as `$ command` plus output (failed ones outlined); the newest plan (TodoWrite) shows as a checklist; edits show +/− and a diff built from their input (wording, plans, changed files and durations unit-tested)
- [~] "N files changed" card after each turn: every edited file with its +/−, a click shows all its changes
- [~] Approvals and agent questions docked above the composer (T3): Claude runs with `--permission-prompt-tool stdio`, so a tool call that needs approval shows a card with the command or the diff and Claude Code's three answers (Yes · Yes, don't ask again in this chat · No), picked with ↑↓⏎, 1–3, a click, or esc; typing a reply refuses and tells Claude what to do instead. "Always" rules are kept to the session, never written to the project's settings. AskUserQuestion shows its options (multi-select too) and takes a typed answer; ExitPlanMode shows the plan. The step waits with "waiting for you", the thread shows Needs you, and the mode chip cycles Ask before edits → Auto-accept edits → Full access. (Format taken from the real CLI; round trip tested against a stand-in CLI; decoder unit-tested.) With claude-sudo armed, its hook approves everything first, so no card appears.
- [ ] Select and copy text inside a reply (gpui has no selectable text; Copy per message and per code block for now)
- [ ] Prune pasted images no thread refers to any more
- [x] Steer a running turn instead of queueing (T3): see above
- [ ] deck as an MCP server injected into every agent session (T3), alongside the CLI
- [ ] Per-turn git checkpoints with "Edit from here", worktree per thread, diff panel (T3)
- [ ] Command palette ⌘K (T3)

## Terminal fixes

- [x] Selection no longer stretches over the screen when a full-screen app scrolls its own region (engine test reproduces the stretch)

## Performance

- [~] The sidebar and inspector are their own cached views: gpui redraws every ancestor of a view that changed, so before this each frame of a streaming agent in a terminal rebuilt the whole sidebar and inspector too (emulator core measured separately: 143 MB/s, 0.28 ms to snapshot a full coloured 200×60 screen)

- [x] Idle CPU 0.1% (was 3.6%): no per-second process spawning, and the sidebar repaints only when a status changes
- [x] Output redraws on PTY wakeup, not on a poll
- [x] Memory: 103 MB for the app with a terminal and an agent chat (debug build); each open web page adds WebKit's own process (~20–90 MB)
