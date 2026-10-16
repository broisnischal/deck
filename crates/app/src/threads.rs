//! Chat threads that outlive a launch: one JSON file per thread beside the
//! control socket, holding the transcript, the unsent draft and its
//! attachments, the ↑ history and the agent's own session id (so Claude can
//! resume losslessly). Closing a thread moves its file to `archive/` rather
//! than deleting it.

use agent_core::Transcript;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub id: String,
    /// The pinned control chat, reopened as the first tab.
    pub control: bool,
    pub agent: String,
    pub cwd: PathBuf,
    pub full_access: bool,
    /// Ask first. Older files have only `full_access`, so this stays apart
    /// from it rather than replacing it.
    pub ask_first: bool,
    pub title: String,
    pub transcript: Transcript,
    pub draft: String,
    pub attachments: Vec<PathBuf>,
    pub history: Vec<String>,
    /// The agent's session id, for `--resume`.
    pub native_id: Option<String>,
    /// How long each finished turn took: (assistant message, millis).
    pub worked: Vec<(usize, u64)>,
    /// Unix millis of the last change; restored threads open oldest first.
    pub updated: i64,
}

pub fn dir() -> PathBuf {
    crate::control::socket_dir().join("threads")
}

/// Where pasted images are kept until the thread is done with them.
pub fn attachments_dir() -> PathBuf {
    crate::control::socket_dir().join("attachments")
}

/// Every saved thread, oldest first. A file that no longer parses is left
/// where it is and skipped, never deleted.
pub fn load_all() -> Vec<Saved> {
    let mut out: Vec<Saved> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(
            |p| match std::fs::read(&p).map(|b| serde_json::from_slice::<Saved>(&b)) {
                Ok(Ok(s)) if !s.id.is_empty() => Some(s),
                Ok(Err(e)) => {
                    tracing::warn!(path = %p.display(), "unreadable thread: {e}");
                    None
                }
                _ => None,
            },
        )
        .collect();
    out.sort_by_key(|s| s.updated);
    out
}

/// Written whole to a temporary file, then renamed over the old one, so a
/// crash mid-write never leaves half a thread.
pub fn write(saved: &Saved) -> std::io::Result<()> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", saved.id));
    let tmp = dir.join(format!(".{}.tmp", saved.id));
    std::fs::write(&tmp, serde_json::to_vec(saved)?)?;
    std::fs::rename(&tmp, &path)
}

pub fn archive(id: &str) {
    let from = dir().join(format!("{id}.json"));
    let to_dir = dir().join("archive");
    if from.exists() && std::fs::create_dir_all(&to_dir).is_ok() {
        if let Err(e) = std::fs::rename(&from, to_dir.join(format!("{id}.json"))) {
            tracing::warn!("could not archive thread {id}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_file_with_missing_fields_still_loads() {
        let s: Saved = serde_json::from_str(r#"{"id":"a","agent":"claude","title":"hi"}"#).unwrap();
        assert_eq!(s.id, "a");
        assert!(s.transcript.messages.is_empty());
        assert!(s.draft.is_empty());
    }

    #[test]
    fn a_saved_thread_round_trips() {
        let mut t = Transcript::default();
        t.push_user("hello".into(), vec![], 1);
        let s = Saved {
            id: "x".into(),
            agent: "claude".into(),
            transcript: t,
            draft: "half".into(),
            ..Default::default()
        };
        let back: Saved = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back.transcript.messages.len(), 1);
        assert_eq!(back.draft, "half");
    }
}
