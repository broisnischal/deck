//! Prompt rewriting: a quick Claude run (Haiku by default) turns what you
//! typed or dictated into the clear, complete prompt you would have written
//! with time to think, using the agent's current task and last reply to work
//! out what "it" and "that" mean.
//!
//! It runs `claude -p` with no tools, no settings and no saved session, so
//! none of your hooks fire and it shows up nowhere as an agent.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Enter rewrites and sends the rewrite.
    #[default]
    Auto,
    /// Enter puts the rewrite in the box; Enter again sends it.
    Review,
    Off,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Auto, Mode::Review, Mode::Off];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "Rewrite and send",
            Mode::Review => "Show first",
            Mode::Off => "Off",
        }
    }
}

static MODE: std::sync::Mutex<(Mode, String)> = std::sync::Mutex::new((Mode::Auto, String::new()));

/// What Settings chose: when to rewrite, and with which model.
pub fn set(mode: Mode, model: &str) {
    *MODE.lock().unwrap_or_else(|e| e.into_inner()) = (mode, model.to_string());
}

pub fn mode() -> Mode {
    MODE.lock().unwrap_or_else(|e| e.into_inner()).0
}

pub fn model() -> String {
    let m = MODE.lock().unwrap_or_else(|e| e.into_inner()).1.clone();
    if m.is_empty() {
        "haiku".into()
    } else {
        m
    }
}

/// The models it may use, by Claude Code's own aliases.
pub const MODELS: [(&str, &str); 3] = [("haiku", "Haiku"), ("sonnet", "Sonnet"), ("opus", "Opus")];

/// It may only restate what you said or what the context holds: a confident
/// rewrite that invents a cause or a file sends the agent the wrong way,
/// which is worse than a vague message.
const SYSTEM: &str = "I type or dictate quick, rough messages to a coding agent working in deck, my workspace of terminals, a browser and agents. Rewrite my message into the prompt I would have written with time to think: clear, specific and complete, so the agent gets it right the first time.

Rules:
- Keep my intent, scope and every concrete detail (names, paths, numbers, error text, URLs, code). Never add requirements, files, causes, libraries or other technical facts that are not in my message or the context. Where something is unknown, tell the agent to find out (read the code, check, ask me) instead of guessing.
- Fix speech to text mistakes, typos and grammar, using the context to work out what I meant. Write in English.
- Write as me, to the agent: first person, imperative (\"Fix\", \"Check\", \"Don't\"). Plain ASCII punctuation, no em dashes.
- Size it to the task. A reply or follow-up in an ongoing conversation (\"yes do that\", \"try again\", an answer to its question) keeps its length: fix the wording and resolve references, add nothing, since the agent already has the whole conversation. \"yes go ahead and commit it\" becomes \"Yes, go ahead and commit it.\" A real task gets the goal, the context that matters, what to do and what to leave alone, and how to tell it is done (what to run or check, what to report back). Use a numbered list only for real steps.
- Use the agent's current task and last reply to resolve \"it\", \"that\" and \"the thing\". Don't repeat what it already knows beyond what the message needs.
- Keep @file mentions exactly as written, where they fit in the text.
- Don't do the task, answer it, or ask me anything. Output only the rewritten prompt, with no preamble, quotes or code fences around it.";

/// What the agent is in the middle of, to resolve "it" and "that" with.
pub struct Context {
    pub agent: String,
    pub folder: String,
    pub task: String,
    pub last_reply: String,
}

/// Whether a message is worth rewriting: not a slash command or a shell
/// escape, and more than a short reply like "yes do it".
pub fn needed(text: &str) -> bool {
    let (_, body) = split_mentions(text);
    if body.is_empty() || body.starts_with('/') || body.starts_with('!') {
        return false;
    }
    body.contains('\n') || body.split_whitespace().count() > 4
}

/// Leading `@file` mentions ride along untouched.
fn split_mentions(text: &str) -> (String, String) {
    let text = text.trim();
    let mut mentions = String::new();
    let mut rest = text;
    while let Some(m) = rest.strip_prefix('@') {
        let end = m.find(char::is_whitespace).unwrap_or(m.len());
        mentions.push('@');
        mentions.push_str(&m[..end]);
        mentions.push(' ');
        rest = m[end..].trim_start();
    }
    (mentions, rest.trim().to_string())
}

fn input(body: &str, ctx: &Context) -> String {
    let mut b = String::from("<agent>\n");
    b.push_str(&format!("name: {}\nfolder: {}\n", ctx.agent, ctx.folder));
    if !ctx.task.is_empty() {
        b.push_str(&format!("current task: {}\n", clip(&ctx.task, 1200)));
    }
    if !ctx.last_reply.is_empty() {
        b.push_str(&format!(
            "its last reply (end):\n{}\n",
            tail(&ctx.last_reply, 3000)
        ));
    }
    b.push_str("</agent>\n\n");
    b.push_str(&format!("<message>\n{body}\n</message>"));
    b
}

/// Rewrite `text` with `model`. Blocking; run it off the main thread.
pub fn rewrite(text: &str, ctx: &Context, model: &str, cwd: &Path) -> Result<String, String> {
    let (mentions, body) = split_mentions(text);
    if body.is_empty() {
        return Ok(text.to_string());
    }
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "--model",
        model,
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--output-format",
        "stream-json",
        "--verbose",
        "--system-prompt",
        SYSTEM,
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    if cwd.is_dir() {
        cmd.current_dir(cwd);
    }
    // Keep it out of any agent session and out of deck's control plane.
    for (k, _) in std::env::vars() {
        if (k.starts_with("CLAUDE") && k != "CLAUDE_CONFIG_DIR") || k.starts_with("DECK_") {
            cmd.env_remove(k);
        }
    }
    let mut child = cmd.spawn().map_err(|e| format!("rewrite failed: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input(&body, ctx).as_bytes());
    }
    let started = Instant::now();
    let stdout = child.stdout.take().ok_or("rewrite failed")?;
    // The result comes about a second before claude exits: read it off the
    // stream and stop the process instead of waiting for its shutdown.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v.get("type").and_then(|t| t.as_str()) == Some("result") {
                    let _ = tx.send(v);
                    return;
                }
            }
        }
    });
    let found = rx.recv_timeout(Duration::from_secs(90).saturating_sub(started.elapsed()));
    let _ = child.kill();
    let _ = child.wait();
    let v = found.map_err(|_| "rewrite timed out".to_string())?;
    let result = v.get("result").and_then(|r| r.as_str()).unwrap_or_default();
    if v.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
        return Err(format!("rewrite failed: {}", clip(result, 120)));
    }
    let refined = clean(result);
    if refined.is_empty() {
        return Err("rewrite came back empty".into());
    }
    Ok(format!("{mentions}{refined}"))
}

/// Drop a code fence or quotes the model wrapped the prompt in.
fn clean(s: &str) -> String {
    let mut s = s.trim().to_string();
    if s.starts_with("```") && s.ends_with("```") && s.len() > 6 {
        let inner = s[3..s.len() - 3].trim().to_string();
        s = match inner.split_once('\n') {
            // The fence's language tag.
            Some((first, rest)) if !first.contains(' ') => rest.trim().to_string(),
            _ => inner,
        };
    }
    if s.len() > 1 && s.starts_with('"') && s.ends_with('"') && !s[1..s.len() - 1].contains('"') {
        s = s[1..s.len() - 1].to_string();
    }
    s.trim().to_string()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The last `n` bytes of `s`, cut at a line start when one is close.
fn tail(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut at = s.len() - n;
    while !s.is_char_boundary(at) {
        at += 1;
    }
    let t = &s[at..];
    match t.find('\n') {
        Some(i) if i < 200 => t[i + 1..].to_string(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_replies_and_commands_go_as_typed() {
        assert!(!needed("yes do it"));
        assert!(!needed("/compact"));
        assert!(!needed("! ls -la and more words here"));
        assert!(!needed("@src/a.rs fix it"));
        assert!(needed("can you make the sidebar rows line up better"));
        assert!(needed("fix\nthis"));
    }

    #[test]
    fn mentions_ride_along_and_wrapping_comes_off() {
        assert_eq!(
            split_mentions("@a.rs @b.rs make it faster"),
            ("@a.rs @b.rs ".to_string(), "make it faster".to_string())
        );
        assert_eq!(clean("```text\nFix the bug.\n```"), "Fix the bug.");
        assert_eq!(clean("\"Fix the bug.\""), "Fix the bug.");
        assert_eq!(clean("  Fix \"this\" one. "), "Fix \"this\" one.");
        assert_eq!(tail("abc\ndef", 100), "abc\ndef");
    }
}
