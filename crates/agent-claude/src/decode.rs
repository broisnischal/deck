//! Line-oriented decoder: Claude Code stream-json -> [`AgentEvent`].
//!
//! Kept pure and stateful-but-synchronous so it is exhaustively testable
//! without spawning a process. All the protocol risk lives here.

use crate::wire::{self, BlockDelta, ContentBlock, ContentField, Line, RawEvent};
use agent_core::{Action, AgentEvent, Decision, RequestId, StopReason, ToolId, ToolOutput};
use serde_json::Value;
use std::collections::HashMap;

/// Hard cap on a single tool result we keep in memory. Beyond this the output is
/// reported as `Truncated`; unbounded tool output is the classic way this class
/// of app dies.
const TOOL_OUTPUT_CAP: usize = 256 * 1024;

pub struct Decoder {
    session_id: Option<String>,
    /// Tool name by tool_use id, so a `tool_result` can be rendered with the
    /// right renderer (the result line carries no tool name).
    tool_names: HashMap<String, String>,
    /// Partial `input_json_delta` accumulation, keyed by content-block index.
    partial_tools: HashMap<u32, PartialTool>,
    /// True once we have streamed actual text/thinking for the message currently
    /// in flight. The CLI sends *both* partial deltas and a final complete
    /// `assistant` message, so without this the text would emit twice.
    ///
    /// Scoped per-message and set only by content-bearing events: a contentless
    /// `stream_event` (`ping`, `message_start`) must not suppress a whole-message
    /// line, and a turn that streams one message then sends another unstreamed
    /// must still emit the second.
    streamed_content: bool,
    /// Our ids for the CLI's open permission requests, by its request id.
    asks: HashMap<String, RequestId>,
    /// Requests decoded since the last `take_asks`, for the session to answer.
    new_asks: Vec<(RequestId, PendingAsk)>,
}

/// What answering a permission request needs: the CLI's request id, the
/// tool input to echo back (possibly with answers added), and the "always
/// allow" rules it suggested.
#[derive(Debug, Clone)]
pub struct PendingAsk {
    pub request_id: String,
    pub input: Value,
    pub suggestions: Vec<Value>,
}

struct PartialTool {
    id: String,
    name: String,
    json: String,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            session_id: None,
            tool_names: HashMap::new(),
            partial_tools: HashMap::new(),
            streamed_content: false,
            asks: HashMap::new(),
            new_asks: Vec::new(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Decode one NDJSON line. Unparseable or unknown lines yield no events
    /// rather than failing the session.
    pub fn decode_line(&mut self, line: &str) -> Vec<AgentEvent> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let parsed: Line = match serde_json::from_str(line) {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!(error = %e, line = %truncate(line, 200), "undecodable stream-json line");
                return Vec::new();
            }
        };
        self.decode(parsed)
    }

    /// Permission requests decoded since the last call.
    pub fn take_asks(&mut self) -> Vec<(RequestId, PendingAsk)> {
        std::mem::take(&mut self.new_asks)
    }

    /// `can_use_tool`: a question for the user (AskUserQuestion) or an
    /// approval for a tool call.
    fn decode_control(&mut self, c: wire::ControlRequestLine, out: &mut Vec<AgentEvent>) {
        let r = &c.request;
        if r.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
            tracing::debug!(request = %r, "unhandled control request");
            return;
        }
        let tool = r
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let input = r.get("input").cloned().unwrap_or(Value::Null);
        let tool_use = r
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(ToolId::new);
        let id = RequestId::new();
        self.asks.insert(c.request_id.clone(), id);
        self.new_asks.push((
            id,
            PendingAsk {
                request_id: c.request_id,
                input: input.clone(),
                suggestions: r
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
        ));
        if tool == "AskUserQuestion" {
            out.push(AgentEvent::Question {
                id,
                tool_use,
                questions: questions(&input),
            });
            return;
        }
        let s = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let action = match tool.as_str() {
            "Bash" => Action::Bash {
                command: s("command"),
                cwd: None,
            },
            "Write" => Action::WriteFile {
                path: s("file_path"),
            },
            "Edit" | "MultiEdit" | "NotebookEdit" => Action::EditFile {
                path: if s("file_path").is_empty() {
                    s("notebook_path")
                } else {
                    s("file_path")
                },
            },
            "Read" => Action::ReadFile {
                path: s("file_path"),
            },
            "WebFetch" => Action::Fetch { url: s("url") },
            name if name.starts_with("mcp__") => {
                let mut parts = name.trim_start_matches("mcp__").splitn(2, "__");
                Action::McpCall {
                    server: parts.next().unwrap_or("").into(),
                    tool: parts.next().unwrap_or("").into(),
                    input: input.clone(),
                }
            }
            name => Action::Other {
                name: name.into(),
                input: input.clone(),
            },
        };
        let reason = r
            .get("description")
            .or_else(|| r.get("decision_reason"))
            .and_then(Value::as_str)
            .map(str::to_string);
        out.push(AgentEvent::Permission {
            id,
            action,
            suggestions: vec![Decision::Allow, Decision::AllowAlways, Decision::Deny],
            tool_use,
            input,
            reason,
        });
    }

    fn decode(&mut self, line: Line) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        match line {
            Line::System(s) => {
                if let Some(id) = s.session_id {
                    let first = self.session_id.is_none();
                    self.session_id = Some(id.clone());
                    if first {
                        out.push(AgentEvent::Started {
                            native_session_id: Some(id),
                            model: s.model.map(Into::into),
                        });
                    }
                }
            }

            Line::StreamEvent(se) => {
                self.decode_raw(se.event, &mut out);
            }

            Line::Assistant(m) => {
                if let ContentField::Blocks(blocks) = m.message.content {
                    for b in blocks {
                        match b {
                            // Already streamed via partial deltas.
                            ContentBlock::Text { text } if !self.streamed_content => {
                                out.push(AgentEvent::Text { delta: text });
                            }
                            ContentBlock::Thinking { thinking } if !self.streamed_content => {
                                out.push(AgentEvent::Reasoning { delta: thinking });
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                // Record the name even when partials already
                                // emitted the call, so tool_result can find it.
                                let known = self.tool_names.insert(id.clone(), name.clone());
                                if known.is_none() {
                                    out.push(tool_call_event(&id, &name, input));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // This message is complete; a following one streams afresh.
                self.streamed_content = false;
            }

            // Our own message read back: the model has it now.
            Line::User(m) if m.is_replay => {
                if let Some(id) = m.uuid {
                    out.push(AgentEvent::UserDelivered { id });
                }
            }

            Line::ControlRequest(c) => self.decode_control(c, &mut out),

            Line::ControlCancelRequest(c) => {
                if let Some(id) = self.asks.remove(&c.request_id) {
                    out.push(AgentEvent::AskCancelled { id });
                }
            }

            Line::CommandLifecycle(c) => {
                if let (Some(id), Some("cancelled")) = (c.command_uuid, c.state.as_deref()) {
                    out.push(AgentEvent::UserDropped { id });
                }
            }

            // Tool results come back as a synthetic `user` message.
            Line::User(m) => {
                if let ContentField::Blocks(blocks) = m.message.content {
                    for b in blocks {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } = b
                        {
                            let name = self
                                .tool_names
                                .get(&tool_use_id)
                                .map(String::as_str)
                                .unwrap_or("");
                            out.push(AgentEvent::ToolResult {
                                id: ToolId::new(tool_use_id.as_str()),
                                output: classify_output(name, &content),
                                is_error,
                            });
                        }
                    }
                }
            }

            Line::Result(r) => {
                if let Some(u) = &r.usage {
                    out.push(AgentEvent::Usage(u.to_core(r.total_cost_usd)));
                } else if let Some(cost) = r.total_cost_usd {
                    out.push(AgentEvent::Usage(agent_core::Usage {
                        cost_usd: Some(cost),
                        ..Default::default()
                    }));
                }
                let stop = match r.subtype.as_deref() {
                    Some("error_max_turns") => StopReason::MaxTurns,
                    Some("error_during_execution") => StopReason::Error,
                    _ if r.is_error => StopReason::Error,
                    _ => StopReason::EndTurn,
                };
                out.push(AgentEvent::TurnEnd { stop });
                // A new turn may reuse block indices and will stream fresh
                // partials; reset per-turn state but keep tool names, since
                // results can straddle the boundary.
                self.partial_tools.clear();
                self.streamed_content = false;
            }

            Line::Unknown => {}
        }
        out
    }

    fn decode_raw(&mut self, event: RawEvent, out: &mut Vec<AgentEvent>) {
        match event {
            RawEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    self.tool_names.insert(id.clone(), name.clone());
                    self.partial_tools.insert(
                        index,
                        PartialTool {
                            id,
                            name,
                            json: String::new(),
                        },
                    );
                }
                ContentBlock::Text { text } if !text.is_empty() => {
                    self.streamed_content = true;
                    out.push(AgentEvent::Text { delta: text });
                }
                ContentBlock::Thinking { thinking } if !thinking.is_empty() => {
                    self.streamed_content = true;
                    out.push(AgentEvent::Reasoning { delta: thinking });
                }
                _ => {}
            },

            RawEvent::ContentBlockDelta { index, delta } => match delta {
                BlockDelta::TextDelta { text } => {
                    self.streamed_content = true;
                    out.push(AgentEvent::Text { delta: text });
                }
                BlockDelta::ThinkingDelta { thinking } => {
                    self.streamed_content = true;
                    out.push(AgentEvent::Reasoning { delta: thinking });
                }
                BlockDelta::InputJsonDelta { partial_json } => {
                    if let Some(p) = self.partial_tools.get_mut(&index) {
                        p.json.push_str(&partial_json);
                    }
                }
                BlockDelta::Unknown => {}
            },

            // Emit the tool call only once its input JSON is complete, so the
            // card has a real title instead of a placeholder that reflows.
            RawEvent::ContentBlockStop { index } => {
                if let Some(p) = self.partial_tools.remove(&index) {
                    let input = if p.json.trim().is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&p.json).unwrap_or(Value::Null)
                    };
                    out.push(tool_call_event(&p.id, &p.name, input));
                }
            }

            RawEvent::MessageDelta { usage } => {
                if let Some(u) = usage {
                    out.push(AgentEvent::Usage(u.to_core(None)));
                }
            }

            RawEvent::Unknown => {}
        }
    }
}

fn tool_call_event(id: &str, name: &str, input: Value) -> AgentEvent {
    AgentEvent::ToolCall {
        id: ToolId::new(id),
        name: name.into(),
        title: tool_title(name, &input),
        input,
    }
}

/// Short label for the collapsed tool card, mirroring what the CLI shows.
fn tool_title(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match name {
        "Bash" | "bash" => truncate(s("command"), 160).to_string(),
        "Read" | "read" => truncate(s("file_path"), 160).to_string(),
        "Write" | "write" => truncate(s("file_path"), 160).to_string(),
        "Edit" | "edit" => truncate(s("file_path"), 160).to_string(),
        "Glob" | "glob" => s("pattern").to_string(),
        "Grep" | "grep" => s("pattern").to_string(),
        "WebFetch" => s("url").to_string(),
        "Task" => s("description").to_string(),
        other => {
            // MCP tools are named `mcp__server__tool`; show `server/tool`.
            match other.strip_prefix("mcp__") {
                Some(rest) => rest.replacen("__", "/", 1),
                None => other.to_string(),
            }
        }
    }
}

/// Pick a renderer for a tool result based on which tool produced it.
fn classify_output(tool_name: &str, content: &Value) -> ToolOutput {
    let text = flatten_content(content);
    if text.len() > TOOL_OUTPUT_CAP {
        return ToolOutput::Truncated {
            preview: truncate(&text, 4096).to_string(),
            total_bytes: text.len() as u64,
            spill_path: None,
        };
    }
    match tool_name {
        "Bash" | "bash" => ToolOutput::Terminal {
            text,
            exit_code: None,
        },
        "Read" | "read" => ToolOutput::FileExcerpt {
            path: String::new(),
            text,
            start_line: 1,
        },
        "Edit" | "edit" | "Write" | "write" => ToolOutput::Text(text),
        _ if content.is_object() || content.is_array() && text.is_empty() => {
            ToolOutput::Structured(content.clone())
        }
        _ => ToolOutput::Text(text),
    }
}

/// `tool_result.content` is a string, or an array of `{type:"text",text}`
/// blocks, or an arbitrary object.
fn flatten_content(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    out.push_str(t);
                }
            }
            out
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Truncate on a char boundary.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let end = s
        .char_indices()
        .take_while(|(i, _)| *i <= max)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    &s[..end]
}

/// AskUserQuestion's input: `{questions: [{question, header, options:
/// [{label, description}], multiSelect}]}`.
fn questions(input: &Value) -> Vec<agent_core::Question> {
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    input
        .get("questions")
        .and_then(Value::as_array)
        .map(|qs| {
            qs.iter()
                .map(|q| agent_core::Question {
                    header: str_of(q, "header"),
                    question: str_of(q, "question"),
                    options: q
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|os| {
                            os.iter()
                                .map(|o| agent_core::QuestionOption {
                                    label: str_of(o, "label"),
                                    description: str_of(o, "description"),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    multi: q
                        .get("multiSelect")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_text(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Text { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn init_line_yields_started_once() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"system","subtype":"init","session_id":"abc","model":"claude-opus-5"}"#,
        );
        assert!(matches!(
            ev.as_slice(),
            [AgentEvent::Started {
                native_session_id: Some(s),
                ..
            }] if s == "abc"
        ));
        assert_eq!(d.session_id(), Some("abc"));
        // A second system line must not re-announce the session.
        let ev = d.decode_line(r#"{"type":"system","subtype":"other","session_id":"abc"}"#);
        assert!(ev.is_empty());
    }

    #[test]
    fn whole_message_mode_emits_text() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Yes. It uses gpui 0.2.2"}]}}"#,
        );
        assert_eq!(ev_text(&ev), "Yes. It uses gpui 0.2.2");
    }

    #[test]
    fn partial_mode_does_not_double_emit_final_message() {
        let mut d = Decoder::new();
        let mut all = Vec::new();
        all.extend(d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}"#,
        ));
        all.extend(d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}}"#,
        ));
        // The CLI now repeats the complete message.
        all.extend(d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#,
        ));
        assert_eq!(ev_text(&all), "Hello", "text emitted exactly once");
    }

    #[test]
    fn streamed_tool_input_emits_one_call_with_parsed_input() {
        let mut d = Decoder::new();
        let mut all = Vec::new();
        all.extend(d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"Bash","input":{}}}}"#,
        ));
        assert!(all.is_empty(), "no call until input is complete");
        for frag in [r#"{"comm"#, r#"and":"ls -la"}"#] {
            let line = serde_json::json!({
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 1,
                    "delta": {"type": "input_json_delta", "partial_json": frag}
                }
            });
            all.extend(d.decode_line(&line.to_string()));
        }
        all.extend(d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#,
        ));

        let calls: Vec<_> = all
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCall { .. }))
            .collect();
        assert_eq!(calls.len(), 1);
        match calls[0] {
            AgentEvent::ToolCall {
                id,
                name,
                title,
                input,
                ..
            } => {
                assert_eq!(id.as_str(), "tu_1");
                assert_eq!(name, "Bash");
                assert_eq!(title, "ls -la");
                assert_eq!(input["command"], "ls -la");
            }
            _ => unreachable!(),
        }

        // The trailing complete assistant message must not duplicate the call.
        let dup = d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls -la"}}]}}"#,
        );
        assert!(
            !dup.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. })),
            "tool call must not be emitted twice"
        );
    }

    #[test]
    fn tool_result_uses_renderer_of_originating_tool() {
        let mut d = Decoder::new();
        d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_9","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        let ev = d.decode_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu_9","content":[{"type":"text","text":"a\nb\n"}]}]}}"#,
        );
        match &ev[0] {
            AgentEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                assert_eq!(id.as_str(), "tu_9");
                assert!(!is_error);
                assert!(
                    matches!(output, ToolOutput::Terminal { text, .. } if text == "a\nb\n"),
                    "bash output should render as a terminal, got {output:?}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn string_content_tool_result_flattens() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"x","content":"plain","is_error":true}]}}"#,
        );
        match &ev[0] {
            AgentEvent::ToolResult {
                output, is_error, ..
            } => {
                assert!(is_error);
                assert!(matches!(output, ToolOutput::Text(t) if t == "plain"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn result_line_reports_usage_cost_and_turn_end() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"result","subtype":"success","total_cost_usd":0.0123,"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}"#,
        );
        match &ev[0] {
            AgentEvent::Usage(u) => {
                assert_eq!((u.input, u.output, u.cache_read), (10, 20, 5));
                assert_eq!(u.cost_usd, Some(0.0123));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            ev[1],
            AgentEvent::TurnEnd {
                stop: StopReason::EndTurn
            }
        ));
    }

    #[test]
    fn max_turns_result_maps_to_max_turns_stop() {
        let mut d = Decoder::new();
        let ev = d.decode_line(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        assert!(matches!(
            ev.last(),
            Some(AgentEvent::TurnEnd {
                stop: StopReason::MaxTurns
            })
        ));
    }

    #[test]
    fn thinking_deltas_become_reasoning() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
        );
        assert!(matches!(&ev[0], AgentEvent::Reasoning { delta } if delta == "hmm"));
    }

    #[test]
    fn contentless_stream_events_do_not_suppress_a_whole_message() {
        let mut d = Decoder::new();
        // A `ping` is a stream_event carrying no content; it must not make the
        // decoder believe the upcoming message was already streamed.
        assert!(d
            .decode_line(r#"{"type":"stream_event","event":{"type":"ping"}}"#)
            .is_empty());
        let ev = d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#,
        );
        assert_eq!(ev_text(&ev), "ok");
    }

    #[test]
    fn second_unstreamed_message_in_one_turn_still_emits() {
        let mut d = Decoder::new();
        let mut all = Vec::new();
        // First message streams.
        all.extend(d.decode_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"first"}}}"#,
        ));
        all.extend(d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first"}]}}"#,
        ));
        // Second arrives whole, with no partials.
        all.extend(d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":" second"}]}}"#,
        ));
        assert_eq!(ev_text(&all), "first second");
    }

    #[test]
    fn garbage_and_unknown_lines_are_ignored_not_fatal() {
        let mut d = Decoder::new();
        assert!(d.decode_line("not json at all").is_empty());
        assert!(d
            .decode_line(r#"{"type":"brand_new_event","x":1}"#)
            .is_empty());
        assert!(d
            .decode_line(r#"{"type":"stream_event","event":{"type":"ping"}}"#)
            .is_empty());
        assert!(d.decode_line("").is_empty());
        // Decoder still works afterwards.
        let ev = d.decode_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#,
        );
        assert_eq!(ev_text(&ev), "ok");
    }

    #[test]
    fn oversized_tool_output_is_reported_as_truncated() {
        let mut d = Decoder::new();
        let big = "x".repeat(TOOL_OUTPUT_CAP + 10);
        let line = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type":"tool_result","tool_use_id":"t","content": big}]}
        });
        let ev = d.decode_line(&line.to_string());
        match &ev[0] {
            AgentEvent::ToolResult { output, .. } => match output {
                ToolOutput::Truncated {
                    total_bytes,
                    preview,
                    ..
                } => {
                    assert_eq!(*total_bytes as usize, TOOL_OUTPUT_CAP + 10);
                    assert!(preview.len() <= 4100);
                }
                other => panic!("expected truncation, got {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_titles_render_as_server_slash_tool() {
        assert_eq!(
            tool_title("mcp__azure__get_work_item", &Value::Null),
            "azure/get_work_item"
        );
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        let s = "héllo wörld";
        for n in 0..s.len() + 2 {
            let _ = truncate(s, n); // must not panic
        }
    }

    #[test]
    fn our_message_read_back_mid_turn_is_reported_as_delivered() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"user","uuid":"22222222-2222-4222-8222-222222222222","isReplay":true,"parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"text","text":"also 42"}]}}"#,
        );
        assert!(
            matches!(&ev[..], [AgentEvent::UserDelivered { id }] if id == "22222222-2222-4222-8222-222222222222")
        );
        // A tool result is still a tool result.
        let ev = d.decode_line(
            r#"{"type":"user","uuid":"x","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#,
        );
        assert!(matches!(&ev[..], [AgentEvent::ToolResult { .. }]));
    }

    #[test]
    fn a_queued_message_the_cli_cancels_is_reported_as_dropped() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"command_lifecycle","command_uuid":"abc","state":"cancelled","uuid":"z"}"#,
        );
        assert!(matches!(&ev[..], [AgentEvent::UserDropped { id }] if id == "abc"));
        assert!(d
            .decode_line(r#"{"type":"command_lifecycle","command_uuid":"abc","state":"queued"}"#)
            .is_empty());
    }

    #[test]
    fn a_tool_waiting_for_approval_becomes_a_permission_request() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"control_request","request_id":"r-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch a.txt"},"description":"Create a.txt","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch a.txt"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"tu_1"}}"#,
        );
        let [AgentEvent::Permission {
            id,
            action,
            tool_use,
            reason,
            ..
        }] = &ev[..]
        else {
            panic!("{ev:?}")
        };
        assert!(matches!(action, Action::Bash { command, .. } if command == "touch a.txt"));
        assert_eq!(tool_use.as_ref().map(|t| t.as_str()), Some("tu_1"));
        assert_eq!(reason.as_deref(), Some("Create a.txt"));
        let asks = d.take_asks();
        assert_eq!(asks.len(), 1);
        assert_eq!(
            (
                asks[0].0,
                asks[0].1.request_id.as_str(),
                asks[0].1.suggestions.len()
            ),
            (*id, "r-1", 1)
        );
        // Cancelled before it was answered.
        let ev = d.decode_line(r#"{"type":"control_cancel_request","request_id":"r-1"}"#);
        assert!(matches!(&ev[..], [AgentEvent::AskCancelled { id: c }] if c == id));
    }

    #[test]
    fn ask_user_question_becomes_a_question() {
        let mut d = Decoder::new();
        let ev = d.decode_line(
            r#"{"type":"control_request","request_id":"q-1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Which color?","header":"Color","options":[{"label":"Red","description":"warm"},{"label":"Blue","description":"cool"}],"multiSelect":false}]},"tool_use_id":"tu_2","requires_user_interaction":true}}"#,
        );
        let [AgentEvent::Question { questions, .. }] = &ev[..] else {
            panic!("{ev:?}")
        };
        assert_eq!(questions[0].question, "Which color?");
        assert_eq!(
            questions[0]
                .options
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Red", "Blue"]
        );
        assert!(!questions[0].multi);
    }
}
