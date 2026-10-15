//! Decoders for the CLI backends.

use agent_core::{AgentEvent, StopReason, ToolId, ToolOutput, Usage};
use serde_json::Value;
use std::collections::HashMap;

/// Decoder for `opencode run --format json`.
///
/// Written against events captured from the real CLI:
/// ```text
/// {"type":"step_start","sessionID":"ses_…","part":{…,"type":"step-start"}}
/// {"type":"text","sessionID":"ses_…","part":{"id":"prt_…","type":"text","text":"Hi",…}}
/// {"type":"step_finish","sessionID":"ses_…","part":{"reason":"stop","tokens":{…},"cost":0,…}}
/// ```
///
/// The important subtlety: a `text` event carries the **whole text of its part**,
/// not a delta, and the same `part.id` reappears as the part grows. Appending
/// each event verbatim duplicates the reply ("HiHi there"), so text is diffed
/// against what that part last contained and only the new suffix is emitted.
#[derive(Default)]
pub struct OpenCodeDecoder {
    session_id: Option<String>,
    /// Last text seen per part id, so repeated snapshots become deltas.
    parts: HashMap<String, String>,
    tools: HashMap<String, String>,
}

impl OpenCodeDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn decode_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            tracing::debug!(line = %&line[..line.len().min(160)], "undecodable opencode line");
            return Vec::new();
        };

        let mut out = Vec::new();
        if let Some(sid) = v.get("sessionID").and_then(Value::as_str) {
            if self.session_id.is_none() {
                self.session_id = Some(sid.to_string());
                out.push(AgentEvent::Started {
                    native_session_id: Some(sid.to_string()),
                    model: None,
                });
            }
        }

        let part = v.get("part");
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(part) = part {
                    let id = part.get("id").and_then(Value::as_str).unwrap_or("");
                    let full = part.get("text").and_then(Value::as_str).unwrap_or("");
                    let seen = self.parts.entry(id.to_string()).or_default();
                    // Only emit what is new. A shorter value means the part was
                    // rewritten rather than extended, so re-emit it whole.
                    let delta = if full.len() >= seen.len() && full.starts_with(seen.as_str()) {
                        full[seen.len()..].to_string()
                    } else {
                        full.to_string()
                    };
                    *seen = full.to_string();
                    if !delta.is_empty() {
                        out.push(AgentEvent::Text { delta });
                    }
                }
            }

            "reasoning" => {
                if let Some(part) = part {
                    let id = format!("r{}", part.get("id").and_then(Value::as_str).unwrap_or(""));
                    let full = part
                        .get("text")
                        .or_else(|| part.get("reasoning"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let seen = self.parts.entry(id).or_default();
                    let delta = if full.starts_with(seen.as_str()) {
                        full[seen.len()..].to_string()
                    } else {
                        full.to_string()
                    };
                    *seen = full.to_string();
                    if !delta.is_empty() {
                        out.push(AgentEvent::Reasoning { delta });
                    }
                }
            }

            "tool" | "tool_start" | "tool_use" => {
                if let Some(part) = part {
                    let id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = part
                        .get("tool")
                        .or_else(|| part.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let input = part
                        .get("state")
                        .and_then(|s| s.get("input"))
                        .or_else(|| part.get("input"))
                        .cloned()
                        .unwrap_or(Value::Null);

                    let status = part
                        .get("state")
                        .and_then(|s| s.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("");

                    if !self.tools.contains_key(&id) {
                        self.tools.insert(id.clone(), name.clone());
                        out.push(AgentEvent::ToolCall {
                            id: ToolId::new(id.as_str()),
                            title: tool_title(&name, &input),
                            name: name.into(),
                            input,
                        });
                    }

                    // A completed tool carries its output on the same event.
                    if matches!(status, "completed" | "error") {
                        let text = part
                            .get("state")
                            .and_then(|s| s.get("output"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        out.push(AgentEvent::ToolResult {
                            id: ToolId::new(id.as_str()),
                            output: ToolOutput::Terminal {
                                text,
                                exit_code: None,
                            },
                            is_error: status == "error",
                        });
                    }
                }
            }

            "step_finish" => {
                if let Some(part) = part {
                    if let Some(t) = part.get("tokens") {
                        let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
                        let cache = t.get("cache");
                        let cn = |k: &str| {
                            cache
                                .and_then(|c| c.get(k))
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        };
                        out.push(AgentEvent::Usage(Usage {
                            input: n("input"),
                            output: n("output"),
                            cache_read: cn("read"),
                            cache_write: cn("write"),
                            cost_usd: part.get("cost").and_then(Value::as_f64),
                        }));
                    }
                }
            }

            "error" => {
                out.push(AgentEvent::Fatal {
                    message: v
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("opencode reported an error")
                        .to_string(),
                    retryable: false,
                });
            }

            _ => {}
        }
        out
    }

    /// Called when the process exits; a one-shot run ends its turn there.
    pub fn finish(&mut self) -> Vec<AgentEvent> {
        self.parts.clear();
        self.tools.clear();
        vec![AgentEvent::TurnEnd {
            stop: StopReason::EndTurn,
        }]
    }
}

fn tool_title(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let pick = match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" => s("command"),
        "read" | "write" | "edit" => {
            let p = s("filePath");
            if p.is_empty() {
                s("path")
            } else {
                p
            }
        }
        "grep" | "glob" => s("pattern"),
        _ => "",
    };
    pick.chars().take(160).collect()
}

/// Decoder for CLIs we drive in plain-text mode.
///
/// Everything on stdout is the reply. ANSI escapes are stripped, because several
/// of these colour their output even when not attached to a TTY, and raw escape
/// bytes in a chat bubble look like corruption.
#[derive(Default)]
pub struct PlainDecoder {
    seen_any: bool,
}

impl PlainDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let clean = strip_ansi(line);
        // Leading blank lines are banner padding, not content.
        if !self.seen_any && clean.trim().is_empty() {
            return Vec::new();
        }
        self.seen_any = true;
        vec![AgentEvent::Text {
            delta: format!("{clean}\n"),
        }]
    }

    pub fn finish(&mut self) -> Vec<AgentEvent> {
        vec![AgentEvent::TurnEnd {
            stop: StopReason::EndTurn,
        }]
    }
}

/// Remove CSI/OSC escape sequences.
pub fn strip_ansi(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x1b {
            match b.get(i + 1) {
                // CSI: ends at the first byte in 0x40..=0x7e
                Some(b'[') => {
                    i += 2;
                    while i < b.len() && !(0x40..=0x7e).contains(&b[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                // OSC: ends at BEL or ST
                Some(b']') => {
                    i += 2;
                    while i < b.len() && b[i] != 0x07 {
                        if b[i] == 0x1b && b.get(i + 1) == Some(&b'\\') {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    i += 1;
                }
                _ => i += 2,
            }
            continue;
        }
        // Copy one whole char so multi-byte sequences survive.
        let ch_len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Text { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Verbatim capture from `opencode run --format json "say hi"`.
    const REAL_STEP_START: &str = r#"{"type":"step_start","timestamp":1785475426548,"sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","part":{"id":"prt_fb6a0e8f00013Dea2biNnVs0yE","messageID":"msg_fb6a0e1da001CKyXMxQWEHotVD","sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","type":"step-start"}}"#;
    const REAL_TEXT: &str = r#"{"type":"text","timestamp":1785475427417,"sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","part":{"id":"prt_fb6a0ec24001EPU2P6RaG9Uu3h","messageID":"msg_fb6a0e1da001CKyXMxQWEHotVD","sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","type":"text","text":"Hi","time":{"start":1785475427364,"end":1785475427401}}}"#;
    const REAL_STEP_FINISH: &str = r#"{"type":"step_finish","timestamp":1785475427417,"sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","part":{"id":"prt_fb6a0ec4e001zSKN3n7BGEHUMq","reason":"stop","messageID":"msg_fb6a0e1da001CKyXMxQWEHotVD","sessionID":"ses_0495f1f9bffesDc1ALe3Uyksmu","type":"step-finish","tokens":{"total":12843,"input":12828,"output":2,"reasoning":13,"cache":{"write":0,"read":0}},"cost":0}}"#;

    #[test]
    fn real_opencode_session_decodes_end_to_end() {
        let mut d = OpenCodeDecoder::new();
        let start = d.decode_line(REAL_STEP_START);
        assert!(matches!(
            start.first(),
            Some(AgentEvent::Started { native_session_id: Some(s), .. })
                if s == "ses_0495f1f9bffesDc1ALe3Uyksmu"
        ));
        assert_eq!(d.session_id(), Some("ses_0495f1f9bffesDc1ALe3Uyksmu"));

        assert_eq!(text_of(&d.decode_line(REAL_TEXT)), "Hi");

        let fin = d.decode_line(REAL_STEP_FINISH);
        match fin.first() {
            Some(AgentEvent::Usage(u)) => {
                assert_eq!((u.input, u.output), (12828, 2));
                assert_eq!(u.cost_usd, Some(0.0));
            }
            other => panic!("expected usage, got {other:?}"),
        }

        assert!(matches!(
            d.finish().first(),
            Some(AgentEvent::TurnEnd {
                stop: StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn repeated_part_snapshots_become_deltas_not_duplicates() {
        // The bug this guards: opencode re-sends the whole part as it grows.
        let mut d = OpenCodeDecoder::new();
        let mut all = Vec::new();
        for full in ["Hi", "Hi there", "Hi there!"] {
            let line = serde_json::json!({
                "type": "text",
                "sessionID": "ses_1",
                "part": {"id": "prt_1", "type": "text", "text": full}
            });
            all.extend(d.decode_line(&line.to_string()));
        }
        assert_eq!(text_of(&all), "Hi there!", "text must not be duplicated");
    }

    #[test]
    fn distinct_parts_accumulate_independently() {
        let mut d = OpenCodeDecoder::new();
        let mut all = Vec::new();
        for (id, t) in [("p1", "one "), ("p2", "two")] {
            let line = serde_json::json!({
                "type": "text", "sessionID": "s",
                "part": {"id": id, "type": "text", "text": t}
            });
            all.extend(d.decode_line(&line.to_string()));
        }
        assert_eq!(text_of(&all), "one two");
    }

    #[test]
    fn a_rewritten_part_is_re_emitted_whole() {
        let mut d = OpenCodeDecoder::new();
        let mk = |t: &str| {
            serde_json::json!({
                "type": "text", "sessionID": "s",
                "part": {"id": "p", "type": "text", "text": t}
            })
            .to_string()
        };
        d.decode_line(&mk("long original text"));
        // Shorter and not a prefix: treat as a replacement rather than silently
        // emitting nothing.
        let ev = d.decode_line(&mk("new"));
        assert_eq!(text_of(&ev), "new");
    }

    #[test]
    fn tool_events_emit_a_call_once_then_a_result() {
        let mut d = OpenCodeDecoder::new();
        let running = serde_json::json!({
            "type": "tool", "sessionID": "s",
            "part": {"id": "t1", "type": "tool", "tool": "bash",
                     "state": {"status": "running", "input": {"command": "ls -la"}}}
        });
        let ev = d.decode_line(&running.to_string());
        // `Started` is emitted first in this same batch, so search rather than
        // assuming a position.
        match ev.iter().find(|e| matches!(e, AgentEvent::ToolCall { .. })) {
            Some(AgentEvent::ToolCall { name, title, .. }) => {
                assert_eq!(name, "bash");
                assert_eq!(title, "ls -la");
            }
            other => panic!("{other:?}"),
        }

        let done = serde_json::json!({
            "type": "tool", "sessionID": "s",
            "part": {"id": "t1", "type": "tool", "tool": "bash",
                     "state": {"status": "completed", "input": {"command": "ls -la"},
                               "output": "a\nb\n"}}
        });
        let ev = d.decode_line(&done.to_string());
        assert!(
            !ev.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. })),
            "the call must not be emitted twice"
        );
        match ev
            .iter()
            .find(|e| matches!(e, AgentEvent::ToolResult { .. }))
        {
            Some(AgentEvent::ToolResult {
                output, is_error, ..
            }) => {
                assert!(!is_error);
                assert!(matches!(output, ToolOutput::Terminal { text, .. } if text == "a\nb\n"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_error_event_is_fatal_and_not_retryable() {
        let mut d = OpenCodeDecoder::new();
        let ev = d.decode_line(r#"{"type":"error","message":"Not signed in."}"#);
        match ev.first() {
            Some(AgentEvent::Fatal { message, retryable }) => {
                assert!(message.contains("Not signed in"));
                assert!(!retryable, "auth failures must not be retried in a loop");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn garbage_and_banner_lines_are_ignored() {
        let mut d = OpenCodeDecoder::new();
        assert!(d.decode_line("█▀▀█ █▀▀█").is_empty());
        assert!(d.decode_line("").is_empty());
        assert!(d.decode_line("not json").is_empty());
        assert!(d.decode_line(r#"{"type":"brand_new"}"#).is_empty());
        // Still functional afterwards.
        assert_eq!(text_of(&d.decode_line(REAL_TEXT)), "Hi");
    }

    #[test]
    fn plain_decoder_passes_lines_through_and_drops_leading_blanks() {
        let mut d = PlainDecoder::new();
        assert!(d.decode_line("   ").is_empty(), "banner padding dropped");
        assert_eq!(text_of(&d.decode_line("hello")), "hello\n");
        // Blank lines *inside* the reply are content.
        assert_eq!(text_of(&d.decode_line("")), "\n");
    }

    #[test]
    fn plain_decoder_strips_ansi_colour() {
        let mut d = PlainDecoder::new();
        let ev = d.decode_line("\x1b[1;32mgreen\x1b[0m text");
        assert_eq!(text_of(&ev), "green text\n");
    }

    #[test]
    fn strip_ansi_handles_osc_and_preserves_unicode() {
        assert_eq!(strip_ansi("\x1b]0;title\x07body"), "body");
        assert_eq!(strip_ansi("héllo 🚀"), "héllo 🚀");
        assert_eq!(strip_ansi("\x1b[Kclean"), "clean");
        // Truncated escape at end of buffer must not panic.
        let _ = strip_ansi("text\x1b[");
        let _ = strip_ansi("text\x1b");
    }
}
