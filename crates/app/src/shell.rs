//! Shell integration: zsh in Deck's terminals marks each prompt and command
//! (OSC 133, plus VS Code's 633;E for the command line) and reports its
//! folder (OSC 7). The terminal turns those marks into blocks: one per
//! command, with its exit status and how long it took (see `term`'s tap).
//!
//! It loads the way Ghostty's and kitty's do, without touching your files:
//! the shell starts with `ZDOTDIR` pointing at a folder of Deck's whose
//! `.zshenv` puts your `ZDOTDIR` back, runs your `.zshenv`, then adds two
//! hooks. Everything else (your `.zprofile`, `.zshrc`, prompt, plugins)
//! loads from your own folder as usual.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const ZSHENV: &str = r#"# Deck's shell integration. Your own ZDOTDIR comes back first, so your
# .zprofile, .zshrc and the rest load from where they always do.
if [[ -n "${DECK_ZDOTDIR+x}" ]]; then
  ZDOTDIR="$DECK_ZDOTDIR"
  unset DECK_ZDOTDIR
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
if [[ -o interactive && -z "${DECK_NO_INTEGRATION-}" ]]; then
  builtin source "${${(%):-%x}:A:h}/deck.zsh"
fi
"#;

const DECK_ZSH: &str = r##"# Marks each prompt and command for Deck (OSC 133, 633;E) and reports the
# folder (OSC 7), so Deck can show every command as a block.
[[ -n "${_deck_marks-}" ]] && return
typeset -g _deck_marks=1 _deck_ran=0

# After a relaunch: what this pane showed before, above the new prompt.
if [[ -n "${DECK_RESTORE-}" && -f "$DECK_RESTORE" ]]; then
  command cat -- "$DECK_RESTORE"
  command rm -f -- "$DECK_RESTORE"
fi
unset DECK_RESTORE

_deck_precmd() {
  local code=$?
  if (( _deck_ran )); then
    builtin printf '\e]133;D;%s\a' "$code"
    _deck_ran=0
  fi
  builtin printf '\e]7;file://%s%s\a' "${HOST}" "${PWD// /%20}"
  builtin printf '\e]133;A\a'
  # A fresh line: ^C ends the last one without line-finish.
  _deck_line_last=
  builtin printf '\e]6973;line;;\a'
  _deck_bind
}

_deck_preexec() {
  _deck_ran=1
  local cmd="$1"
  # Escapes and bells would end the mark early; backslashes are escaped.
  cmd="${cmd//\\/\\\\}"
  cmd="${cmd//$'\e'/}"
  cmd="${cmd//$'\a'/}"
  cmd="${cmd//$'\n'/\\x0a}"
  builtin printf '\e]633;E;%s\a' "${cmd[1,4000]}"
  builtin printf '\e]133;C\a'
}

# "# what you want" at the prompt, then ⏎: Deck writes the command and
# types it at the prompt for you to read and run. Any other line goes on as
# usual (through an accept-line of your own, if you have one).
_deck_accept_line() {
  if [[ $BUFFER == '# '* ]]; then
    local ask=${BUFFER#\# }
    ask=${ask//$'\e'/}
    ask=${ask//$'\a'/}
    ask=${ask//$'\n'/ }
    builtin print -s -- "$BUFFER"
    builtin printf '\e]6973;ask;%s\a' "$ask"
    BUFFER=""
    zle -M "Deck is writing that command…"
    return
  fi
  if (( ${+widgets[_deck_prev_accept_line]} )); then
    zle _deck_prev_accept_line
  else
    zle .accept-line
  fi
}

# Suggestions as you type: the newest command in your history that starts
# with what you've typed, in grey after the cursor; → or End takes it.
# Left to zsh-autosuggestions when you have it (it does the same).
_deck_suggest() {
  region_highlight=("${(@)region_highlight:#*memo=deck}")
  POSTDISPLAY=
  (( CURSOR == ${#BUFFER} )) || return
  [[ -z $BUFFER || $BUFFER == [[:space:]]* || $BUFFER == '# '* ]] && return
  local match=${history[(r)${(b)BUFFER}*]}
  [[ -n $match && $match != "$BUFFER" && $match != *$'\n'* ]] || return
  POSTDISPLAY=${match#"$BUFFER"}
  region_highlight+=("${#BUFFER} $(( ${#BUFFER} + ${#POSTDISPLAY} )) fg=8,memo=deck")
}

_deck_unsuggest() {
  region_highlight=("${(@)region_highlight:#*memo=deck}")
  POSTDISPLAY=
}

_deck_take() {
  if [[ -n $POSTDISPLAY ]] && (( CURSOR == ${#BUFFER} )); then
    BUFFER+=$POSTDISPLAY
    CURSOR=${#BUFFER}
    _deck_unsuggest
  else
    zle ".$WIDGET"
  fi
}

# The line as you edit it, for Deck's completion menu: sent when it
# changes, and once more, empty, when it is entered or abandoned.
typeset -g _deck_line_last=
_deck_line() {
  local now="$CURSOR;$BUFFER"
  [[ $now == "$_deck_line_last" ]] && return
  _deck_line_last=$now
  local b=${BUFFER[1,2000]}
  b=${b//\\/\\\\}
  b=${b//$'\e'/}
  b=${b//$'\a'/}
  b=${b//$'\n'/\\x0a}
  builtin printf '\e]6973;line;%s;%s\a' "$CURSOR" "$b" >/dev/tty
}

_deck_line_done() {
  _deck_line_last=
  builtin printf '\e]6973;line;;\a' >/dev/tty
}

_deck_bind() {
  (( _deck_bound )) && return
  typeset -g _deck_bound=1
  zmodload zsh/zleparameter 2>/dev/null
  zmodload zsh/parameter 2>/dev/null
  [[ ${widgets[accept-line]-} == user:* ]] && zle -A accept-line _deck_prev_accept_line
  zle -N accept-line _deck_accept_line
  autoload -Uz add-zle-hook-widget
  if [[ -z ${DECK_NO_COMPLETE-} ]]; then
    add-zle-hook-widget line-pre-redraw _deck_line
    add-zle-hook-widget line-finish _deck_line_done
  fi
  if (( ! ${+functions[_zsh_autosuggest_start]} )) && [[ -z ${DECK_NO_SUGGEST-} ]]; then
    add-zle-hook-widget line-pre-redraw _deck_suggest
    add-zle-hook-widget line-finish _deck_unsuggest
    zle -N forward-char _deck_take
    zle -N end-of-line _deck_take
  fi
}

# First in line, so $? is still the command's when ours runs.
typeset -ga precmd_functions preexec_functions
precmd_functions=(_deck_precmd ${precmd_functions[@]:#_deck_precmd})
preexec_functions=(${preexec_functions[@]:#_deck_preexec} _deck_preexec)
"##;

/// Where the integration's files live: beside Deck's other data.
fn dir() -> PathBuf {
    crate::control::socket_dir().join("shell").join("zsh")
}

/// Write the files if they are missing or from another version.
fn install() -> std::io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    for (name, text) in [(".zshenv", ZSHENV), ("deck.zsh", DECK_ZSH)] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(dir)
}

/// The environment that loads the integration into a terminal's shell:
/// nothing for a shell other than zsh, or when it cannot be installed.
pub fn env() -> HashMap<String, String> {
    use std::sync::OnceLock;
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    let mut env = HashMap::new();
    let shell = std::env::var("SHELL").unwrap_or_default();
    if !shell.ends_with("/zsh") {
        return env;
    }
    let Some(dir) = DIR
        .get_or_init(|| install().map_err(|e| tracing::warn!("shell integration: {e}")).ok())
        .clone()
    else {
        return env;
    };
    if let Some(own) = std::env::var_os("ZDOTDIR") {
        env.insert("DECK_ZDOTDIR".into(), own.to_string_lossy().into_owned());
    }
    env.insert("ZDOTDIR".into(), dir.display().to_string());
    env
}

/// The skill that teaches a coding agent in Deck to run commands and use
/// the browser where the user can see them. The marker line says Deck owns
/// a copy; once someone edits it out, Deck leaves that copy alone.
const SKILL_MARK: &str = "<!-- managed by Deck: edits are kept if you delete this line -->";
const SKILL: &str = r##"---
name: deck
description: Use when running inside Deck (DECK_SOCKET is set). Run dev servers, watchers, tests and other long commands in a terminal pane the user watches live, read their logs and wait for them; open web pages in Deck's browser beside you or in a tab, then read, click, type, screenshot and debug them through their console, errors and network requests.
---
<!-- managed by Deck: edits are kept if you delete this line -->

# Working in Deck

You are inside Deck, a Mac workspace with terminals, a browser and agent
chats in one window. The `deck` command drives it, and the user sees
everything you do there as it happens. Output is JSON.

## Long-running commands: a shell you both watch

Don't background processes yourself (`cmd &`, `nohup`): the output is hidden
from the user and the process outlives you. Run them in a pane:

```sh
deck run --name dev npm run dev           # a terminal split beside you, running it
deck run --name test --dir down cargo test
deck run --name dev npm run dev           # again: Ctrl-C, then rerun in the same pane
deck pane wait --name dev --for "ready|Local:|listening" --timeout 90
deck pane read --name dev --lines 80      # the last 80 lines of its output
deck pane send --name dev "rs"            # type into it (Enter after)
deck pane close --name dev
```

`pane wait` exits 0 with the matching line, or 1 with the pane's last lines
when the text never shows up; look for the error there. Read logs with
`--lines`, not the whole scrollback. Check `deck pane list` before starting a
second dev server; if one is running, read its logs instead.

## The browser

```sh
deck open --name app http://localhost:3000   # beside you; the same name reuses that pane
deck open --tab https://example.com          # in a new tab instead
deck browser snapshot --name app             # title, headings and numbered elements
deck browser click --name app 12             # by number from the snapshot
deck browser click --name app "text=Sign in" # or by text, or a CSS selector
deck browser type --name app 4 "me@example.com" --enter
deck browser key --name app Escape
deck browser wait --name app --for "Dashboard|Welcome" --timeout 20
deck browser wait --name app --selector .spinner --gone
deck browser scroll --name app bottom        # up, down, top, or text=Pricing
deck browser reload --name app               # also back, forward; they wait for the load
deck browser dialog --name app accept        # answer an alert/confirm/prompt or a login
deck browser eval --name app "document.querySelectorAll('li').length"
deck pane read --name app                    # the page's text
```

With one page open, or one in your own tab, `--name` can be left out.

Take a fresh `snapshot` after the page changes: element numbers are only
good until the next one.

## Debugging a page

```sh
deck browser errors --name app               # uncaught errors, console.error, failed requests
deck browser console --name app --since 120  # console output after entry 120
deck browser network --name app --failed     # requests that failed or got 4xx/5xx
deck browser network --name app --filter /api/
```

Network entries carry method, URL, status, timing and the first part of the
request and response bodies for fetch and XHR. Every entry has a `seq`: pass
the last one you saw as `--since` to get only what is new after you act.

## Seeing it

```sh
deck browser shot --name app shot.png        # a PNG of just that page
deck window shot shot.png                    # the whole Deck window
```

Open the PNG with your image reading tool to check layout and styling.

## Good habits

- Tell the user which pane you started (`dev`, `app`) so they can watch it.
- Fix what `errors` and `network --failed` show before saying a page works.
- After a change, reload, then check `errors` again with `--since`.
- Close panes you opened just for a quick check; leave dev servers running.
"##;

/// Where agents look for skills: Claude Code's folder (OpenCode, Cursor and
/// Goose read it too), the shared `~/.agents/skills` (Codex, Gemini CLI,
/// OpenCode, Cline, Warp, Zed…), and the own folder of each agent that reads
/// neither, for the ones installed (their home folder exists).
fn skill_dirs(home: &Path) -> Vec<PathBuf> {
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
    let mut out = Vec::new();
    if claude.is_dir() {
        out.push(claude.join("skills"));
    }
    let shared_users = [".agents", ".codex", ".gemini", ".config/opencode", ".cline", ".pi"];
    if shared_users.iter().any(|d| home.join(d).is_dir()) {
        out.push(home.join(".agents/skills"));
    }
    // (an agent's home, its skills folder)
    let own: &[(&str, &str)] = &[
        (".cursor", ".cursor/skills"),
        (".grok", ".grok/skills"),
        (".config/goose", ".config/goose/skills"),
        (".vibe", ".vibe/skills"),
        (".copilot", ".copilot/skills"),
        (".config/agents", ".config/agents/skills"),
        (".factory", ".factory/skills"),
        (".kilo", ".kilo/skills"),
        (".kiro", ".kiro/skills"),
        (".qwen", ".qwen/skills"),
        (".config/crush", ".config/crush/skills"),
        (".roo", ".roo/skills"),
        (".continue", ".continue/skills"),
        (".augment", ".augment/skills"),
        (".junie", ".junie/skills"),
        (".openhands", ".openhands/skills"),
        (".codebuddy", ".codebuddy/skills"),
        (".iflow", ".iflow/skills"),
    ];
    out.extend(own.iter().filter(|(h, _)| home.join(h).is_dir()).map(|(_, d)| home.join(d)));
    out
}

/// Put Deck's skill in every agent's skills folder (`on`), or take Deck's
/// copies out again. A copy without the marker is the user's and is kept.
pub fn install_skill(on: bool) {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        place_skill(&skill_dirs(&home), on);
    }
}

fn place_skill(dirs: &[PathBuf], on: bool) {
    for dir in dirs {
        let path = dir.join("deck").join("SKILL.md");
        let now = std::fs::read_to_string(&path).ok();
        if now.as_deref().is_some_and(|n| !n.contains(SKILL_MARK)) {
            continue;
        }
        if !on {
            if now.is_some() {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir(dir.join("deck"));
            }
            continue;
        }
        if now.as_deref() == Some(SKILL) {
            continue;
        }
        let _ = std::fs::create_dir_all(dir.join("deck"));
        if let Err(e) = std::fs::write(&path, SKILL) {
            tracing::warn!("deck skill in {}: {e}", dir.display());
        }
    }
}

/// zsh's history, newest first, with when each ran (extended history), at
/// most `limit` distinct commands. zsh writes non-ASCII "metafied" (0x83,
/// then the byte xor 32) and ends a line that continues with `\`.
pub fn zsh_history(limit: usize) -> Vec<(String, Option<i64>)> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let file = std::env::var_os("HISTFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("DECK_ZDOTDIR").map(|d| PathBuf::from(d).join(".zsh_history")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| home.join(".zsh_history"));
    let Ok(raw) = std::fs::read(file) else {
        return Vec::new();
    };
    parse_zsh_history(&raw, limit)
}

fn parse_zsh_history(raw: &[u8], limit: usize) -> Vec<(String, Option<i64>)> {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut it = raw.iter();
    while let Some(&b) = it.next() {
        if b == 0x83 {
            if let Some(&n) = it.next() {
                bytes.push(n ^ 32);
            }
        } else {
            bytes.push(b);
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut entries: Vec<(String, Option<i64>)> = Vec::new();
    let mut pending = String::new();
    for line in text.split('\n') {
        if let Some(cont) = line.strip_suffix('\\') {
            pending.push_str(cont);
            pending.push('\n');
            continue;
        }
        pending.push_str(line);
        let entry = std::mem::take(&mut pending);
        // ": 1700000000:0;command"
        let (when, cmd) = match entry.strip_prefix(": ").and_then(|r| r.split_once(';')) {
            Some((meta, cmd)) => (meta.split(':').next().and_then(|t| t.parse().ok()), cmd.to_string()),
            None => (None, entry),
        };
        if !cmd.trim().is_empty() {
            entries.push((cmd, when));
        }
    }
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .rev()
        .filter(|(c, _)| seen.insert(c.clone()))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_carries_its_marker() {
        assert!(SKILL.starts_with("---\nname: deck\n"));
        assert!(SKILL.contains(SKILL_MARK));
    }

    #[test]
    fn the_skill_goes_to_each_installed_agent_and_keeps_edits() {
        let home = std::env::temp_dir().join(format!("deck-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in [".codex", ".grok", ".config/goose"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let dirs = skill_dirs(&home);
        assert!(dirs.contains(&home.join(".agents/skills")));
        assert!(dirs.contains(&home.join(".grok/skills")));
        assert!(dirs.contains(&home.join(".config/goose/skills")));
        assert!(!dirs.contains(&home.join(".cursor/skills")), "not installed");

        place_skill(&dirs, true);
        let grok = home.join(".grok/skills/deck/SKILL.md");
        assert_eq!(std::fs::read_to_string(&grok).unwrap(), SKILL);
        // The user's own version (marker gone) stays through both.
        let agents = home.join(".agents/skills/deck/SKILL.md");
        std::fs::write(&agents, "mine").unwrap();
        place_skill(&dirs, true);
        place_skill(&dirs, false);
        assert_eq!(std::fs::read_to_string(&agents).unwrap(), "mine");
        assert!(!grok.exists() && !home.join(".grok/skills/deck").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn zsh_history_reads_newest_first() {
        let raw = b": 100:0;ls\n: 200:0;git status\n: 300:0;echo a\\\nb\n: 400:0;ls\nplain\n\x83\xe3\x83\x83";
        let h = parse_zsh_history(raw, 10);
        let cmds: Vec<&str> = h.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(cmds[..4], ["\u{e3}", "plain", "ls", "echo a\nb"]);
        assert_eq!(h[2].1, Some(400));
        assert!(cmds.iter().filter(|c| **c == "ls").count() == 1);
    }

    /// A real zsh, through the integration: your .zshenv and .zshrc still
    /// load, and a command comes out marked.
    #[test]
    fn zsh_loads_your_files_and_marks_commands() {
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("deck-shell-{}", std::process::id()));
        let (home, deck) = (tmp.join("home"), tmp.join("deck"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&deck).unwrap();
        std::fs::write(home.join(".zshenv"), "export FROM_ZSHENV=1\n").unwrap();
        std::fs::write(home.join(".zshrc"), "export FROM_ZSHRC=1\n").unwrap();
        std::fs::write(deck.join(".zshenv"), ZSHENV).unwrap();
        std::fs::write(deck.join("deck.zsh"), DECK_ZSH).unwrap();
        let out = std::process::Command::new("/bin/zsh")
            .args(["-i", "-c", "print -n \"$FROM_ZSHENV$FROM_ZSHRC:$ZDOTDIR:\"; _deck_preexec 'echo \\a\\b'; _deck_precmd"])
            .env("HOME", &home)
            .env("ZDOTDIR", &deck)
            .env_remove("DECK_ZDOTDIR")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("11::"), "{text:?}");
        assert!(text.contains("\x1b]633;E;echo \\\\a\\\\b\x07\x1b]133;C\x07"), "{text:?}");
        assert!(text.contains("\x1b]133;D;0\x07"), "{text:?}");
        assert!(text.ends_with("\x1b]133;A\x07\x1b]6973;line;;\x07"), "{text:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
