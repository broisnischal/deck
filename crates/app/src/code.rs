//! Deck's code editor: a CodeMirror app (`crates/app/editor`, built into
//! `dist/`) shown in a browser pane at `deckcode://localhost/`, and the API
//! that page calls on the same origin to read and write the project's files
//! and work its git repository.
//!
//! Only a pane Deck opened as an editor answers the API. Any other page on
//! the scheme gets the app's files and a 403, so a website in a browser pane
//! cannot reach the disk through it.

use serde_json::{json, Value};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SCHEME: &str = "deckcode";
const ORIGIN: &str = "deckcode://localhost/";

static INDEX: &[u8] = include_bytes!("../editor/dist/index.html");
static APP_JS: &[u8] = include_bytes!("../editor/dist/app.js");
static APP_CSS: &[u8] = include_bytes!("../editor/dist/app.css");

/// Files over this open as "too large" rather than in the editor.
const MAX_READ: u64 = 5 * 1024 * 1024;

/// What one editor pane works on.
#[derive(Clone, Debug, PartialEq)]
pub struct Editor {
    /// The project: the folder the file tree starts at.
    pub root: PathBuf,
    /// A file to open first, relative to `root`.
    pub open: Option<String>,
    /// "files", "changes" or "history".
    pub view: &'static str,
    /// A commit to show first (History).
    pub commit: Option<String>,
}

pub type Shared = Arc<Mutex<Option<Editor>>>;

impl Editor {
    /// The page's address; kept in the session so the pane comes back.
    pub fn url(&self) -> String {
        let q = crate::browser::encode_query;
        let mut url = format!("{ORIGIN}index.html?root={}", q(&self.root.to_string_lossy()));
        if self.view != "files" {
            url.push_str(&format!("&view={}", self.view));
        }
        url
    }

    /// The editor a saved address names (the session is ours, so its root is
    /// trusted).
    pub fn from_url(url: &str) -> Option<Editor> {
        let query = url.strip_prefix(ORIGIN)?.split_once('?')?.1;
        let mut root = None;
        let mut view = "files";
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "root" => root = Some(PathBuf::from(decode_query(v))),
                "view" if v == "changes" => view = "changes",
                "view" if v == "history" => view = "history",
                _ => {}
            }
        }
        Some(Editor {
            root: root.filter(|r| r.is_dir())?,
            open: None,
            view,
            commit: None,
        })
    }
}

pub fn is_editor_url(url: &str) -> bool {
    url.starts_with("deckcode:")
}

fn decode_query(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The terminal's font, which the editor writes code in too.
static FONT: Mutex<(String, f32)> = Mutex::new((String::new(), 13.));

pub fn set_font(family: &str, px: f32) {
    *FONT.lock().unwrap_or_else(|e| e.into_inner()) = (family.to_string(), px);
}

/// The theme and fonts, as the page wants them (also sent on a change).
pub fn look() -> Value {
    use crate::theme;
    let c = theme::current();
    let font = FONT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let hex = |v: u32| format!("#{:06x}", v & 0xffffff);
    json!({
        "theme": {
            "dark": c.dark,
            "bg": hex(c.pane),
            "panel": hex(c.sidebar),
            "raise": hex(c.raised),
            "line": hex(c.border),
            "text": hex(c.text),
            "text2": hex(c.text_2),
            "dim": hex(c.dim),
            "accent": hex(c.accent),
            "red": hex(c.ansi[1]),
            "green": hex(c.ansi[2]),
            "yellow": hex(c.ansi[3]),
            "blue": hex(c.ansi[4]),
            "purple": hex(c.ansi[5]),
            "cyan": hex(c.ansi[6]),
            "selection": hex(c.selection),
        },
        "font": { "family": font.0, "size": font.1 },
        "ui_font_size": 13,
    })
}

/// The handler for `deckcode://`: the app's files, and the API for an
/// editor pane. API calls run on a thread of their own (git can be slow).
pub fn protocol(
    shared: Shared,
) -> impl Fn(wry::WebViewId, wry::http::Request<Vec<u8>>, wry::RequestAsyncResponder)
       + Send
       + Sync
       + 'static {
    move |_, req, responder| {
        let path = req.uri().path().to_string();
        let file = |bytes: &'static [u8], mime: &str| reply(200, mime, Cow::Borrowed(bytes));
        match path.as_str() {
            "/" | "/index.html" => responder.respond(file(INDEX, "text/html; charset=utf-8")),
            "/app.js" => responder.respond(file(APP_JS, "text/javascript; charset=utf-8")),
            "/app.css" => responder.respond(file(APP_CSS, "text/css; charset=utf-8")),
            p if p.starts_with("/api/") => {
                let editor = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let Some(editor) = editor else {
                    return responder.respond(json_reply(403, &json!({ "error": "this page is not an editor" })));
                };
                let op = p["/api/".len()..].to_string();
                let body = req.into_body();
                std::thread::Builder::new()
                    .name("deck-editor".into())
                    .spawn(move || {
                        let args: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
                        let out = match api(&editor, &op, &args) {
                            Ok(v) => json_reply(200, &v),
                            Err(e) => json_reply(400, &json!({ "error": e })),
                        };
                        responder.respond(out);
                    })
                    .ok();
            }
            _ => responder.respond(reply(404, "text/plain", Cow::Borrowed(b"not found"))),
        }
    }
}

fn reply(status: u16, mime: &str, body: Cow<'static, [u8]>) -> wry::http::Response<Cow<'static, [u8]>> {
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .body(body)
        .unwrap_or_else(|_| wry::http::Response::new(Cow::Borrowed(b"")))
}

fn json_reply(status: u16, v: &Value) -> wry::http::Response<Cow<'static, [u8]>> {
    reply(status, "application/json", Cow::Owned(serde_json::to_vec(v).unwrap_or_default()))
}

// -- the API ----------------------------------------------------------------------------

type R = Result<Value, String>;

fn api(ed: &Editor, op: &str, a: &Value) -> R {
    let root = &ed.root;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let paths = || -> Result<Vec<String>, String> {
        let list: Vec<String> = a
            .get("paths")
            .and_then(Value::as_array)
            .map(|v| v.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        for p in &list {
            inside(root, p)?;
        }
        if list.is_empty() {
            return Err("no files given".into());
        }
        Ok(list)
    };
    match op {
        "hello" => {
            let mut v = look();
            v["root"] = json!(root);
            v["name"] = json!(root.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
            v["git"] = json!(git(root, &["rev-parse", "--is-inside-work-tree"]).is_ok());
            v["open"] = json!(ed.open);
            v["view"] = json!(ed.view);
            v["commit"] = json!(ed.commit);
            Ok(v)
        }
        "ls" => ls(root, &s("dir")),
        "read" => read(root, &s("path")),
        "write" => {
            let path = inside(root, &s("path"))?;
            let text = a.get("text").and_then(Value::as_str).ok_or("no text")?;
            std::fs::write(&path, text).map_err(|e| format!("couldn't save: {e}"))?;
            Ok(json!({ "ok": true }))
        }
        "status" => status(root),
        "base" => {
            let path = s("path");
            inside(root, &path)?;
            let spec = match s("against").as_str() {
                "index" => format!(":./{path}"),
                _ => format!("HEAD:./{path}"),
            };
            Ok(match git(root, &["show", &spec]) {
                Ok(text) => json!({ "text": text, "exists": true }),
                Err(_) => json!({ "text": "", "exists": false }),
            })
        }
        "stage" => {
            let mut args = vec!["add", "--"];
            let p = paths()?;
            args.extend(p.iter().map(String::as_str));
            git(root, &args).map(|_| json!({ "ok": true }))
        }
        "unstage" => {
            let mut args = vec!["restore", "--staged", "--"];
            let p = paths()?;
            args.extend(p.iter().map(String::as_str));
            git(root, &args).map(|_| json!({ "ok": true }))
        }
        "discard" => discard(root, &paths()?),
        "commit" => {
            let message = s("message");
            let amend = a.get("amend").and_then(Value::as_bool).unwrap_or(false);
            let mut args = vec!["commit"];
            if amend {
                args.push("--amend");
            }
            if message.trim().is_empty() {
                if !amend {
                    return Err("write a commit message first".into());
                }
                args.push("--no-edit");
            } else {
                args.extend(["-m", message.as_str()]);
            }
            let out = git(root, &args)?;
            if a.get("push").and_then(Value::as_bool).unwrap_or(false) {
                let pushed = push(root)?;
                return Ok(json!({ "ok": true, "output": format!("{}\n{}", out.trim(), pushed["output"].as_str().unwrap_or("")) }));
            }
            Ok(json!({ "ok": true, "output": out.trim() }))
        }
        "last_message" => Ok(json!({ "message": git(root, &["log", "-1", "--format=%B"]).unwrap_or_default().trim_end() })),
        "branches" => branches(root),
        "switch" => {
            let name = s("name");
            if name.trim().is_empty() || name.starts_with('-') {
                return Err("that isn't a branch name".into());
            }
            let create = a.get("create").and_then(Value::as_bool).unwrap_or(false);
            let args: Vec<&str> = if create {
                vec!["switch", "-c", &name]
            } else {
                vec!["switch", &name]
            };
            git(root, &args).map(|out| json!({ "ok": true, "output": out.trim() }))
        }
        "fetch" => net(root, &["fetch", "--prune"]),
        "pull" => net(root, &["pull", "--ff-only"]),
        "push" => push(root),
        "log" => log(root, a),
        "show" => show(root, &s("hash")),
        "commit_file" => commit_file(root, &s("hash"), &s("path"), a.get("orig").and_then(Value::as_str)),
        "worktrees" => worktrees(root),
        "worktree_add" => {
            let branch = s("branch");
            if branch.trim().is_empty() || branch.starts_with('-') {
                return Err("that isn't a branch name".into());
            }
            let top = toplevel(root)?;
            let mut path = PathBuf::from(s("path"));
            if path.as_os_str().is_empty() {
                let repo = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                path = PathBuf::from(format!("../{repo}-{}", branch.replace('/', "-")));
            }
            let path = if path.is_absolute() { path } else { top.join(path) };
            let path_s = path.to_string_lossy().into_owned();
            let create = a.get("create").and_then(Value::as_bool).unwrap_or(false);
            let base = a.get("base").and_then(Value::as_str).filter(|b| !b.is_empty() && !b.starts_with('-'));
            let mut args: Vec<&str> = vec!["worktree", "add"];
            if create {
                args.extend(["-b", branch.as_str(), path_s.as_str()]);
                args.extend(base);
            } else {
                args.extend([path_s.as_str(), branch.as_str()]);
            }
            let out = run(&top, &args, Duration::from_secs(120))?;
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            Ok(json!({ "ok": true, "output": out.trim(), "path": path }))
        }
        "worktree_remove" => {
            let path = s("path");
            if path.trim().is_empty() {
                return Err("which worktree?".into());
            }
            let mut args = vec!["worktree", "remove"];
            if a.get("force").and_then(Value::as_bool).unwrap_or(false) {
                args.push("--force");
            }
            args.push(path.as_str());
            git(root, &args).map(|o| json!({ "ok": true, "output": o.trim() }))
        }
        "merge" => {
            let name = s("name");
            if name.trim().is_empty() || name.starts_with('-') {
                return Err("that isn't a branch name".into());
            }
            git(root, &["merge", "--no-edit", &name]).map(|o| json!({ "ok": true, "output": o.trim() }))
        }
        "delete_branch" => {
            let name = s("name");
            if name.trim().is_empty() || name.starts_with('-') {
                return Err("that isn't a branch name".into());
            }
            let flag = if a.get("force").and_then(Value::as_bool).unwrap_or(false) { "-D" } else { "-d" };
            git(root, &["branch", flag, &name]).map(|o| json!({ "ok": true, "output": o.trim() }))
        }
        "stash" => git(root, &["stash", "push", "--include-untracked", "-m", "Deck"]).map(|o| json!({ "ok": true, "output": o.trim() })),
        "unstash" => git(root, &["stash", "pop"]).map(|o| json!({ "ok": true, "output": o.trim() })),
        _ => Err(format!("unknown call {op}")),
    }
}

/// `rel` under `root`, refusing anything that climbs out of it.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches("./");
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err(format!("{rel} is outside the project"));
    }
    Ok(root.join(p))
}

fn ls(root: &Path, dir: &str) -> R {
    let path = inside(root, dir)?;
    let mut entries: Vec<(bool, String)> = std::fs::read_dir(&path)
        .map_err(|e| format!("couldn't list {dir}: {e}"))?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            // A link to a folder lists as a folder.
            let dir = std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false);
            Some((dir, name))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    Ok(json!({ "entries": entries.into_iter().map(|(dir, name)| json!({ "name": name, "dir": dir })).collect::<Vec<_>>() }))
}

fn read(root: &Path, rel: &str) -> R {
    let path = inside(root, rel)?;
    let size = std::fs::metadata(&path).map_err(|e| format!("couldn't open {rel}: {e}"))?.len();
    if size > MAX_READ {
        return Ok(json!({ "too_large": true, "size": size }));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("couldn't open {rel}: {e}"))?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return Ok(json!({ "binary": true, "size": size }));
    }
    Ok(json!({ "text": String::from_utf8_lossy(&bytes), "size": size }))
}

fn status(root: &Path) -> R {
    let out = git(root, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])?;
    Ok(parse_status(&out, &prefix(root)))
}

/// Where `root` sits inside its repository ("" at the top, else "sub/dir/"):
/// git reports paths from the top, the editor wants them from `root`.
fn prefix(root: &Path) -> String {
    git(root, &["rev-parse", "--show-prefix"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn parse_status(out: &str, prefix: &str) -> Value {
    let mut branch = None;
    let mut detached = false;
    let mut upstream = None;
    let (mut ahead, mut behind) = (0i64, 0i64);
    let mut files = Vec::new();
    let mut fields = out.split('\0').peekable();
    let local = |p: &str| p.strip_prefix(prefix).map(str::to_string);
    let letter = |c: char| if c == '.' { String::new() } else { c.to_string() };
    while let Some(f) = fields.next() {
        if let Some(h) = f.strip_prefix("# branch.head ") {
            if h == "(detached)" {
                detached = true;
            } else {
                branch = Some(h.to_string());
            }
        } else if let Some(u) = f.strip_prefix("# branch.upstream ") {
            upstream = Some(u.to_string());
        } else if let Some(ab) = f.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ');
            ahead = it.next().and_then(|s| s.trim_start_matches('+').parse().ok()).unwrap_or(0);
            behind = it.next().and_then(|s| s.trim_start_matches('-').parse().ok()).unwrap_or(0);
        } else if let Some(rest) = f.strip_prefix("1 ").or_else(|| f.strip_prefix("u ")) {
            // XY sub mH mI mW hH hI path (u: XY sub m1 m2 m3 mW h1 h2 h3 path)
            let unmerged = f.starts_with('u');
            let n = if unmerged { 9 } else { 7 };
            let parts: Vec<&str> = rest.splitn(n + 1, ' ').collect();
            let (Some(xy), Some(path)) = (parts.first(), parts.get(n)) else { continue };
            let Some(path) = local(path) else { continue };
            let mut c = xy.chars();
            let (x, y) = (c.next().unwrap_or('.'), c.next().unwrap_or('.'));
            let (staged, unstaged) = if unmerged { ("U".into(), "U".into()) } else { (letter(x), letter(y)) };
            files.push(json!({ "path": path, "orig": null, "staged": staged, "unstaged": unstaged }));
        } else if let Some(rest) = f.strip_prefix("2 ") {
            // XY sub mH mI mW hH hI Xscore path, then the original path.
            let parts: Vec<&str> = rest.splitn(9, ' ').collect();
            let orig = fields.next().unwrap_or("");
            let (Some(xy), Some(path)) = (parts.first(), parts.get(8)) else { continue };
            let Some(path) = local(path) else { continue };
            let mut c = xy.chars();
            let (x, y) = (c.next().unwrap_or('.'), c.next().unwrap_or('.'));
            files.push(json!({ "path": path, "orig": local(orig), "staged": letter(x), "unstaged": letter(y) }));
        } else if let Some(path) = f.strip_prefix("? ") {
            if let Some(path) = local(path) {
                files.push(json!({ "path": path, "orig": null, "staged": "", "unstaged": "?" }));
            }
        }
    }
    json!({ "branch": branch, "detached": detached, "upstream": upstream, "ahead": ahead, "behind": behind, "files": files })
}

fn discard(root: &Path, paths: &[String]) -> R {
    let mut tracked = Vec::new();
    for p in paths {
        let untracked = git(root, &["ls-files", "--error-unmatch", "--", p]).is_err();
        if untracked {
            let full = inside(root, p)?;
            let gone = if full.is_dir() { std::fs::remove_dir_all(&full) } else { std::fs::remove_file(&full) };
            gone.map_err(|e| format!("couldn't delete {p}: {e}"))?;
        } else {
            tracked.push(p.as_str());
        }
    }
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(tracked);
        git(root, &args)?;
    }
    Ok(json!({ "ok": true }))
}

fn branches(root: &Path) -> R {
    let out = git(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(refname:short)%00%(upstream:short)%00%(upstream:track)%00%(HEAD)%00%(committerdate:unix)%00%(contents:subject)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    // Merged into what you're on: everything `git branch --merged` lists.
    let merged: std::collections::HashSet<String> = git(root, &["branch", "--all", "--merged", "HEAD", "--format=%(refname)"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let mut current = None;
    let list: Vec<Value> = out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\0').collect();
            let (full, name) = (*f.first()?, f.get(1)?.to_string());
            let remote = full.starts_with("refs/remotes/");
            if remote && name.ends_with("/HEAD") {
                return None;
            }
            let here = f.get(4) == Some(&"*");
            if here {
                current = Some(name.clone());
            }
            let (ahead, behind) = track(f.get(3).copied().unwrap_or(""));
            Some(json!({
                "name": name,
                "upstream": f.get(2).filter(|u| !u.is_empty()),
                "current": here,
                "remote": remote,
                "merged": !here && merged.contains(full),
                "ahead": ahead,
                "behind": behind,
                "date": f.get(5).and_then(|d| d.parse::<i64>().ok()),
                "subject": f.get(6).copied().unwrap_or(""),
            }))
        })
        .collect();
    Ok(json!({ "current": current, "branches": list }))
}

/// `[ahead 1, behind 2]` → (1, 2).
fn track(t: &str) -> (i64, i64) {
    let num = |key: &str| {
        t.split(key)
            .nth(1)
            .and_then(|r| r.trim_start().split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    (num("ahead"), num("behind"))
}

fn push(root: &Path) -> R {
    if git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok() {
        net(root, &["push"])
    } else {
        net(root, &["push", "--set-upstream", "origin", "HEAD"])
    }
}

fn toplevel(root: &Path) -> Result<PathBuf, String> {
    git(root, &["rev-parse", "--show-toplevel"]).map(|s| PathBuf::from(s.trim()))
}

const F: char = '\x1f';
const REC: char = '\x1e';

fn refs_of(d: &str) -> Vec<String> {
    d.split(", ").map(str::trim).filter(|r| !r.is_empty()).map(str::to_string).collect()
}

/// The history, newest first in topological order, for the graph.
fn log(root: &Path, a: &Value) -> R {
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(200).clamp(1, 2000).to_string();
    let skip = a.get("skip").and_then(Value::as_u64).unwrap_or(0).to_string();
    let all = a.get("all").and_then(Value::as_bool).unwrap_or(true);
    let format = format!("--format=%H{F}%h{F}%P{F}%s{F}%an{F}%ae{F}%at{F}%D{REC}");
    let mut args = vec!["log", "--topo-order", format.as_str(), "-n", limit.as_str(), "--skip", skip.as_str()];
    if all {
        args.push("--all");
    }
    // No commits yet is an empty history, not an error.
    let out = match git(root, &args) {
        Ok(out) => out,
        Err(_) => return Ok(json!({ "commits": [] })),
    };
    let commits: Vec<Value> = out
        .split(REC)
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.split(F).collect();
            Some(json!({
                "hash": f.first()?,
                "short": f.get(1)?,
                "parents": f.get(2)?.split_whitespace().collect::<Vec<_>>(),
                "subject": f.get(3)?,
                "author": f.get(4)?,
                "email": f.get(5)?,
                "date": f.get(6)?.parse::<i64>().unwrap_or(0),
                "refs": refs_of(f.get(7).copied().unwrap_or("")),
            }))
        })
        .collect();
    Ok(json!({ "commits": commits }))
}

fn valid_hash(h: &str) -> Result<&str, String> {
    if h.len() >= 4 && h.len() <= 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(h)
    } else {
        Err("that isn't a commit".into())
    }
}

/// One commit: who, when, why, and what it changed (against its first
/// parent, for a merge).
fn show(root: &Path, hash: &str) -> R {
    let hash = valid_hash(hash)?;
    let format = format!("--format=%H{F}%h{F}%P{F}%s{F}%b{F}%an{F}%ae{F}%at{F}%D");
    let head = git(root, &["show", "-s", format.as_str(), hash])?;
    let f: Vec<&str> = head.trim_end_matches('\n').split(F).collect();
    let parents: Vec<&str> = f.get(2).copied().unwrap_or("").split_whitespace().collect();
    let range: Vec<String> = match parents.first() {
        Some(p) => vec![p.to_string(), hash.to_string()],
        None => vec!["--root".into(), hash.to_string()],
    };
    let tree = |kind: &str| {
        let mut args = vec!["diff-tree", "-r", "-M", "-z", "--no-commit-id", kind];
        args.extend(range.iter().map(String::as_str));
        git(root, &args).unwrap_or_default()
    };
    // name-status: "M\0path\0" or "R100\0old\0new\0"
    let names = tree("--name-status");
    let mut files: Vec<Value> = Vec::new();
    let mut it = names.split('\0').filter(|x| !x.is_empty());
    while let Some(st) = it.next() {
        let letter = st.chars().next().unwrap_or('M').to_string();
        if st.starts_with('R') || st.starts_with('C') {
            let (Some(old), Some(new)) = (it.next(), it.next()) else { break };
            files.push(json!({ "path": new, "orig": old, "status": letter, "added": null, "removed": null }));
        } else if let Some(path) = it.next() {
            files.push(json!({ "path": path, "orig": null, "status": letter, "added": null, "removed": null }));
        }
    }
    // numstat: "a\tr\tpath\0" or "a\tr\t\0old\0new\0"; "-" for binary.
    let nums = tree("--numstat");
    let mut it = nums.split('\0');
    while let Some(rec) = it.next() {
        let mut parts = rec.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let path = if path.is_empty() {
            let _old = it.next();
            it.next().unwrap_or("").to_string()
        } else {
            path.to_string()
        };
        if let Some(file) = files.iter_mut().find(|x| x["path"] == path.as_str()) {
            file["added"] = add.parse::<i64>().map(Value::from).unwrap_or(Value::Null);
            file["removed"] = del.parse::<i64>().map(Value::from).unwrap_or(Value::Null);
        }
    }
    Ok(json!({
        "hash": f.first().copied().unwrap_or(hash),
        "short": f.get(1).copied().unwrap_or(""),
        "parents": parents,
        "subject": f.get(3).copied().unwrap_or(""),
        "body": f.get(4).copied().unwrap_or("").trim(),
        "author": f.get(5).copied().unwrap_or(""),
        "email": f.get(6).copied().unwrap_or(""),
        "date": f.get(7).and_then(|d| d.trim().parse::<i64>().ok()).unwrap_or(0),
        "refs": refs_of(f.get(8).copied().unwrap_or("").trim()),
        "files": files,
    }))
}

/// A file before and after a commit (against its first parent).
fn commit_file(root: &Path, hash: &str, path: &str, orig: Option<&str>) -> R {
    let hash = valid_hash(hash)?;
    if path.is_empty() || path.starts_with('-') || path.split('/').any(|c| c == "..") {
        return Err("that isn't a file in the project".into());
    }
    let top = toplevel(root)?;
    let at = |rev: &str, p: &str| run(&top, &["show", &format!("{rev}:{p}")], Duration::from_secs(30)).ok();
    let parent = git(root, &["rev-parse", &format!("{hash}^1")]).ok().map(|p| p.trim().to_string());
    let before = parent.as_deref().and_then(|p| at(p, orig.unwrap_or(path))).unwrap_or_default();
    let after = at(hash, path).unwrap_or_default();
    let binary = before.contains('\0') || after.contains('\0');
    Ok(json!({ "before": if binary { "" } else { &before }, "after": if binary { "" } else { &after }, "binary": binary }))
}

fn worktrees(root: &Path) -> R {
    let out = git(root, &["worktree", "list", "--porcelain"])?;
    let here = toplevel(root).ok().and_then(|t| std::fs::canonicalize(t).ok());
    let mut list = Vec::new();
    for block in out.split("\n\n").filter(|b| !b.trim().is_empty()) {
        let mut w = json!({ "path": "", "branch": null, "head": "", "detached": false, "locked": false, "prunable": false, "current": false });
        for line in block.lines() {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            match k {
                "worktree" => {
                    w["path"] = json!(v);
                    let canon = std::fs::canonicalize(v).ok();
                    w["current"] = json!(canon.is_some() && canon == here);
                }
                "HEAD" => w["head"] = json!(v),
                "branch" => w["branch"] = json!(v.strip_prefix("refs/heads/").unwrap_or(v)),
                "detached" => w["detached"] = json!(true),
                "locked" => w["locked"] = json!(true),
                "prunable" => w["prunable"] = json!(true),
                _ => {}
            }
        }
        list.push(w);
    }
    Ok(json!({ "worktrees": list }))
}

/// A git call that talks to a remote: never prompts (there's no terminal to
/// answer in), gives up after two minutes.
fn net(root: &Path, args: &[&str]) -> R {
    run(root, args, Duration::from_secs(120)).map(|o| json!({ "ok": true, "output": o.trim() }))
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    run(root, args, Duration::from_secs(30))
}

fn run(root: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut child = Command::new("git")
        .args(["-c", "core.quotepath=false", "-c", "color.ui=never"])
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run git: {e}"))?;
    // Read both pipes as they fill, so a chatty command never blocks on them.
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut o = Vec::new();
        if let Some(s) = out.as_mut() {
            let _ = s.read_to_end(&mut o);
        }
        o
    });
    let err_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut e = Vec::new();
        if let Some(s) = err.as_mut() {
            let _ = s.read_to_end(&mut e);
        }
        e
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git {} took too long and was stopped", args.first().unwrap_or(&"")));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    if status.success() {
        // Push and pull say what they did on stderr.
        Ok(if stdout.trim().is_empty() { stderr } else { stdout })
    } else {
        let msg = if stderr.trim().is_empty() { stdout } else { stderr };
        Err(msg.trim().trim_start_matches("fatal: ").trim_start_matches("error: ").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_project() {
        let root = Path::new("/w/p");
        assert_eq!(inside(root, "src/a.rs").unwrap(), PathBuf::from("/w/p/src/a.rs"));
        assert_eq!(inside(root, "./src").unwrap(), PathBuf::from("/w/p/src"));
        assert!(inside(root, "../etc/passwd").is_err());
        assert!(inside(root, "src/../../x").is_err());
        assert!(inside(root, "/etc/passwd").is_err());
    }

    #[test]
    fn status_parses_porcelain_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaa bbb src/a.rs\0\
1 A. N... 000000 100644 100644 000 ccc new file.rs\0\
2 R. N... 100644 100644 100644 ddd eee R100 lib/b.rs\0lib/old.rs\0\
? notes.txt\0";
        let v = parse_status(out, "");
        assert_eq!(v["branch"], "main");
        assert_eq!(v["upstream"], "origin/main");
        assert_eq!((v["ahead"].as_i64(), v["behind"].as_i64()), (Some(2), Some(1)));
        let f = v["files"].as_array().unwrap();
        assert_eq!(f[0]["path"], "src/a.rs");
        assert_eq!((f[0]["staged"].as_str(), f[0]["unstaged"].as_str()), (Some(""), Some("M")));
        assert_eq!(f[1]["path"], "new file.rs");
        assert_eq!(f[1]["staged"], "A");
        assert_eq!((f[2]["path"].as_str(), f[2]["orig"].as_str()), (Some("lib/b.rs"), Some("lib/old.rs")));
        assert_eq!(f[3]["unstaged"], "?");
        // From a folder inside the repository, paths are from that folder.
        let sub = parse_status("1 .M N... 1 1 1 a b app/src/x.rs\0? docs/y.md\0", "app/");
        assert_eq!(sub["files"].as_array().unwrap().len(), 1);
        assert_eq!(sub["files"][0]["path"], "src/x.rs");
    }

    /// A real repository: a branch merged back, a worktree, and the calls
    /// the History and Worktrees views make.
    #[test]
    fn history_and_worktrees_from_a_real_repo() {
        let tmp = std::env::temp_dir().join(format!("deck-code-{}", std::process::id()));
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sh = |cmd: &str| {
            let ok = Command::new("sh")
                .args(["-c", cmd])
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "Maya")
                .env("GIT_AUTHOR_EMAIL", "maya@example.com")
                .env("GIT_COMMITTER_NAME", "Maya")
                .env("GIT_COMMITTER_EMAIL", "maya@example.com")
                .output()
                .unwrap();
            assert!(ok.status.success(), "{cmd}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        sh("git init -q -b main && echo one > a.txt && git add a.txt && git commit -qm first");
        sh("git switch -qc feature && echo two >> a.txt && echo new > b.txt && git add . && git commit -qm 'feature work'");
        sh("git switch -q main && echo side > c.txt && git add c.txt && git commit -qm 'main side' && git merge -q --no-edit --no-ff feature");
        let ed = Editor { root: repo.clone(), open: None, view: "files", commit: None };
        let call = |op: &str, args: Value| api(&ed, op, &args).unwrap();

        let log = call("log", json!({ "limit": 10 }));
        let commits = log["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 4);
        assert_eq!(commits[0]["parents"].as_array().unwrap().len(), 2, "the merge is newest");
        assert!(commits[0]["refs"].as_array().unwrap().iter().any(|r| r == "HEAD -> main"));
        assert_eq!(commits.last().unwrap()["subject"], "first");

        let feature = commits.iter().find(|c| c["subject"] == "feature work").unwrap();
        let shown = call("show", json!({ "hash": feature["hash"] }));
        let files = shown["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        let a_txt = files.iter().find(|f| f["path"] == "a.txt").unwrap();
        assert_eq!((a_txt["status"].as_str(), a_txt["added"].as_i64()), (Some("M"), Some(1)));
        assert_eq!(files.iter().find(|f| f["path"] == "b.txt").unwrap()["status"], "A");
        let diff = call("commit_file", json!({ "hash": feature["hash"], "path": "a.txt" }));
        assert_eq!((diff["before"].as_str(), diff["after"].as_str()), (Some("one\n"), Some("one\ntwo\n")));
        assert!(api(&ed, "show", &json!({ "hash": "--all" })).is_err());

        let branches = call("branches", json!({}));
        let f = branches["branches"].as_array().unwrap().iter().find(|b| b["name"] == "feature").unwrap();
        assert_eq!(f["merged"], true);
        assert_eq!(f["remote"], false);

        let added = call("worktree_add", json!({ "branch": "hotfix", "create": true, "path": "" }));
        let wt = PathBuf::from(added["path"].as_str().unwrap());
        assert!(wt.join("a.txt").exists(), "{added}");
        let list = call("worktrees", json!({}));
        let list = list["worktrees"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|w| w["current"] == true && w["branch"] == "main"));
        assert!(list.iter().any(|w| w["branch"] == "hotfix" && w["current"] == false));
        call("worktree_remove", json!({ "path": wt }));
        assert!(!wt.exists());
        call("delete_branch", json!({ "name": "hotfix" }));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(tmp.parent().unwrap().join("repo-hotfix"));
    }

    #[test]
    fn upstream_tracking_reads() {
        assert_eq!(track("[ahead 2, behind 1]"), (2, 1));
        assert_eq!(track("[behind 3]"), (0, 3));
        assert_eq!(track(""), (0, 0));
    }

    #[test]
    fn editor_urls_round_trip() {
        let dir = std::env::temp_dir();
        let e = Editor { root: dir.clone(), open: None, view: "changes", commit: None };
        let back = Editor::from_url(&e.url()).unwrap();
        assert_eq!(back.root, dir);
        assert_eq!(back.view, "changes");
        assert!(Editor::from_url("https://example.com/?root=/").is_none());
        assert_eq!(decode_query("a%20b+c%2Fd"), "a b c/d");
    }
}
