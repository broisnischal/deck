//! Snippets: messages you keep to put in again, into a chat's composer or
//! a terminal's prompt. ⌘; lists them, ⌘K finds them by name, `;keyword`
//! in a composer expands one, and Settings keeps them. Yours live in one
//! JSON file beside the settings; a project can share its own in
//! `.deck/snippets.json`, committed with the code. Both are read fresh each
//! time they are listed, so a change made anywhere shows everywhere.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub body: String,
    /// Typed after `;` in a chat's composer, followed by a space, it
    /// becomes the body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub keyword: String,
    /// It came from the project's file, not yours.
    #[serde(skip)]
    pub project: bool,
}

/// What `{…}` may stand for in a body.
pub const VARS: [&str; 4] = ["clipboard", "selection", "branch", "folder"];

pub fn path() -> PathBuf {
    crate::control::socket_dir().join("snippets.json")
}

/// A project's shared snippets, in its repository.
pub fn project_path(root: &Path) -> PathBuf {
    root.join(".deck").join("snippets.json")
}

/// Your snippets. A first run gets a few to show what they are for.
pub fn load() -> Vec<Snippet> {
    match std::fs::read(path()) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => starters(),
    }
}

/// Yours, then the project's (when `root` is one and has any).
pub fn load_all(root: Option<&Path>) -> Vec<Snippet> {
    let mut all = load();
    if let Some(b) = root.and_then(|r| std::fs::read(project_path(r)).ok()) {
        match serde_json::from_slice::<Vec<Snippet>>(&b) {
            Ok(shared) => all.extend(shared.into_iter().map(|s| Snippet { project: true, ..s })),
            Err(e) => tracing::warn!("the project's .deck/snippets.json does not parse: {e}"),
        }
    }
    all
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
        keyword: String::new(),
        project: false,
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

/// The text to put in: each `{clipboard}`, `{selection}`, `{branch}` and
/// `{folder}` becomes what `var` gives for it (asked once per name); any
/// other braces stay as written.
pub fn expand(body: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut known: HashMap<&str, String> = HashMap::new();
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let name = tail
            .find('}')
            .map(|j| &tail[1..j])
            .and_then(|n| VARS.iter().find(|v| **v == n).copied());
        match name {
            Some(name) => {
                let value = known
                    .entry(name)
                    .or_insert_with(|| var(name).unwrap_or_default());
                out.push_str(value);
                rest = &tail[name.len() + 2..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `;keyword ` just typed at the end of `before` (the composer's text up to
/// the caret): how many bytes it takes, and the keyword.
pub fn trigger(before: &str) -> Option<(usize, &str)> {
    let typed = before.strip_suffix(' ')?;
    let start = typed
        .rfind(|c: char| c.is_whitespace())
        .map_or(0, |i| i + 1);
    let word = typed[start..].strip_prefix(';')?;
    let ok = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    ok.then_some((before.len() - start, word))
}

/// The branch checked out in `dir`, for `{branch}`.
pub fn branch(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!b.is_empty()).then_some(b)
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

/// What a palette row says under a snippet's name.
pub fn detail(s: &Snippet) -> String {
    let mut d = String::new();
    if s.project {
        d.push_str("project · ");
    }
    if !s.keyword.is_empty() {
        d.push(';');
        d.push_str(&s.keyword);
        d.push_str(" · ");
    }
    d.push_str(&preview(&s.body));
    d
}

fn starters() -> Vec<Snippet> {
    [
        (
            "Review my changes",
            "review",
            "Review the uncommitted changes for bugs, missed edge cases and anything that does not match the code around it. List what you find, most serious first, before changing anything.",
        ),
        (
            "Run the tests",
            "tests",
            "Run the tests. If any fail, find out why and fix the cause, then run them again until they pass.",
        ),
        (
            "Commit",
            "commit",
            "Commit the changes on {branch} with a clear message that says what changed and why.",
        ),
        (
            "Explain this error",
            "explain",
            "Explain this error and fix it:\n\n{clipboard}",
        ),
    ]
    .into_iter()
    .map(|(name, keyword, body)| Snippet {
        name: name.into(),
        body: body.into(),
        keyword: keyword.into(),
        project: false,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str) -> Snippet {
        Snippet {
            name: name.into(),
            body: String::new(),
            keyword: String::new(),
            project: false,
        }
    }

    #[test]
    fn names_come_from_the_first_line_and_stay_unique() {
        let all = vec![snippet("Fix the bug")];
        assert_eq!(name_for("\n  Fix the bug\nmore", &all), "Fix the bug 2");
        assert_eq!(name_for("", &[]), "Snippet");
        let long = name_for(
            "please go through every file in the project and tidy the imports",
            &[],
        );
        assert!(long.ends_with('…') && long.chars().count() <= 42, "{long}");
    }

    #[test]
    fn fill_ins_go_in_only_when_asked_for() {
        let var = |n: &str| match n {
            "clipboard" => Some("E42".to_string()),
            "branch" => Some("main".to_string()),
            _ => None,
        };
        assert_eq!(expand("why: {clipboard}", var), "why: E42");
        assert_eq!(
            expand("on {branch}: {selection}{branch} {json} {", var),
            "on main: main {json} {"
        );
        assert_eq!(expand("plain", |_| panic!("not read")), "plain");
        assert_eq!(preview("one\ntwo"), "one …");
        assert_eq!(preview("  one  "), "one");
    }

    #[test]
    fn a_keyword_fires_on_the_space_after_it() {
        assert_eq!(trigger(";review "), Some((8, "review")));
        assert_eq!(trigger("then ;tests "), Some((7, "tests")));
        assert_eq!(trigger(";review"), None);
        assert_eq!(trigger("a;review "), None);
        assert_eq!(trigger("; "), None);
        assert_eq!(trigger("x ;a.b "), None);
    }

    #[test]
    fn project_snippets_are_marked_and_old_files_still_load() {
        let old: Vec<Snippet> = serde_json::from_str(r#"[{"name":"a","body":"b"}]"#).unwrap();
        assert_eq!(old[0].keyword, "");
        let s = Snippet {
            keyword: "ship".into(),
            project: true,
            ..snippet("Ship it")
        };
        assert_eq!(detail(&s), "project · ;ship · ");
        assert!(!serde_json::to_string(&s).unwrap().contains("project"));
    }
}
