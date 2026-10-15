//! The window's layout, kept across launches: which tabs were open and how
//! each was split, what every pane was (a shell and its folder, an agent and
//! its conversation, a page, a chat thread, Settings), which one had the
//! keyboard, and whether the sidebar and inspector were showing. Written
//! when it changes, read once at launch.
//!
//! Shells come back in their folder; the command that was running is not run
//! again, since nobody wants `rm` replayed. Agents are the exception: an
//! agent CLI comes back with the flags it had and resumes its conversation
//! (Claude Code by session id, Codex and OpenCode by "the last one").

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<TabState>,
    pub active: usize,
    pub sidebar: bool,
    pub inspector: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            sidebar: true,
            inspector: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    #[serde(default)]
    pub pinned: bool,
    /// Which pane had the keyboard: an index into the tab's panes in order.
    #[serde(default)]
    pub focused: usize,
    pub root: NodeState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeState {
    Pane(PaneState),
    Split {
        row: bool,
        children: Vec<NodeState>,
        /// Each child's share, by weight; missing in older files.
        #[serde(default)]
        sizes: Vec<f32>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PaneState {
    Terminal {
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        agent: Option<AgentRun>,
    },
    Browser {
        url: String,
    },
    Chat {
        thread: String,
    },
    Settings,
}

/// An agent CLI that was running in a terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRun {
    /// `claude`, `codex`, …
    pub cmd: String,
    /// The options it was started with, without a prompt or an earlier
    /// resume (see [`keep_flags`]).
    #[serde(default)]
    pub flags: Vec<String>,
    /// Claude Code's conversation, when deck could tell which one it was.
    #[serde(default)]
    pub session: Option<String>,
    /// Started by deck as an agent tab, rather than typed into a shell.
    #[serde(default)]
    pub tab: bool,
}

pub fn path() -> PathBuf {
    crate::control::socket_dir().join("session.json")
}

pub fn load() -> Option<Session> {
    let bytes = std::fs::read(path()).ok()?;
    serde_json::from_slice(&bytes)
        .map_err(|e| tracing::warn!("session.json unreadable, starting fresh: {e}"))
        .ok()
}

/// Written to a temporary file and renamed over the old one, so a crash
/// mid-write never leaves half a layout. Writes are serialised: two at once
/// would race on the temporary file.
pub fn save(session: &Session) {
    static WRITING: Mutex<()> = Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = path();
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(session)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(&tmp, json))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        tracing::warn!("could not save the session: {e}");
    }
}

/// Options that take a value, per agent; anything else that does not start
/// with `-` is a prompt or a subcommand and is left out.
fn valued(cmd: &str) -> &'static [&'static str] {
    match cmd {
        "claude" => &[
            "--model",
            "--permission-mode",
            "--add-dir",
            "--settings",
            "--setting-sources",
            "--mcp-config",
            "--agent",
            "--agents",
            "--fallback-model",
            "--append-system-prompt",
            "--system-prompt",
            "--allowedTools",
            "--allowed-tools",
            "--disallowedTools",
            "--disallowed-tools",
            "--plugin-dir",
        ],
        "codex" => &[
            "-m",
            "--model",
            "-c",
            "--config",
            "-s",
            "--sandbox",
            "-a",
            "--ask-for-approval",
            "-p",
            "--profile",
            "-C",
            "--cd",
        ],
        "opencode" => &["-m", "--model", "--agent"],
        _ => &[],
    }
}

/// Options that pick a conversation or make the run one-shot: dropped,
/// since the restore picks the conversation itself. The value-taking ones
/// take their value with them.
fn dropped(cmd: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match cmd {
        "claude" => (
            &["--continue", "-c", "--fork-session", "--print", "-p"],
            &[
                "--resume",
                "-r",
                "--session-id",
                "--output-format",
                "--input-format",
            ],
        ),
        "codex" => (&["--last"], &[]),
        "opencode" => (&["--continue", "-c"], &["--session", "-s", "--prompt"]),
        _ => (&[], &[]),
    }
}

/// The options worth starting `cmd` with again, from the `argv` it ran with:
/// its options and their values, never its prompt.
pub fn keep_flags(cmd: &str, argv: &[String]) -> Vec<String> {
    let valued = valued(cmd);
    let (drop, drop_valued) = dropped(cmd);
    let mut out = Vec::new();
    let mut args = argv.iter().skip(1);
    while let Some(a) = args.next() {
        let name = a.split('=').next().unwrap_or(a);
        let inline = a.contains('=');
        if drop.contains(&a.as_str()) {
            continue;
        }
        if drop_valued.contains(&name) {
            if !inline {
                args.next();
            }
            continue;
        }
        if !a.starts_with('-') {
            continue;
        }
        out.push(a.clone());
        if valued.contains(&name) && !inline {
            if let Some(v) = args.next() {
                out.push(v.clone());
            }
        }
    }
    out
}

/// The command line that brings an agent back where it was.
pub fn resume_line(run: &AgentRun) -> String {
    let mut words: Vec<String> = vec![run.cmd.clone()];
    match run.cmd.as_str() {
        "claude" => {
            words.extend(run.flags.iter().cloned());
            match &run.session {
                Some(id) => words.extend(["--resume".into(), id.clone()]),
                None => words.push("--continue".into()),
            }
        }
        "codex" => {
            words.extend(["resume".into(), "--last".into()]);
            words.extend(run.flags.iter().cloned());
        }
        "opencode" => {
            words.extend(run.flags.iter().cloned());
            words.push("--continue".into());
        }
        _ => words.extend(run.flags.iter().cloned()),
    }
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// Single-quoted for the shell when it needs to be.
fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Where Claude Code keeps the conversations started in `cwd`: the path
/// with every character that is not a letter or digit turned into `-`.
fn claude_dir(cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".claude/projects")
        .join(slug)
}

/// Claude Code's conversations in `cwd`, the most recently written first.
pub fn claude_sessions(cwd: &Path) -> Vec<String> {
    let mut found: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(claude_dir(cwd))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path
                .file_name()?
                .to_str()?
                .strip_suffix(".jsonl")?
                .to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, id))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &str) -> Vec<String> {
        words.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn flags_keep_options_and_drop_prompts_and_resumes() {
        assert_eq!(
            keep_flags(
                "claude",
                &argv("claude --allow-dangerously-skip-permissions --model opus --resume abc fix the bug")
            ),
            argv("--allow-dangerously-skip-permissions --model opus")
        );
        assert_eq!(
            keep_flags("claude", &argv("claude -c --permission-mode=plan -p hi")),
            argv("--permission-mode=plan")
        );
        assert_eq!(
            keep_flags("codex", &argv("codex -m gpt-5 --yolo resume --last")),
            argv("-m gpt-5 --yolo")
        );
        assert!(keep_flags("gemini", &argv("gemini")).is_empty());
    }

    #[test]
    fn agents_come_back_into_their_conversation() {
        let run = |cmd: &str, flags: &str, session: Option<&str>| AgentRun {
            cmd: cmd.into(),
            flags: if flags.is_empty() {
                vec![]
            } else {
                argv(flags)
            },
            session: session.map(str::to_string),
            tab: false,
        };
        assert_eq!(
            resume_line(&run("claude", "--model opus", Some("1f2e-33"))),
            "claude --model opus --resume 1f2e-33"
        );
        assert_eq!(resume_line(&run("claude", "", None)), "claude --continue");
        assert_eq!(
            resume_line(&run("codex", "--yolo", None)),
            "codex resume --last --yolo"
        );
        assert_eq!(
            resume_line(&run("opencode", "", None)),
            "opencode --continue"
        );
        let mut spaced = run("claude", "", None);
        spaced.flags = vec!["--append-system-prompt".into(), "be brief".into()];
        assert_eq!(
            resume_line(&spaced),
            "claude --append-system-prompt 'be brief' --continue"
        );
    }

    #[test]
    fn claude_project_folders_match_its_naming() {
        let d = claude_dir(Path::new("/Users/me/my.app/src_x"));
        assert!(
            d.ends_with(".claude/projects/-Users-me-my-app-src-x"),
            "{d:?}"
        );
    }

    #[test]
    fn sessions_round_trip_and_old_files_load() {
        let s = Session {
            tabs: vec![TabState {
                pinned: false,
                focused: 1,
                root: NodeState::Split {
                    row: true,
                    sizes: vec![2., 1.],
                    children: vec![
                        NodeState::Pane(PaneState::Terminal {
                            cwd: Some("/tmp".into()),
                            agent: Some(AgentRun {
                                cmd: "claude".into(),
                                flags: vec![],
                                session: Some("abc".into()),
                                tab: true,
                            }),
                        }),
                        NodeState::Pane(PaneState::Browser {
                            url: "https://example.com".into(),
                        }),
                    ],
                },
            }],
            active: 0,
            sidebar: false,
            inspector: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), s);
        let old: Session =
            serde_json::from_str(r#"{"tabs":[{"root":{"pane":{"kind":"settings"}}}]}"#).unwrap();
        assert!(old.sidebar && old.tabs.len() == 1);
    }
}
