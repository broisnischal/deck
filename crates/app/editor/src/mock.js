// In-memory stand-in for Deck's /api, used when the page is not served from
// deckcode:// (e.g. a plain static server during development). It models a
// tiny git repo with HEAD, index and working tree so every op does something
// believable.

const MAIN_RS_HEAD = `use std::env;
use std::process::ExitCode;

mod config;
mod ui;

/// Entry point: parse the arguments, load the config, run the app.
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "-v");

    let config = match config::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    if verbose {
        println!("loaded {} panes", config.panes.len());
    }

    ui::run(config);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_flags() {
        assert_eq!(2 + 2, 4);
    }
}
`;

const MAIN_RS_WORK = `use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

mod config;
mod ui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Entry point: parse the arguments, load the config, run the app.
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");

    if args.iter().any(|a| a == "--version") {
        println!("deck {VERSION}");
        return ExitCode::SUCCESS;
    }

    let root = args.last().map(PathBuf::from).unwrap_or_else(|| ".".into());
    let config = match config::load(&root) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("deck: {err}");
            return ExitCode::from(2);
        }
    };

    if verbose {
        println!("loaded {} panes from {}", config.panes.len(), root.display());
    }

    ui::run(config);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_flags() {
        assert_eq!(2 + 2, 4);
    }
}
`;

const CONFIG_RS = `use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub panes: Vec<Pane>,
    pub font_size: f32,
}

#[derive(Debug, Clone)]
pub struct Pane {
    pub title: String,
    pub cwd: String,
}

pub fn load(root: &Path) -> Result<Config, String> {
    let path = root.join("deck.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

fn parse(text: &str) -> Result<Config, String> {
    let mut config = Config { font_size: 13.0, ..Default::default() };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Some(title) = line.strip_prefix("pane = ") {
            config.panes.push(Pane { title: title.trim_matches('"').into(), cwd: ".".into() });
        }
    }
    Ok(config)
}
`;

const APP_TS_HEAD = `import { createStore, type Store } from "./store";
import { render } from "./render";

export interface Session {
  id: string;
  title: string;
  createdAt: number;
}

const store: Store<Session[]> = createStore([]);

export function addSession(title: string): Session {
  const session = { id: crypto.randomUUID(), title, createdAt: Date.now() };
  store.update((all) => [...all, session]);
  return session;
}

export function start(root: HTMLElement) {
  store.subscribe((sessions) => render(root, sessions));
}
`;

const APP_TS_INDEX = `import { createStore, type Store } from "./store";
import { render } from "./render";

export interface Session {
  id: string;
  title: string;
  createdAt: number;
  pinned?: boolean;
}

const store: Store<Session[]> = createStore([]);

export function addSession(title: string, pinned = false): Session {
  const session = { id: crypto.randomUUID(), title, createdAt: Date.now(), pinned };
  store.update((all) => [...all, session]);
  return session;
}

export function start(root: HTMLElement) {
  store.subscribe((sessions) => render(root, sessions));
}
`;

const APP_TS_WORK = `import { createStore, type Store } from "./store";
import { render } from "./render";

export interface Session {
  id: string;
  title: string;
  createdAt: number;
  pinned?: boolean;
}

const store: Store<Session[]> = createStore([]);

export function addSession(title: string, pinned = false): Session {
  const session = { id: crypto.randomUUID(), title, createdAt: Date.now(), pinned };
  store.update((all) => [...all, session].sort(byPinned));
  return session;
}

function byPinned(a: Session, b: Session): number {
  return Number(b.pinned ?? false) - Number(a.pinned ?? false) || a.createdAt - b.createdAt;
}

export function start(root: HTMLElement) {
  store.subscribe((sessions) => render(root, sessions));
}
`;

const BUTTON_TSX = `import { useState } from "react";

type Props = {
  label: string;
  onPress?: () => void;
  tone?: "default" | "danger";
};

export function Button({ label, onPress, tone = "default" }: Props) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      className={\`btn btn-\${tone}\`}
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await onPress?.();
        setBusy(false);
      }}
    >
      {label}
    </button>
  );
}
`;

const README = `# deck

A calm terminal and agent workspace for macOS, written in Rust with gpui.

## Building

\`\`\`sh
cargo build --release
./target/release/deck ~/code/project
\`\`\`

## Layout

- \`src/main.rs\` — argument parsing and startup
- \`src/ui/\` — panes, tabs and the theme
- \`web/\` — the embedded editor

> Everything runs locally. No accounts, no telemetry.
`;

const FILES = {
  ".gitignore": "/target\nnode_modules/\n.DS_Store\n*.log\n",
  "Cargo.toml": `[package]
name = "deck"
version = "0.4.2"
edition = "2021"

[dependencies]
gpui = { git = "https://github.com/zed-industries/zed" }
serde = { version = "1", features = ["derive"] }
anyhow = "1"

[profile.release]
lto = true
codegen-units = 1
`,
  "Cargo.lock": "# This file is automatically @generated by Cargo.\nversion = 3\n\n[[package]]\nname = \"anyhow\"\nversion = \"1.0.86\"\n",
  "README.md": README,
  "Dockerfile": "FROM rust:1.80 AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n\nFROM debian:bookworm-slim\nCOPY --from=build /src/target/release/deck /usr/local/bin/deck\nENTRYPOINT [\"deck\"]\n",
  "src/main.rs": MAIN_RS_WORK,
  "src/config.rs": CONFIG_RS,
  "src/ui/mod.rs": "pub mod theme;\n\nuse crate::config::Config;\n\npub fn run(config: Config) {\n    let theme = theme::Theme::dark();\n    println!(\"{} panes, accent {}\", config.panes.len(), theme.accent);\n}\n",
  "src/ui/theme.rs": "#[derive(Debug, Clone, Copy)]\npub struct Theme {\n    pub bg: u32,\n    pub text: u32,\n    pub accent: u32,\n}\n\nimpl Theme {\n    pub const fn dark() -> Self {\n        Self { bg: 0x0a0a0a, text: 0xf5f5f5, accent: 0xffffff }\n    }\n}\n",
  "web/app.ts": APP_TS_WORK,
  "web/store.ts": "export type Store<T> = {\n  get(): T;\n  update(fn: (value: T) => T): void;\n  subscribe(fn: (value: T) => void): () => void;\n};\n\nexport function createStore<T>(initial: T): Store<T> {\n  let value = initial;\n  const subs = new Set<(value: T) => void>();\n  return {\n    get: () => value,\n    update(fn) {\n      value = fn(value);\n      subs.forEach((s) => s(value));\n    },\n    subscribe(fn) {\n      subs.add(fn);\n      return () => subs.delete(fn);\n    },\n  };\n}\n",
  "web/components/Button.tsx": BUTTON_TSX,
  "web/index.html": "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <title>Deck</title>\n    <link rel=\"stylesheet\" href=\"styles.css\" />\n  </head>\n  <body>\n    <main id=\"root\"></main>\n    <script type=\"module\" src=\"app.ts\"></script>\n  </body>\n</html>\n",
  "web/styles.css": ":root {\n  --bg: #0a0a0a;\n  --text: #f5f5f5;\n}\n\nbody {\n  margin: 0;\n  background: var(--bg);\n  color: var(--text);\n  font: 13px/1.4 -apple-system, sans-serif;\n}\n",
  "scripts/release.sh": "#!/usr/bin/env bash\nset -euo pipefail\n\nversion=\"$(grep '^version' Cargo.toml | head -1 | cut -d'\"' -f2)\"\necho \"releasing v$version\"\ncargo build --release\ngit tag \"v$version\"\n",
  "scripts/stats.py": "import sys\nfrom pathlib import Path\n\n\ndef count_lines(root: Path) -> int:\n    \"\"\"Count lines of Rust code under root.\"\"\"\n    return sum(len(p.read_text().splitlines()) for p in root.rglob(\"*.rs\"))\n\n\nif __name__ == \"__main__\":\n    print(count_lines(Path(sys.argv[1] if len(sys.argv) > 1 else \".\")))\n",
  "notes/todo.md": "# Todo\n\n- [ ] Split view for diffs\n- [ ] Remember sidebar width\n- [x] Quick open\n",
  "assets/logo.png": "\u0089PNG",
  "data/fixtures.json": "{}\n",
  "node_modules/left-pad/index.js": "module.exports = (s, n) => String(s).padStart(n);\n",
};

// Deeper paths, so the panel has long folders to truncate.
const SESSION_RS_HEAD = `use serde::{Deserialize, Serialize};

/// What Deck writes to disk so a window comes back as it was.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub panes: Vec<PaneState>,
    pub active: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneState {
    pub cwd: String,
    pub title: Option<String>,
}

impl Session {
    pub fn empty() -> Self {
        Self { panes: Vec::new(), active: 0 }
    }
}
`;
const SESSION_RS_WORK = SESSION_RS_HEAD.replace(
  "    pub title: Option<String>,\n}",
  "    pub title: Option<String>,\n    /// The editor pane's address, when the pane is the editor.\n    pub url: Option<String>,\n}",
);
const TAP_RS_HEAD = `//! Reads what the shell prints without blocking the renderer.

use std::io::Read;

pub struct Tap<R> {
    inner: R,
    buf: Vec<u8>,
}

impl<R: Read> Tap<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, buf: vec![0; 64 * 1024] }
    }

    pub fn pump(&mut self) -> std::io::Result<&[u8]> {
        let n = self.inner.read(&mut self.buf)?;
        Ok(&self.buf[..n])
    }
}
`;
const TAP_RS_WORK = TAP_RS_HEAD.replace("vec![0; 64 * 1024]", "vec![0; 256 * 1024]").replace(
  "        Ok(&self.buf[..n])",
  "        if n == 0 {\n            return Err(std::io::ErrorKind::UnexpectedEof.into());\n        }\n        Ok(&self.buf[..n])",
);

Object.assign(FILES, {
  "crates/app/src/session.rs": SESSION_RS_WORK,
  "crates/term/src/tap.rs": TAP_RS_WORK,
  "docs/architecture.md": "# Architecture\n\nDeck is a gpui app. Each pane is a terminal, a browser or the editor.\n",
  "web/tsconfig.json": `{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "strict": true,
    "jsx": "react-jsx",
    "noEmit": true,
    "lib": ["DOM", "ES2022"],
    "paths": { "@/*": ["./*"] }
  },
  "include": ["**/*.ts", "**/*.tsx"],
  "exclude": ["node_modules", null]
}
`,
  // Untracked: a new fonts folder and the editor itself, file by file.
  "crates/app/assets/fonts/JetBrainsMono-Bold.ttf": "\u0000ttf",
  "crates/app/assets/fonts/JetBrainsMono-BoldItalic.ttf": "\u0000ttf",
  "crates/app/assets/fonts/JetBrainsMono-Italic.ttf": "\u0000ttf",
  "crates/app/assets/fonts/JetBrainsMono-Regular.ttf": "\u0000ttf",
  "crates/app/assets/fonts/OFL.txt": "Copyright 2020 The JetBrains Mono Project Authors\n\nThis Font Software is licensed under the SIL Open Font License, Version 1.1.\n",
  "crates/app/editor/README.md": "# Deck editor\n\nThe code editor Deck shows in a WKWebView pane.\n",
  "crates/app/editor/build.mjs": "import * as esbuild from \"esbuild\";\n\nawait esbuild.build({ entryPoints: [\"src/main.js\"], bundle: true, outdir: \"dist\" });\n",
  "crates/app/editor/package.json": "{\n  \"name\": \"deck-editor\",\n  \"private\": true,\n  \"type\": \"module\"\n}\n",
  "crates/app/editor/src/changes.js": "export function mountChanges(root) {}\n",
  "crates/app/editor/src/history.js": "export function mountHistory(root) {}\n",
  "crates/app/editor/src/main.js": "import \"./styles.css\";\n",
  "crates/app/editor/src/styles.css": ":root {\n  --row: 24px;\n}\n",
});

const UNTRACKED = new Set(Object.keys(FILES).filter((p) => p.startsWith("crates/app/assets/fonts/") || p.startsWith("crates/app/editor/")));
UNTRACKED.add("notes/todo.md");

const HEAD = {
  ...FILES,
  "src/main.rs": MAIN_RS_HEAD,
  "web/app.ts": APP_TS_HEAD,
  "src/legacy.rs": "// Old entry point, kept for reference.\nfn legacy_main() {}\n",
  "crates/app/src/session.rs": SESSION_RS_HEAD,
  "crates/term/src/tap.rs": TAP_RS_HEAD,
  "notes/architecture.md": FILES["docs/architecture.md"],
};
for (const p of UNTRACKED) delete HEAD[p];
delete HEAD["src/config.rs"];
delete HEAD["docs/architecture.md"];

const INDEX = { ...HEAD, "web/app.ts": APP_TS_INDEX, "src/config.rs": CONFIG_RS, "docs/architecture.md": FILES["docs/architecture.md"] };
delete INDEX["notes/architecture.md"];

const SIZES = { "assets/logo.png": 24580, "data/fixtures.json": 14_680_064 };
const BINARY = /\.(png|jpe?g|gif|ico|icns|ttf|otf|woff2?)$/i;

// ------------------------------------------------------------------ history

const NOW = Math.floor(Date.now() / 1000);

function seeded(seed) {
  let s = seed >>> 0 || 1;
  return () => (s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 4294967296;
}
const pick = (rand, list) => list[Math.floor(rand() * list.length)];

function hex40(n) {
  let out = "";
  let x = Math.imul(n + 1, 2654435761) >>> 0;
  for (let i = 0; i < 5; i++) {
    x = Math.imul(x ^ (x >>> 15), 2246822519) >>> 0;
    x = (x + 0x9e3779b9 + Math.imul(i + 1, n + 7919)) >>> 0;
    x = Math.imul(x ^ (x >>> 13), 3266489917) >>> 0;
    out += x.toString(16).padStart(8, "0");
  }
  return out;
}

const AUTHORS = [
  ["Nischal Dahal", "nischal@stroke.click"],
  ["Nischal Dahal", "nischal@stroke.click"],
  ["Nischal Dahal", "nischal@stroke.click"],
  ["Maya Chen", "maya@stroke.click"],
  ["Jonas Weber", "jonas@hey.com"],
  ["Priya Raman", "priya.raman@fastmail.com"],
  ["Sam Okafor", "sam@okafor.dev"],
];

const SUBJECTS = [
  "Draw the cursor after the selection so it stays visible",
  "Keep the scroll position when a pane is resized",
  "Read the shell's title escape and show it in the tab",
  "Cache glyph runs per line instead of per frame",
  "Fix the IME candidate window position on Sonoma",
  "Add a setting for the cursor blink rate",
  "Debounce resize events while the window is being dragged",
  "Use the system accent colour for focus rings",
  "Show a badge on panes that finished a long command",
  "Move theme loading off the main thread",
  "Handle OSC 8 hyperlinks",
  "Restore the working directory of each pane on launch",
  "Paste multi-line text with bracketed paste",
  "Make ⌘K clear the scrollback, not just the screen",
  "Bump gpui to the latest main",
  "Fix a crash when the last pane is closed during a resize",
  "Respect TERM_PROGRAM in child processes",
  "Tidy up the settings window layout",
  "Add tests for the escape parser",
  "Render underline styles: curly, dotted and dashed",
  "Stop leaking file descriptors when a shell exits",
  "Group notifications by pane",
  "Use a fixed tab width for the pane strip",
  "Keep the scroll position when a pane is resized while the terminal is printing output faster than it renders",
  "Support the kitty keyboard protocol's disambiguate flag",
  "Show the git branch in the pane header",
  "Fix the light theme's selection colour",
  "Load the font once and share it between windows",
  "Explain the release process in the README",
  "Sign and notarise the release build",
  "Add a --version flag",
  "Rename Workspace to Deck in the code",
  "Hide the scrollbar until the pane is scrolled",
  "Quit cleanly when the last window closes",
  "Clamp the font size between 9 and 32 points",
  "Faster startup: lazy-load the command palette index",
  "Make the agent alert sound optional",
  "Fix wide characters at the right edge of the screen",
  "Draw box-drawing characters ourselves so they connect",
  "Remember the sidebar width per window",
  "Let ⌥-click move the cursor in the shell prompt",
  "Fix focus being stolen by the inspector",
  "Prefer the login shell from the passwd entry",
  "Add a Midnight theme",
  "Send SIGWINCH only when the size actually changed",
  "Polish the empty state of the snippets list",
  "Fix the 1px gap under the tab strip on non-retina screens",
  "Trim trailing spaces when copying",
  "Document the deck.toml options",
  "Retry the update check with backoff",
];

const BODIES = [
  "The old code assumed one glyph per cell, which broke on emoji and CJK.\nThis measures the run once and reuses it until the line changes.",
  "Closes #214.",
  "We used to recompute this on every frame. Profiling showed 18% of frame\ntime spent here on a 4K display.\n\nCo-authored-by: Maya Chen <maya@stroke.click>",
  "Without this, a resize in the middle of output could leave the cursor\none row too low.",
  "Reported by several people on the Discord. Reproduced with a 2 GB log\npiped through `less -R`.",
  "See the discussion in #188 for why the default stays on.",
];

const FEATURES = [
  "feature/tabs", "feature/command-palette", "fix/scrollback", "feature/themes", "feature/session-restore",
  "fix/ime-input", "feature/agent-alerts", "feature/snippets", "perf/renderer", "feature/ssh",
  "fix/crash-on-close", "feature/updates", "feature/touch-id", "feature/inspector", "chore/deps",
  "feature/editor", "fix/font-fallback", "feature/sub-agents", "docs/readme", "feature/prompt-rewrite",
  "feature/shortcuts", "fix/paste-bracketed", "feature/website", "feature/trial",
];

const G_at = (commits, i) => commits[Math.min(i, commits.length - 1)].hash;

function buildHistory(behind) {
  const rand = seeded(42);
  const commits = [];
  const byHash = new Map();
  let t = NOW - 86400 * 160;
  let n = 0;
  let subj = 0;
  const add = (parents, subject, opts = {}) => {
    t += Math.round(opts.gap ?? 2400 + rand() * 40000);
    const [author, email] = opts.author || pick(rand, AUTHORS);
    const body = opts.body ?? (rand() < 0.22 ? pick(rand, BODIES) : "");
    const c = { hash: hex40(n++), parents: parents.filter(Boolean), subject: subject ?? SUBJECTS[subj++ % SUBJECTS.length], body, author, email, date: t };
    commits.push(c);
    byHash.set(c.hash, c);
    return c.hash;
  };
  const tags = new Map();
  const local = new Map();
  const remote = new Map();
  const upstream = new Map();

  let main = add([], "Initial commit", { author: AUTHORS[0], body: "" });
  main = add([main], "Add a gpui window with a single terminal pane", { author: AUTHORS[0] });
  main = add([main], "Spawn the login shell in a pty", { author: AUTHORS[0] });
  let pr = 41;
  for (let e = 0; e < 22; e++) {
    const name = FEATURES[e];
    for (let i = 0, k = 1 + Math.floor(rand() * 3); i < k; i++) main = add([main]);
    let br = main;
    let side = null;
    if (rand() < 0.3) side = main; // a second branch in flight at the same time
    const k = 2 + Math.floor(rand() * 4);
    for (let i = 0; i < k; i++) {
      br = add([br]);
      if (rand() < 0.35) main = add([main]);
      if (side && i === 1) side = add([side], null, { author: AUTHORS[4] });
    }
    if (side) {
      side = add([side], null, { author: AUTHORS[4] });
      main = add([main, side], `Merge pull request #${pr++} from stroke/fix/${["hotkeys", "flicker", "locale", "wrap"][e % 4]}`, { body: "", gap: 1800 });
    }
    main = add([main, br], `Merge pull request #${pr++} from stroke/${name}`, { body: pick(rand, SUBJECTS), gap: 3600 });
    if (e === 4) tags.set("v0.1.0", main);
    if (e === 10) tags.set("v0.2.0", main);
    if (e === 16) tags.set("v0.3.0", main);
  }
  main = add([main], "Release 0.4.0");
  tags.set("v0.4.0", main);
  const v040 = main;

  // A release branch with a backport, only on the remote.
  let rel = add([v040], "Fix a crash when closing the last pane", { author: AUTHORS[3] });
  main = add([main], "Start the 0.5 cycle");
  rel = add([rel], "Release 0.4.1", { author: AUTHORS[3] });
  tags.set("v0.4.1", rel);
  remote.set("origin/release/0.4", rel);

  // Merged, and the branch is still around.
  let wt = add([main], "Model git worktrees in the backend");
  main = add([main], "Bump serde to 1.0.210", { author: ["dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com"], body: "Bumps serde from 1.0.209 to 1.0.210." });
  wt = add([wt], "List worktrees in the branch menu");
  wt = add([wt], "Open a worktree in a new Deck window");
  main = add([main, wt], "Merge branch 'feature/worktrees'", { body: "" });
  local.set("feature/worktrees", wt);
  remote.set("origin/feature/worktrees", wt);
  upstream.set("feature/worktrees", "origin/feature/worktrees");

  // An open feature branch, one commit ahead of its remote.
  let sd = add([main], "Side-by-side diffs with a shared scroll");
  main = add([main], "Show the commit count in the sync button");
  sd = add([sd], "Collapse unchanged lines in split view");
  main = add([main], "Fix the branch picker's keyboard focus");
  sd = add([sd], "Label the two sides of a split diff");
  remote.set("origin/feature/split-diff", sd);
  sd = add([sd], "Revert a hunk from the split view");
  local.set("feature/split-diff", sd);
  upstream.set("feature/split-diff", "origin/feature/split-diff");

  // A dependabot branch that never got merged.
  remote.set(
    "origin/dependabot/cargo/anyhow-1.0.89",
    add([main], "Bump anyhow from 1.0.86 to 1.0.89", { author: ["dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com"], body: "Bumps anyhow from 1.0.86 to 1.0.89." }),
  );

  main = add([main], "Release 0.4.2");
  tags.set("v0.4.2", main);
  // A local-only fix branch.
  let crlf = add([main], "Normalise CRLF line endings on save", { author: AUTHORS[0] });
  main = add([main], "Track the editor pane in the session");
  crlf = add([crlf], "Keep CRLF when the file already uses it", { author: AUTHORS[0] });
  local.set("fix/crlf", crlf);
  // Older local branches, so the branch menu has a Recent section.
  local.set("spike/gpu-text", G_at(commits, 60));
  local.set("chore/ci-cache", G_at(commits, 120));
  local.set("feature/inline-blame", G_at(commits, 170));
  const pushed = add([main], "Undo a turn from the chat sidebar", { author: AUTHORS[0], gap: 5000 });
  let originMain = pushed;
  for (let i = 0; i < behind; i++) originMain = add([originMain], `${pick(rand, SUBJECTS)}`, { author: AUTHORS[3], gap: 1200 });
  main = add([pushed], "Show the loaded pane count in verbose mode", { author: AUTHORS[0], gap: 2000, body: "" });
  // Nudge the newest commits close to "now".
  const shift = NOW - 3 * 3600 - t;
  for (const c of commits) c.date += shift;
  local.set("main", main);
  remote.set("origin/main", originMain);
  upstream.set("main", "origin/main");
  upstream.set("fix/crlf", null);
  return { commits, byHash, local, remote, upstream, tags, v040, n };
}

// Files a commit touched, made up but stable for a given commit.
const PATHS = [
  "src/main.rs", "src/config.rs", "src/ui/mod.rs", "src/ui/theme.rs", "web/app.ts", "web/store.ts",
  "web/components/Button.tsx", "web/styles.css", "web/index.html", "README.md", "Cargo.toml", "Cargo.lock",
  "scripts/release.sh", "scripts/stats.py", "crates/app/src/session.rs", "crates/term/src/tap.rs",
  "crates/term/src/lib.rs", "crates/app/src/workspace.rs", "docs/architecture.md", "assets/logo.png",
];

const EXTRA_LINES = {
  rs: [
    "    let width = area.width.saturating_sub(2);",
    "    tracing::debug!(cols, rows, \"resize\");",
    "    if self.dirty.is_empty() {\n        return;\n    }",
    "    // Keep the cursor visible after a resize.",
    "    let theme = cx.global::<Theme>();",
    "    self.scroll = self.scroll.min(self.lines.len());",
    "#[inline]\nfn cell_width(c: char) -> usize {\n    unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)\n}",
  ],
  ts: [
    "  const width = Math.max(0, root.clientWidth - 2);",
    "  if (!sessions.length) return;",
    "  // Keep the newest session at the top.",
    "export const VERSION = \"0.4.2\";",
    "  store.update((all) => all.filter((s) => s.id !== id));",
  ],
  md: ["", "## Releases", "Run `scripts/release.sh` from a clean checkout.", "- Works offline", "> The website lives in `site/`."],
  toml: ["tracing = \"0.1\"", "unicode-width = \"0.2\"", "[features]\ndefault = []"],
  css: ["  --radius: 6px;", "  scrollbar-width: thin;", ".pane:focus-visible {\n  outline: 2px solid var(--accent);\n}"],
  sh: ["git push --tags", "echo \"done\"", "test -z \"$(git status --porcelain)\" || { echo \"dirty tree\"; exit 1; }"],
  txt: ["", "Updated."],
};
const extOf = (p) => {
  const e = p.slice(p.lastIndexOf(".") + 1).toLowerCase();
  if (e === "rs") return "rs";
  if (e === "ts" || e === "tsx" || e === "js") return "ts";
  if (e === "md") return "md";
  if (e === "toml" || e === "lock") return "toml";
  if (e === "css") return "css";
  if (e === "sh") return "sh";
  return "txt";
};

function synthText(path) {
  if (HEAD[path] !== undefined) return HEAD[path];
  if (FILES[path] !== undefined) return FILES[path];
  const ext = extOf(path);
  const mod = path.slice(path.lastIndexOf("/") + 1).replace(/\..*$/, "");
  if (ext === "rs")
    return `//! ${mod}\n\nuse std::collections::HashMap;\n\npub struct ${mod[0].toUpperCase() + mod.slice(1)} {\n    items: HashMap<String, usize>,\n}\n\nimpl ${mod[0].toUpperCase() + mod.slice(1)} {\n    pub fn new() -> Self {\n        Self { items: HashMap::new() }\n    }\n\n    pub fn len(&self) -> usize {\n        self.items.len()\n    }\n}\n`;
  return `${mod}\n`;
}

function mutate(text, rand, ext) {
  const lines = text.split("\n");
  const ops = 1 + Math.floor(rand() * 3);
  for (let i = 0; i < ops; i++) {
    const at = Math.min(lines.length - 1, 1 + Math.floor(rand() * Math.max(1, lines.length - 2)));
    const r = rand();
    if (r < 0.45) lines.splice(at, 0, ...pick(rand, EXTRA_LINES[ext]).split("\n"));
    else if (r < 0.7 && lines.length > 6) lines.splice(at, 1 + Math.floor(rand() * 2));
    else if (lines[at] && lines[at].trim()) lines[at] = lines[at].replace(/\s*$/, "") + (ext === "md" || ext === "txt" ? " (updated)" : ext === "sh" ? " # tidy" : " // tidy");
    else lines.splice(at, 0, ...pick(rand, EXTRA_LINES[ext]).split("\n"));
  }
  return lines.join("\n");
}

function diffCounts(a, b) {
  const A = a ? a.split("\n") : [], B = b ? b.split("\n") : [];
  let s = 0;
  while (s < A.length && s < B.length && A[s] === B[s]) s++;
  let e = 0;
  while (e < A.length - s && e < B.length - s && A[A.length - 1 - e] === B[B.length - 1 - e]) e++;
  const x = A.slice(s, A.length - e), y = B.slice(s, B.length - e);
  const m = x.length, n = y.length;
  if (!m || !n || m * n > 400000) return { added: n, removed: m };
  let prev = new Uint16Array(n + 1), cur = new Uint16Array(n + 1);
  for (let i = 0; i < m; i++) {
    for (let j = 0; j < n; j++) cur[j + 1] = x[i] === y[j] ? prev[j] + 1 : Math.max(prev[j + 1], cur[j]);
    [prev, cur] = [cur, prev];
  }
  const l = prev[n];
  return { added: n - l, removed: m - l };
}

export function createMock(params) {
  const ROOT = "/Users/you/code/deck";
  const fresh = params.has("mockempty");
  const work = new Map(Object.entries(FILES));
  let head = fresh ? new Map() : new Map(Object.entries(HEAD));
  let index = fresh ? new Map() : new Map(Object.entries(INDEX));
  const G = fresh
    ? { commits: [], byHash: new Map(), local: new Map(), remote: new Map(), upstream: new Map([["main", null]]), tags: new Map(), n: 0 }
    : buildHistory(+params.get("mockbehind") || 0);
  let branch = "main";
  let detachedAt = params.has("mockdetached") && !fresh ? G.v040 : null;
  if (detachedAt) branch = null;
  let stashes = [];
  let conflicts = new Set();
  let mergeHead = null;
  if (params.has("mockconflict") && !fresh) {
    conflicts = new Set(["src/config.rs", "web/styles.css"]);
    mergeHead = G.local.get("fix/crlf");
    work.set(
      "src/config.rs",
      CONFIG_RS.replace(
        "pub fn load(root: &Path) -> Result<Config, String> {",
        "<<<<<<< HEAD\npub fn load(root: &Path) -> Result<Config, String> {\n=======\npub fn load(root: &Path, crlf: bool) -> Result<Config, String> {\n>>>>>>> fix/crlf",
      ),
    );
  }
  let worktrees = fresh
    ? [{ path: ROOT, branch: "main", head: "", detached: false, locked: false, prunable: false, current: true }]
    : [
        { path: ROOT, branch: "main", detached: false, locked: false, prunable: false, current: true },
        { path: "/Users/you/code/deck-split-diff", branch: "feature/split-diff", detached: false, locked: false, prunable: false, current: false, dirty: true },
        { path: "/Users/you/code/deck-review-0.4", branch: null, head: G.v040, detached: true, locked: true, prunable: false, current: false },
      ];
  if (params.has("mockprunable")) worktrees.push({ path: "/private/tmp/deck-old-experiment", branch: "fix/old-experiment", detached: false, locked: false, prunable: true, current: false });

  const THEMES = {
    black: { dark: true, bg: "#0a0a0a", panel: "#000000", raise: "#111111", line: "#1a1a1a", text: "#f5f5f5", text2: "#a3a3a3", dim: "#737373", accent: "#ffffff", selection: "#3a3a3a" },
    graphite: { dark: true, bg: "#232325", panel: "#1c1c1e", raise: "#2a2a2c", line: "#38383b", text: "#f2f2f7", text2: "#b0b0b5", dim: "#8e8e93", accent: "#ffffff", selection: "#4c4c51" },
    midnight: { dark: true, bg: "#10141c", panel: "#0b0e14", raise: "#161b25", line: "#202634", text: "#e8ebf2", text2: "#a6aec0", dim: "#76809a", accent: "#ffffff", selection: "#313b55" },
    light: { dark: false, bg: "#ffffff", panel: "#f0f0f0", raise: "#f7f7f7", line: "#e2e2e2", text: "#1d1d1f", text2: "#55555a", dim: "#76767b", accent: "#000000", selection: "#cfcfd4" },
  };
  const DARK_ANSI = { red: "#ff7b7b", green: "#6ddf9a", yellow: "#ffc069", blue: "#8b9dff", purple: "#c4a6ff", cyan: "#8fd3f4" };
  const LIGHT_ANSI = { red: "#c4282e", green: "#1a7f37", yellow: "#9a6700", blue: "#0550ae", purple: "#8250df", cyan: "#1b7c83" };
  const themeName = params.has("mocklight") ? "light" : params.get("mocktheme") || "black";
  const base = THEMES[themeName] || THEMES.black;
  const theme = { ...base, ...(base.dark ? DARK_ANSI : LIGHT_ANSI) };
  if (params.get("mockaccent")) theme.accent = "#" + params.get("mockaccent");

  const fail = (message) => {
    const e = new Error(message);
    e.apiError = true;
    throw e;
  };

  // ---------------------------------------------------------------- graph helpers

  const headHash = () => (branch ? G.local.get(branch) : detachedAt) || null;
  function reach(...starts) {
    const seen = new Set();
    const stack = starts.filter(Boolean);
    while (stack.length) {
      const h = stack.pop();
      if (seen.has(h)) continue;
      seen.add(h);
      const c = G.byHash.get(h);
      if (c) stack.push(...c.parents);
    }
    return seen;
  }
  const minus = (a, b) => {
    let n = 0;
    for (const h of a) if (!b.has(h)) n++;
    return n;
  };
  function counts(localTip, remoteTip) {
    if (!localTip || !remoteTip) return { ahead: 0, behind: 0 };
    const a = reach(localTip), b = reach(remoteTip);
    return { ahead: minus(a, b), behind: minus(b, a) };
  }
  const shortOf = (h) => (h ? h.slice(0, 7) : "");
  function refsFor(hash) {
    const out = [];
    if (detachedAt === hash) out.push("HEAD");
    for (const [name, tip] of G.local) if (tip === hash) out.push(name === branch ? `HEAD -> ${name}` : name);
    for (const [name, tip] of G.tags) if (tip === hash) out.push(`tag: ${name}`);
    for (const [name, tip] of G.remote) if (tip === hash) out.push(name);
    if (G.remote.get("origin/main") === hash) out.push("origin/HEAD");
    return out;
  }
  function newCommit(parents, message, author = AUTHORS[0]) {
    const [subject, ...rest] = String(message).split("\n");
    const c = {
      hash: hex40(G.n++ + 1000),
      parents: parents.filter(Boolean),
      subject,
      body: rest.join("\n").replace(/^\n+/, ""),
      author: author[0],
      email: author[1],
      date: Math.floor(Date.now() / 1000),
    };
    G.commits.push(c);
    G.byHash.set(c.hash, c);
    return c.hash;
  }
  function moveHead(hash) {
    if (branch) G.local.set(branch, hash);
    else detachedAt = hash;
  }
  const worktreeOf = (name) => worktrees.find((w) => !w.current && w.branch === name);

  // Commit file lists: stable per commit, counts match commit_file.
  function filesOf(c) {
    const i = G.commits.indexOf(c);
    const rand = seeded(i * 7 + 3);
    if (!c.parents.length) return ["README.md", "Cargo.toml", "src/main.rs", ".gitignore"].map((path) => ({ path, orig: null, status: "A" }));
    const n = c.parents.length > 1 ? 4 + Math.floor(rand() * 7) : 1 + Math.floor(rand() * (rand() < 0.2 ? 9 : 4));
    const chosen = new Set();
    while (chosen.size < n) chosen.add(pick(rand, PATHS));
    return [...chosen].map((path) => {
      const r = rand();
      if (path === "assets/logo.png") return { path, orig: null, status: "M" };
      if (r < 0.1) return { path: path.replace(/(\.[a-z]+)$/, "_v2$1"), orig: null, status: "A" };
      if (r < 0.15) return { path: path.replace(/(\.[a-z]+)$/, "_old$1"), orig: null, status: "D" };
      if (r < 0.2) return { path: path.replace(/^([^/]+)\//, "$1/next/"), orig: path, status: "R" };
      return { path, orig: null, status: "M" };
    });
  }
  function fileVersions(c, f) {
    if (BINARY.test(f.path)) return { before: "", after: "", binary: true };
    const i = G.commits.indexOf(c);
    const seed = i * 131 + [...f.path].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7);
    const rand = seeded(seed);
    const src = synthText((f.orig || f.path).replace(/_(v2|old)(\.[a-z]+)$/, "$2").replace("/next/", "/"));
    const ext = extOf(f.path);
    if (f.status === "A") return { before: "", after: mutate(src, rand, ext), binary: false };
    if (f.status === "D") return { before: src, after: "", binary: false };
    const before = mutate(src, seeded(seed ^ 0x5bd1e995), ext);
    return { before, after: f.status === "R" && rand() < 0.4 ? before : mutate(before, rand, ext), binary: false };
  }

  // ---------------------------------------------------------------- status

  function statusFiles() {
    const all = new Set([...head.keys(), ...index.keys(), ...work.keys()]);
    let files = [];
    for (const path of [...all].sort()) {
      if (path.startsWith("node_modules/")) continue;
      if (conflicts.has(path)) {
        files.push({ path, orig: null, staged: "U", unstaged: "U" });
        continue;
      }
      const h = head.get(path), i = index.get(path), w = work.get(path);
      let staged = "";
      if (h === undefined && i !== undefined) staged = "A";
      else if (h !== undefined && i === undefined) staged = "D";
      else if (h !== i) staged = "M";
      let unstaged = "";
      if (i === undefined && w !== undefined) unstaged = "?";
      else if (i !== undefined && w === undefined) unstaged = "D";
      else if (i !== w) unstaged = "M";
      if (unstaged === "?") staged = "";
      if (staged || unstaged) files.push({ path, orig: null, staged, unstaged });
    }
    // Rename detection: a staged add whose content matches a staged delete.
    for (const add of files.filter((f) => f.staged === "A")) {
      const del = files.find((f) => f.staged === "D" && !f.unstaged && head.get(f.path) === index.get(add.path));
      if (del) {
        add.staged = "R";
        add.orig = del.path;
        files = files.filter((f) => f !== del);
      }
    }
    return files;
  }

  const ops = {
    hello: () => ({
      root: ROOT,
      name: "deck",
      git: !params.has("mocknogit"),
      theme,
      font: { family: "JetBrains Mono", size: +params.get("mockfont") || 13 },
      ui_font_size: +params.get("mockuisize") || +params.get("mockui") || 13,
      open: params.get("mockopen") || null,
      view: ["changes", "history"].includes(params.get("mockview")) ? params.get("mockview") : "files",
      commit: params.get("mockhello") || null,
    }),
    ls: ({ dir = "" }) => {
      const prefix = dir ? dir + "/" : "";
      const dirs = new Set(), files = new Set();
      for (const path of work.keys()) {
        if (!path.startsWith(prefix)) continue;
        const rest = path.slice(prefix.length);
        const slash = rest.indexOf("/");
        if (slash >= 0) dirs.add(rest.slice(0, slash));
        else files.add(rest);
      }
      if (dir && !dirs.size && !files.size) fail(`${dir}: No such file or directory`);
      const cmp = (a, b) => a.localeCompare(b, undefined, { sensitivity: "base" });
      return {
        entries: [
          ...[...dirs].sort(cmp).map((name) => ({ name, dir: true })),
          ...[...files].sort(cmp).map((name) => ({ name, dir: false })),
        ],
      };
    },
    read: ({ path }) => {
      if (!work.has(path)) fail(`${path}: No such file or directory`);
      const size = SIZES[path] ?? new Blob([work.get(path)]).size;
      if (BINARY.test(path)) return { binary: true, size: SIZES[path] ?? 112_640 };
      if (size > 4_000_000) return { too_large: true, size };
      return { text: work.get(path), size };
    },
    write: ({ path, text }) => {
      if (typeof text !== "string") fail("write: missing text");
      work.set(path, text);
      return { ok: true };
    },
    status: () => {
      const up = branch ? G.upstream.get(branch) ?? null : null;
      const { ahead, behind } = up ? counts(G.local.get(branch), G.remote.get(up)) : { ahead: 0, behind: 0 };
      return { branch, detached: !branch, upstream: up, ahead, behind, files: statusFiles() };
    },
    base: ({ path, against }) => {
      const map = against === "index" ? index : head;
      return map.has(path) ? { text: map.get(path), exists: true } : { text: "", exists: false };
    },
    stage: ({ paths }) => {
      for (const p of paths) {
        const dir = p.endsWith("/") ? p : null;
        const list = dir ? [...work.keys()].filter((k) => k.startsWith(dir)) : [p];
        for (const q of list) {
          conflicts.delete(q);
          work.has(q) ? index.set(q, work.get(q)) : index.delete(q);
        }
      }
      return { ok: true };
    },
    unstage: ({ paths }) => {
      for (const p of paths) head.has(p) ? index.set(p, head.get(p)) : index.delete(p);
      // A staged rename unstages both sides.
      for (const p of paths) for (const [k, v] of head) if (!index.has(k) && v === work.get(p)) index.set(k, v);
      return { ok: true };
    },
    discard: ({ paths }) => {
      for (const p of paths) {
        const list = p.endsWith("/") ? [...work.keys()].filter((k) => k.startsWith(p)) : [p];
        for (const q of list) {
          conflicts.delete(q);
          index.has(q) ? work.set(q, index.get(q)) : work.delete(q);
        }
      }
      return { ok: true };
    },
    commit: ({ message, amend, push }) => {
      if (!message?.trim()) fail("Aborting commit due to empty commit message.");
      if (conflicts.size) fail("error: Committing is not possible because you have unmerged files.\nhint: Fix them up in the work tree, and then use 'git add <file>'.");
      const changed = statusFiles().filter((f) => f.staged).length;
      if (!changed && !amend && !mergeHead) fail("nothing to commit, working tree clean");
      head = new Map(index);
      const tip = headHash();
      const parents = amend ? G.byHash.get(tip)?.parents || [] : [tip, mergeHead];
      mergeHead = null;
      const hash = newCommit(parents, message);
      moveHead(hash);
      let out = `[${branch || "detached HEAD"} ${shortOf(hash)}] ${message.split("\n")[0]}\n ${changed} file${changed === 1 ? "" : "s"} changed`;
      if (push) out += "\n" + ops.push().output;
      return { ok: true, output: out };
    },
    last_message: () => {
      const c = G.byHash.get(headHash());
      return { message: c ? c.subject + (c.body ? "\n\n" + c.body : "") : "" };
    },
    branches: () => {
      const cur = headHash();
      const curReach = reach(cur);
      const info = (tip) => {
        const c = G.byHash.get(tip);
        return { date: c?.date || 0, subject: c?.subject || "" };
      };
      const list = [];
      for (const [name, tip] of [...G.local].sort((a, b) => a[0].localeCompare(b[0]))) {
        const up = G.upstream.get(name) ?? null;
        const { ahead, behind } = up && G.remote.has(up) ? counts(tip, G.remote.get(up)) : { ahead: 0, behind: 0 };
        list.push({ name, upstream: up, current: name === branch, remote: false, merged: name !== branch && curReach.has(tip), ahead, behind, ...info(tip) });
      }
      for (const [name, tip] of [...G.remote].sort((a, b) => a[0].localeCompare(b[0])))
        list.push({ name, upstream: null, current: false, remote: true, merged: curReach.has(tip), ahead: 0, behind: 0, ...info(tip) });
      return { current: branch, branches: list };
    },
    switch: ({ name, create }) => {
      if (create) {
        if (G.local.has(name)) fail(`fatal: a branch named '${name}' already exists`);
        G.local.set(name, headHash());
        G.upstream.set(name, null);
        branch = name;
        detachedAt = null;
        return { ok: true, output: `Switched to a new branch '${name}'` };
      }
      const wt = worktreeOf(name);
      if (wt) fail(`fatal: '${name}' is already used by worktree at '${wt.path}'`);
      if (!G.local.has(name)) {
        const r = G.remote.get("origin/" + name);
        if (!r) fail(`fatal: invalid reference: ${name}`);
        G.local.set(name, r);
        G.upstream.set(name, "origin/" + name);
        branch = name;
        detachedAt = null;
        worktrees[0].branch = name;
        return { ok: true, output: `branch '${name}' set up to track 'origin/${name}'.\nSwitched to a new branch '${name}'` };
      }
      branch = name;
      detachedAt = null;
      worktrees[0].branch = name;
      return { ok: true, output: `Switched to branch '${name}'` };
    },
    fetch: () => ({ ok: true, output: "From github.com:stroke/deck\n   3f92312..9be41a0  main       -> origin/main" }),
    pull: () => {
      const up = branch && G.upstream.get(branch);
      if (!up) fail(`There is no tracking information for the current branch.`);
      const { ahead, behind } = counts(G.local.get(branch), G.remote.get(up));
      if (!behind) return { ok: true, output: "Already up to date." };
      if (ahead) moveHead(newCommit([G.local.get(branch), G.remote.get(up)], `Merge branch '${up.replace(/^origin\//, "")}' of github.com:stroke/deck`));
      else moveHead(G.remote.get(up));
      return { ok: true, output: ahead ? "Merge made by the 'ort' strategy." : `Updating ${shortOf(G.local.get(branch))}\nFast-forward` };
    },
    push: () => {
      if (!branch) fail("fatal: You are not currently on a branch.");
      const up = G.upstream.get(branch) || `origin/${branch}`;
      const published = !G.upstream.get(branch);
      if (!published) {
        const { behind } = counts(G.local.get(branch), G.remote.get(up));
        if (behind) fail(`To github.com:stroke/deck.git\n ! [rejected]        ${branch} -> ${branch} (fetch first)\nerror: failed to push some refs to 'github.com:stroke/deck.git'\nhint: Updates were rejected because the remote contains work that you do not\nhint: have locally.`);
      }
      G.upstream.set(branch, up);
      G.remote.set(up, G.local.get(branch));
      return {
        ok: true,
        output: published
          ? `branch '${branch}' set up to track '${up}'.\nTo github.com:stroke/deck.git\n * [new branch]      ${branch} -> ${branch}`
          : `To github.com:stroke/deck.git\n   3f92312..9be41a0  ${branch} -> ${branch}`,
      };
    },
    stash: () => {
      if (!statusFiles().some((f) => f.unstaged !== "?")) fail("No local changes to save");
      stashes.push({ work: new Map(work), index: new Map(index) });
      const untracked = [...work].filter(([p]) => !index.has(p) && !head.has(p));
      index = new Map(head);
      work.clear();
      for (const [p, t] of [...head, ...untracked]) work.set(p, t);
      return { ok: true, output: `Saved working directory and index state WIP on ${branch}` };
    },
    unstash: () => {
      const s = stashes.pop();
      if (!s) fail("No stash entries found.");
      work.clear();
      for (const [p, t] of s.work) work.set(p, t);
      index = s.index;
      return { ok: true, output: "Dropped refs/stash@{0}" };
    },

    // ------------------------------------------------------------ history

    log: ({ limit = 200, skip = 0, all = true }) => {
      const starts = all ? [headHash(), ...G.local.values(), ...G.remote.values(), ...G.tags.values()] : [headHash()];
      const set = reach(...starts);
      const list = G.commits.filter((c) => set.has(c.hash)).sort((a, b) => b.date - a.date);
      return {
        commits: list.slice(skip, skip + limit).map((c) => ({
          hash: c.hash,
          short: shortOf(c.hash),
          parents: c.parents,
          subject: c.subject,
          author: c.author,
          email: c.email,
          date: c.date,
          refs: refsFor(c.hash),
        })),
      };
    },
    show: ({ hash }) => {
      const c = G.byHash.get(hash) || G.commits.find((x) => x.hash.startsWith(String(hash || "")));
      if (!c || !hash) fail(`fatal: bad object ${hash}`);
      const files = filesOf(c).map((f) => {
        const v = fileVersions(c, f);
        const n = v.binary ? { added: null, removed: null } : diffCounts(v.before, v.after);
        return { ...f, ...n };
      });
      return { hash: c.hash, short: shortOf(c.hash), parents: c.parents, subject: c.subject, body: c.body, author: c.author, email: c.email, date: c.date, refs: refsFor(c.hash), files };
    },
    commit_file: ({ hash, path, orig }) => {
      const c = G.byHash.get(hash);
      if (!c) fail(`fatal: bad object ${hash}`);
      const f = filesOf(c).find((x) => x.path === path && (!orig || x.orig === orig));
      if (!f) fail(`fatal: path '${path}' does not exist in '${shortOf(hash)}'`);
      return fileVersions(c, f);
    },
    worktrees: () => ({
      worktrees: worktrees.map((w) => ({
        path: w.path,
        branch: w.current ? branch : w.branch,
        head: w.current ? headHash() || "" : w.branch ? G.local.get(w.branch) || "" : w.head || "",
        detached: w.current ? !branch : !w.branch,
        locked: !!w.locked,
        prunable: !!w.prunable,
        current: !!w.current,
      })),
    }),
    worktree_add: ({ path, branch: name, create, base }) => {
      if (!name) fail("fatal: no branch given");
      const rel = path || `../deck-${name.replace(/[/\s]+/g, "-")}`;
      const parts = (rel.startsWith("/") ? rel : ROOT + "/" + rel).split("/");
      const abs = [];
      for (const p of parts) p === ".." ? abs.pop() : p && p !== "." && abs.push(p);
      const full = "/" + abs.join("/");
      if (worktrees.some((w) => w.path === full)) fail(`fatal: '${full}' already exists`);
      if (create) {
        if (G.local.has(name)) fail(`fatal: a branch named '${name}' already exists`);
        G.local.set(name, (base && (G.local.get(base) || G.remote.get(base))) || headHash());
        G.upstream.set(name, null);
      } else {
        if (!G.local.has(name)) {
          const r = G.remote.get("origin/" + name);
          if (!r) fail(`fatal: invalid reference: ${name}`);
          G.local.set(name, r);
          G.upstream.set(name, "origin/" + name);
        }
        const used = worktrees.find((w) => w.branch === name);
        if (used) fail(`fatal: '${name}' is already used by worktree at '${used.path}'`);
      }
      worktrees.push({ path: full, branch: name, detached: false, locked: false, prunable: false, current: false });
      const tip = G.byHash.get(G.local.get(name));
      return { ok: true, path: full, output: `Preparing worktree (${create ? "new branch" : "checking out"} '${name}')\nHEAD is now at ${shortOf(tip?.hash)} ${tip?.subject || ""}` };
    },
    worktree_remove: ({ path, force }) => {
      const w = worktrees.find((x) => x.path === path);
      if (!w) fail(`fatal: '${path}' is not a working tree`);
      if (w.current) fail(`fatal: '${path}' is a main working tree`);
      if (w.locked && !force) fail(`fatal: cannot remove a locked working tree, lock reason: reviewing 0.4\nuse 'remove -f -f' to override or unlock first`);
      if (w.dirty && !force) fail(`fatal: '${path}' contains modified or untracked files, use --force to delete it`);
      worktrees = worktrees.filter((x) => x !== w);
      return { ok: true, output: "" };
    },
    merge: ({ name }) => {
      const target = G.local.get(name) || G.remote.get(name);
      if (!target) fail(`merge: ${name} - not something we can merge`);
      const tip = headHash();
      const mine = reach(tip);
      if (mine.has(target)) return { ok: true, output: "Already up to date." };
      if (reach(target).has(tip)) {
        moveHead(target);
        return { ok: true, output: `Updating ${shortOf(tip)}..${shortOf(target)}\nFast-forward` };
      }
      if (name === "fix/crlf" || params.has("mockmergeconflict")) {
        conflicts = new Set(["src/config.rs", "web/styles.css"]);
        mergeHead = target;
        fail("Auto-merging src/config.rs\nCONFLICT (content): Merge conflict in src/config.rs\nAuto-merging web/styles.css\nCONFLICT (content): Merge conflict in web/styles.css\nAutomatic merge failed; fix conflicts and then commit the result.");
      }
      moveHead(newCommit([tip, target], `Merge branch '${name}'${branch && branch !== "main" ? ` into ${branch}` : ""}`));
      return { ok: true, output: "Merge made by the 'ort' strategy." };
    },
    delete_branch: ({ name, force }) => {
      if (name === branch) fail(`error: Cannot delete branch '${name}' checked out at '${ROOT}'`);
      const wt = worktreeOf(name);
      if (wt) fail(`error: Cannot delete branch '${name}' checked out at '${wt.path}'`);
      const tip = G.local.get(name);
      if (!tip) fail(`error: branch '${name}' not found.`);
      if (!force && !reach(headHash()).has(tip)) fail(`error: The branch '${name}' is not fully merged.\nIf you are sure you want to delete it, run 'git branch -D ${name}'.`);
      G.local.delete(name);
      G.upstream.delete(name);
      return { ok: true, output: `Deleted branch ${name} (was ${shortOf(tip)}).` };
    },
  };

  return async function call(op, args) {
    await new Promise((r) => setTimeout(r, op === "log" || op === "show" ? 60 : 25));
    const fn = ops[op];
    if (!fn) fail(`unknown op: ${op}`);
    return JSON.parse(JSON.stringify(fn(args || {})));
  };
}
