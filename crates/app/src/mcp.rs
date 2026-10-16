//! `deck mcp`: deck's control socket as an MCP server on stdin/stdout, so
//! an agent gets deck's commands as tools of its own (open a terminal, read
//! one, open a page, hand out a task, see the window) instead of composing
//! `deck …` shell lines. Deck adds it to every chat's Claude session with
//! that chat's token; any other MCP client can run `deck mcp` from a deck
//! terminal.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// (tool, socket method, what it does, its input schema properties, required)
type Tool = (
    &'static str,
    &'static str,
    &'static str,
    fn() -> Value,
    &'static [&'static str],
);

const TOOLS: &[Tool] = &[
    (
        "deck_panes",
        "pane.list",
        "List deck's open panes (terminals, pages, chats) with their ids, titles, folders and what runs in them.",
        || json!({}),
        &[],
    ),
    (
        "deck_split",
        "pane.split",
        "Open a terminal or a page beside a pane (beside yours when no pane is given). With cmd, the terminal runs it (a dev server, tests).",
        || {
            json!({
                "dir": {"type": "string", "enum": ["right", "down"]},
                "kind": {"type": "string", "enum": ["terminal", "browser"]},
                "cmd": {"type": "string", "description": "command for the new terminal"},
                "url": {"type": "string", "description": "page for a new browser pane"},
                "pane": {"type": "integer", "description": "split beside this pane"}
            })
        },
        &[],
    ),
    (
        "deck_new_tab",
        "tab.new",
        "Open a new tab: a terminal (optionally running cmd), a page, a chat with an agent (chat: claude, codex…), or an agent's own TUI in a terminal (agent).",
        || {
            json!({
                "kind": {"type": "string", "enum": ["terminal", "browser", "chat"]},
                "chat": {"type": "string"}, "agent": {"type": "string"},
                "cmd": {"type": "string"}, "url": {"type": "string"}
            })
        },
        &[],
    ),
    (
        "deck_read",
        "pane.read",
        "Read what a pane shows: a terminal's screen and recent scrollback, a page's text, or a chat's messages.",
        || json!({"pane": {"type": "integer"}}),
        &["pane"],
    ),
    (
        "deck_send",
        "pane.send",
        "Type text into a terminal pane (Enter after it unless enter is false), or send it to a chat pane as a message.",
        || {
            json!({
                "pane": {"type": "integer"},
                "text": {"type": "string"},
                "enter": {"type": "boolean"}
            })
        },
        &["pane", "text"],
    ),
    (
        "deck_focus",
        "pane.focus",
        "Bring a pane to the front.",
        || json!({"pane": {"type": "integer"}}),
        &["pane"],
    ),
    (
        "deck_close",
        "pane.close",
        "Close a pane.",
        || json!({"pane": {"type": "integer"}}),
        &["pane"],
    ),
    (
        "deck_open",
        "browser.open",
        "Open a URL in deck's browser, beside you (or in the given browser pane).",
        || json!({"url": {"type": "string"}, "pane": {"type": "integer"}}),
        &["url"],
    ),
    (
        "deck_eval",
        "browser.eval",
        "Run JavaScript in a browser pane's page and get its result (document.title, the text of an element, whether a selector exists).",
        || json!({"pane": {"type": "integer"}, "js": {"type": "string"}}),
        &["pane", "js"],
    ),
    (
        "deck_task_new",
        "task.new",
        "Hand out a task: a sub-agent on its own branch and worktree of this project, working on the brief. You hear back when it finishes a turn.",
        || {
            json!({
                "text": {"type": "string", "description": "the brief"},
                "agent": {"type": "string"},
                "cwd": {"type": "string"}
            })
        },
        &["text"],
    ),
    (
        "deck_tasks",
        "task.list",
        "List open tasks: what each agent is doing and what it changed.",
        || json!({}),
        &[],
    ),
    (
        "deck_task_show",
        "task.show",
        "A task's status, changes and its agent's last reply.",
        || json!({"task": {"type": "string"}}),
        &["task"],
    ),
    (
        "deck_task_diff",
        "task.diff",
        "A task's changes as a diff against where it started.",
        || json!({"task": {"type": "string"}}),
        &["task"],
    ),
    (
        "deck_task_send",
        "task.send",
        "Send a task's agent a follow-up.",
        || json!({"task": {"type": "string"}, "text": {"type": "string"}}),
        &["task", "text"],
    ),
    (
        "deck_task_merge",
        "task.merge",
        "Squash a task's branch into its base branch.",
        || json!({"task": {"type": "string"}}),
        &["task"],
    ),
    (
        "deck_task_discard",
        "task.discard",
        "Throw a task's branch and worktree away.",
        || json!({"task": {"type": "string"}}),
        &["task"],
    ),
    (
        "deck_screenshot",
        "window.shot",
        "See deck's window as it is now: an image of everything on screen in it, pages included.",
        || json!({}),
        &[],
    ),
];

pub fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = reply(&mut out, Value::Null, Err((-32700, "parse error".into())));
            continue;
        };
        // Notifications (no id) get no answer.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let answer = handle(method, &params);
        if reply(&mut out, id, answer).is_err() {
            break;
        }
    }
    0
}

fn reply(out: &mut impl Write, id: Value, r: Result<Value, (i64, String)>) -> std::io::Result<()> {
    let msg = match r {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    writeln!(out, "{msg}")?;
    out.flush()
}

fn handle(method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "deck", "version": crate::updates::VERSION},
            "instructions": "Tools that drive the deck window this agent runs in: terminals, pages, chats and sub-agent tasks. Pane ids come from deck_panes."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(
            json!({ "tools": TOOLS.iter().map(|(name, _, about, props, req)| json!({
            "name": name,
            "description": about,
            "inputSchema": {"type": "object", "properties": props(), "required": req},
        })).collect::<Vec<_>>() }),
        ),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let Some((_, rpc, ..)) = TOOLS.iter().find(|t| t.0 == name) else {
                return Err((-32602, format!("no tool {name}")));
            };
            Ok(call(rpc, args))
        }
        _ => Err((-32601, format!("no method {method}"))),
    }
}

/// Run one tool through the control socket; a failure is the tool's
/// answer (`isError`), not a protocol error, so the agent can read it.
fn call(rpc: &str, args: Value) -> Value {
    if rpc == "window.shot" {
        return screenshot();
    }
    match crate::control::call(rpc, args) {
        Ok(v) => json!({"content": [{"type": "text", "text": pretty(&v)}]}),
        Err(e) => json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true}),
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// The window as an image, scaled to at most 1600 px so it costs the agent a
/// sensible number of tokens.
fn screenshot() -> Value {
    let path = std::env::temp_dir().join(format!("deck-mcp-shot-{}.png", std::process::id()));
    let shot = crate::control::call("window.shot", json!({ "out": path }));
    if let Err(e) = shot {
        return json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true});
    }
    let _ = std::process::Command::new("/usr/bin/sips")
        .args(["-Z", "1600"])
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let bytes = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    json!({"content": [{"type": "image", "data": base64(&bytes), "mimeType": "image/png"}]})
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// What a chat's agent session is given: `deck mcp` with that chat's socket
/// and token, so its tools act for that chat.
pub fn server_for(env: &[(String, String)]) -> Option<agent_core::McpServerSpec> {
    let exe = std::env::current_exe().ok()?;
    let env: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| k.starts_with("DECK_"))
        .cloned()
        .collect();
    Some(agent_core::McpServerSpec {
        name: "deck".into(),
        transport: agent_core::McpTransport::Stdio {
            command: exe.to_string_lossy().into_owned(),
            args: vec!["mcp".into()],
            env,
        },
        enabled: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaks_mcp() {
        let init = handle("initialize", &json!({"protocolVersion": "2025-03-26"})).unwrap();
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert_eq!(init["serverInfo"]["name"], "deck");
        let list = handle("tools/list", &Value::Null).unwrap();
        let tools = list["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        let read = tools.iter().find(|t| t["name"] == "deck_read").unwrap();
        assert_eq!(read["inputSchema"]["required"], json!(["pane"]));
        assert!(handle("tools/call", &json!({"name": "nope"})).is_err());
        assert_eq!(handle("bogus", &Value::Null).unwrap_err().0, -32601);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
