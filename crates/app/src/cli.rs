//! `deck <group> <verb> [--key value]... [text]`: a thin client over the
//! control socket. Output is JSON so agents can parse it.

use crate::control;
use serde_json::{Map, Value};

const HELP: &str = "\
deck: drive the deck window you are running in.

  deck doctor                       show the PATH and which agent CLIs deck can find
  deck open URL                     open URL in a browser beside this terminal
  deck pane list
  deck pane split [--dir right|down] [--kind terminal|browser] [--cmd CMD] [--url URL] [--pane N]
  deck pane send  --pane N [--no-enter] TEXT     (to a chat pane: sends TEXT as a message)
  deck pane read  --pane N
  deck pane focus --pane N
  deck pane close --pane N
  deck tab new [--kind terminal|browser|chat] [--chat claude|codex|…] [--agent claude|…] [--cmd CMD] [--url URL]
      --chat starts a structured chat with that agent; --agent runs its TUI in a terminal
  deck browser open [--pane N] URL
  deck browser eval --pane N JS

Run it from a terminal inside deck; it reads DECK_SOCKET and DECK_TOKEN.
Output is JSON on stdout; errors go to stderr with exit code 1.
";

pub fn run(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("doctor") {
        return doctor();
    }
    match parse(args) {
        Ok(None) => {
            print!("{HELP}");
            0
        }
        Ok(Some((method, params))) => match control::call(&method, params) {
            Ok(v) => {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
                0
            }
            Err(e) => {
                eprintln!("deck: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("deck: {e}\n\n{HELP}");
            2
        }
    }
}

/// `deck doctor`: what the app would see at launch. The PATH is the one a
/// Finder or Dock launch ends up with, and each agent CLI says whether it
/// is reachable on it.
fn doctor() -> i32 {
    crate::adopt_shell_path();
    let path = std::env::var("PATH").unwrap_or_default();
    println!("PATH {path}");
    let mut found = 0;
    for agent in crate::workspace::AGENTS {
        match term::which(agent.cmd) {
            Some(p) => {
                found += 1;
                println!("  {:<13} {}", agent.cmd, p.display());
            }
            None => println!("  {:<13} not found", agent.cmd),
        }
    }
    if found == 0 {
        eprintln!("deck: no agent CLI is reachable; chats will not start");
        return 1;
    }
    0
}

fn parse(args: &[String]) -> Result<Option<(String, Value)>, String> {
    // `deck open URL` is what $BROWSER runs, so it has to take the URL bare.
    if let [open, url @ ..] = args {
        if open == "open" {
            if url.is_empty() {
                return Err("open needs a URL".into());
            }
            return Ok(Some((
                "browser.open".into(),
                serde_json::json!({ "url": url.join(" ") }),
            )));
        }
    }
    let [group, verb, rest @ ..] = args else {
        return match args.first().map(String::as_str) {
            None | Some("help" | "-h" | "--help") => Ok(None),
            Some(_) => Err("expected `deck <group> <verb>`".into()),
        };
    };
    let method = format!("{group}.{verb}");
    let mut params = Map::new();
    let mut positional: Vec<String> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            positional.extend(it.by_ref().cloned());
            break;
        }
        let Some(key) = a.strip_prefix("--") else {
            positional.push(a.clone());
            continue;
        };
        if let Some(flag) = key.strip_prefix("no-") {
            params.insert(flag.into(), Value::Bool(false));
            continue;
        }
        let v = it.next().ok_or_else(|| format!("--{key} needs a value"))?;
        let v = match key {
            "pane" => Value::from(
                v.parse::<u64>()
                    .map_err(|_| format!("--pane takes a number, got {v:?}"))?,
            ),
            _ => Value::String(v.clone()),
        };
        params.insert(key.into(), v);
    }
    if !positional.is_empty() {
        let key = match method.as_str() {
            "pane.send" => "text",
            "browser.open" => "url",
            "browser.eval" => "js",
            m => return Err(format!("{m} takes no positional arguments")),
        };
        params.insert(key.into(), Value::String(positional.join(" ")));
    }
    Ok(Some((method, Value::Object(params))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(s: &str) -> Result<Option<(String, Value)>, String> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn flags_and_positionals_become_params() {
        assert_eq!(p("").unwrap(), None);
        assert_eq!(
            p("pane list").unwrap(),
            Some(("pane.list".into(), json!({})))
        );
        assert_eq!(
            p("pane send --pane 3 --no-enter ls -la").unwrap(),
            Some((
                "pane.send".into(),
                json!({"pane": 3, "enter": false, "text": "ls -la"})
            ))
        );
        assert_eq!(
            p("pane split --dir down --kind browser --url example.com").unwrap(),
            Some((
                "pane.split".into(),
                json!({"dir": "down", "kind": "browser", "url": "example.com"})
            ))
        );
        assert!(p("pane read --pane x").is_err());
        assert!(p("pane list extra").is_err());
        assert!(p("pane").is_err());
        assert_eq!(
            p("open http://localhost:3000").unwrap(),
            Some((
                "browser.open".into(),
                json!({"url": "http://localhost:3000"})
            ))
        );
        assert_eq!(
            p("tab new --chat opencode").unwrap(),
            Some(("tab.new".into(), json!({"chat": "opencode"})))
        );
        assert_eq!(
            p("tab new --agent claude").unwrap(),
            Some(("tab.new".into(), json!({"agent": "claude"})))
        );
    }
}
