//! Snippets: messages you keep to put in again, into a chat's composer or
//! a terminal's prompt. ⌘; lists them, ⌘K finds them by name, and Settings
//! keeps them. They live in one JSON file beside the settings, read fresh
//! each time they are listed, so a change made anywhere shows everywhere.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub body: String,
}

pub fn path() -> PathBuf {
    crate::control::socket_dir().join("snippets.json")
}

/// The saved snippets. A first run gets a few to show what they are for.
pub fn load() -> Vec<Snippet> {
    match std::fs::read(path()) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => starters(),
    }
}

pub fn save(all: &[Snippet]) {
    let target = path();
    let tmp = target.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&tmp, serde_json::to_vec_pretty(all)?)?;
        std::fs::rename(&tmp, &target)
    };
    if let Err(e) = write() {
        tracing::warn!("could not save snippets: {e}");
    }
}

/// Keep `body` as a new snippet, named after its first line. Returns the
/// name it got.
pub fn add(body: &str) -> String {
    let mut all = load();
    let name = name_for(body, &all);
    all.push(Snippet {
        name: name.clone(),
        body: body.trim_end().to_string(),
    });
    save(&all);
    name
}

/// The first line, cut to a few words, made unique among `all`.
fn name_for(body: &str, all: &[Snippet]) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Snippet");
    let mut base = String::new();
    for w in line.split_whitespace() {
        if !base.is_empty() && base.chars().count() + w.chars().count() > 40 {
            base.push('…');
            break;
        }
        if !base.is_empty() {
            base.push(' ');
        }
        base.push_str(w);
    }
    let mut name = base.clone();
    let mut n = 2;
    while all.iter().any(|s| s.name == name) {
        name = format!("{base} {n}");
        n += 1;
    }
    name
}

/// The text to put in: `{clipboard}` becomes what is on the clipboard.
pub fn expand(body: &str, clipboard: impl FnOnce() -> Option<String>) -> String {
    if body.contains("{clipboard}") {
        body.replace("{clipboard}", &clipboard().unwrap_or_default())
    } else {
        body.to_string()
    }
}

/// The body's first line, for a one-line preview.
pub fn preview(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if body.trim().lines().count() > 1 {
        format!("{line} …")
    } else {
        line.to_string()
    }
}

fn starters() -> Vec<Snippet> {
    [
        (
            "Review my changes",
            "Review the uncommitted changes for bugs, missed edge cases and anything that does not match the code around it. List what you find, most serious first, before changing anything.",
        ),
        (
            "Run the tests",
            "Run the tests. If any fail, find out why and fix the cause, then run them again until they pass.",
        ),
        (
            "Commit",
            "Commit the changes with a clear message that says what changed and why.",
        ),
        (
            "Explain this error",
            "Explain this error and fix it:\n\n{clipboard}",
        ),
    ]
    .into_iter()
    .map(|(name, body)| Snippet {
        name: name.into(),
        body: body.into(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_first_line_and_stay_unique() {
        let all = vec![Snippet {
            name: "Fix the bug".into(),
            body: String::new(),
        }];
        assert_eq!(name_for("\n  Fix the bug\nmore", &all), "Fix the bug 2");
        assert_eq!(name_for("", &[]), "Snippet");
        let long = name_for(
            "please go through every file in the project and tidy the imports",
            &[],
        );
        assert!(long.ends_with('…') && long.chars().count() <= 42, "{long}");
    }

    #[test]
    fn the_clipboard_goes_in_only_when_asked_for() {
        assert_eq!(
            expand("why: {clipboard}", || Some("E42".into())),
            "why: E42"
        );
        assert_eq!(expand("plain", || panic!("not read")), "plain");
        assert_eq!(preview("one\ntwo"), "one …");
        assert_eq!(preview("  one  "), "one");
    }
}
