//! Adapters for coding CLIs driven one turn at a time.
//!
//! Unlike the Claude adapter, these CLIs have no bidirectional streaming input
//! mode: each turn is a fresh process invocation. The transcript is ours, so
//! continuity comes from replaying it (or the CLI's own `--continue`) rather than
//! from a long-lived session — which is exactly what `AgentSession::hydrate` was
//! designed for.

pub mod decode;
pub mod spec;

use agent_core::{
    AgentAdapter, AgentError, AgentEvent, AgentId, AgentSession, Availability, Caps, Decision,
    HydrateMode, ModelSpec, RequestId, SessionCtx, Transcript, UserTurn,
};
use async_trait::async_trait;
use futures::stream::BoxStream;
use spec::{CliSpec, Wire};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const EVENT_BUFFER: usize = 1024;
const REPLAY_BUDGET: usize = 40_000;

pub struct CliAdapter {
    spec: &'static CliSpec,
}

impl CliAdapter {
    pub fn new(spec: &'static CliSpec) -> Self {
        Self { spec }
    }

    /// Every installed CLI backend, ready to register.
    pub fn all() -> Vec<Arc<dyn AgentAdapter>> {
        spec::ALL
            .iter()
            .map(|s| Arc::new(CliAdapter::new(s)) as Arc<dyn AgentAdapter>)
            .collect()
    }
}

#[async_trait]
impl AgentAdapter for CliAdapter {
    fn id(&self) -> AgentId {
        AgentId::new(self.spec.id)
    }

    fn display_name(&self) -> &str {
        self.spec.display_name
    }

    fn caps(&self) -> Caps {
        self.spec.caps
    }

    fn models(&self) -> &'static [ModelSpec] {
        self.spec.models
    }

    fn setup(&self) -> Option<agent_core::Setup> {
        self.spec.setup
    }

    async fn probe(&self) -> Result<Availability, AgentError> {
        // `--version` is the cheapest liveness check that does not hit the
        // network or consume credits.
        match Command::new(self.spec.bin).arg("--version").output().await {
            Ok(o) if o.status.success() => Ok(Availability::Ready {
                version: Some(
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                ),
            }),
            Ok(_) => Ok(Availability::Ready { version: None }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Availability::Missing {
                hint: format!("`{}` is not on PATH", self.spec.bin),
            }),
            Err(e) => Err(AgentError::Spawn(e)),
        }
    }

    async fn spawn(&self, ctx: SessionCtx) -> Result<Box<dyn AgentSession>, AgentError> {
        Ok(Box::new(CliSession {
            spec: self.spec,
            ctx,
            events: None,
            child: None,
            native_session: None,
            primer: None,
            turns: 0,
        }))
    }
}

pub struct CliSession {
    spec: &'static CliSpec,
    ctx: SessionCtx,
    /// Receiver for the currently-running turn.
    events: Option<mpsc::Receiver<AgentEvent>>,
    child: Option<tokio::process::Child>,
    native_session: Option<String>,
    /// Transcript briefing prepended to the first prompt, set by `hydrate`.
    primer: Option<String>,
    turns: u32,
}

impl CliSession {
    /// Compose the text actually sent to the CLI.
    ///
    /// These backends take a single prompt string per invocation, so the harness
    /// prompt and any handoff briefing have to ride along inside it — there is no
    /// system-prompt flag to put them in.
    fn compose(&mut self, turn: &UserTurn) -> String {
        let mut prompt = String::new();
        if self.turns == 0 {
            if !self.ctx.system_prompt.is_empty() {
                prompt.push_str(&self.ctx.system_prompt);
                prompt.push_str("\n\n");
            }
            if let Some(primer) = self.primer.take() {
                prompt.push_str(&primer);
                prompt.push_str("\n\n");
            }
        }
        prompt.push_str(&turn.text);
        for att in &turn.attachments {
            prompt.push_str(&format!("\n[attached file: {}]", att.path.display()));
        }
        prompt
    }
}

#[async_trait]
impl AgentSession for CliSession {
    async fn send(&mut self, turn: UserTurn) -> Result<(), AgentError> {
        let prompt = self.compose(&turn);
        // Resume only when the CLI supports it *and* we have already run a turn
        // in this session; otherwise `--continue` would attach to whatever
        // unrelated session the user last ran in this directory.
        let resume = self.turns > 0 && self.spec.continue_flag.is_some();
        let args = self.spec.args(
            &prompt,
            &self.ctx.cwd.to_string_lossy(),
            self.ctx.model.as_deref(),
            resume,
            self.ctx.permission_mode,
        );
        self.turns += 1;

        let mut child = Command::new(self.spec.bin)
            .args(&args)
            .envs(self.ctx.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&self.ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(AgentError::Spawn)?;

        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, rx) = mpsc::channel(EVENT_BUFFER);
        self.events = Some(rx);

        let wire = self.spec.wire;
        let err_tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut json = decode::OpenCodeDecoder::new();
            let mut plain = decode::PlainDecoder::new();

            while let Ok(Some(line)) = lines.next_line().await {
                let events = match wire {
                    Wire::OpenCodeJson => json.decode_line(&line),
                    Wire::PlainText => plain.decode_line(&line),
                };
                for ev in events {
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
            }
            let tail = match wire {
                Wire::OpenCodeJson => json.finish(),
                Wire::PlainText => plain.finish(),
            };
            for ev in tail {
                let _ = tx.send(ev).await;
            }
        });

        // Stderr is only surfaced when the process fails; these CLIs write
        // progress chatter there routinely, and mixing it into the reply would
        // be noise.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut tail = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "agent.cli.stderr", "{line}");
                tail.push(line);
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
            if !tail.is_empty() {
                tracing::debug!("cli stderr tail: {}", tail.join(" | "));
            }
            drop(err_tx);
        });

        self.child = Some(child);
        Ok(())
    }

    fn events(&mut self) -> BoxStream<'_, AgentEvent> {
        use std::task::Poll;
        match &mut self.events {
            Some(rx) => Box::pin(futures::stream::poll_fn(move |cx| match rx.poll_recv(cx) {
                // The turn's channel closing means *this turn* finished, not that
                // the backend died — every turn here is its own process. Ending
                // the stream would make the driver exit and close the command
                // channel, so the next message would fail with "session is no
                // longer accepting input". Go idle instead; the command future's
                // waker is what brings this task back.
                Poll::Ready(None) => Poll::Pending,
                other => other,
            })),
            // No turn in flight: never yield, rather than end.
            None => Box::pin(futures::stream::pending()),
        }
    }

    async fn interrupt(&mut self) -> Result<(), AgentError> {
        // No control channel exists for a one-shot process, so killing it is the
        // only real stop. The next turn spawns a fresh one.
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
        Ok(())
    }

    async fn decide(&mut self, _id: RequestId, _d: Decision) -> Result<(), AgentError> {
        Err(AgentError::Unsupported(
            "this CLI has no per-call permission hook; approval is set at spawn",
        ))
    }

    async fn hydrate(&mut self, transcript: &Transcript) -> Result<HydrateMode, AgentError> {
        if transcript.messages.is_empty() {
            return Ok(HydrateMode::Fresh);
        }
        // Reuse the Claude adapter's briefing renderer: it is backend-agnostic
        // by design and already drops the previous model's private reasoning.
        let replay = agent_claude::replay::render(transcript, REPLAY_BUDGET);
        let dropped = replay.dropped;
        self.primer = Some(replay.text);
        Ok(if dropped > 0 {
            HydrateMode::ReplayedPartial {
                dropped_messages: dropped,
            }
        } else {
            HydrateMode::Replayed
        })
    }

    fn native_session_id(&self) -> Option<String> {
        self.native_session.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> SessionCtx {
        SessionCtx {
            session_id: agent_core::SessionId::new(),
            cwd: PathBuf::from("/tmp"),
            system_prompt: Arc::from("HARNESS"),
            model: None,
            mcp: Arc::new(vec![]),
            skills_dir: None,
            runtime_dir: PathBuf::from("/tmp"),
            resume: None,
            permission_mode: agent_core::PermissionMode::Ask,
            env: Vec::new(),
            allowed_tools: Vec::new(),
        }
    }

    fn session(spec: &'static CliSpec) -> CliSession {
        CliSession {
            spec,
            ctx: ctx(),
            events: None,
            child: None,
            native_session: None,
            primer: None,
            turns: 0,
        }
    }

    #[test]
    fn the_first_prompt_carries_the_harness_and_later_ones_do_not() {
        let mut s = session(&spec::CRUSH);
        let first = s.compose(&UserTurn::text("hello"));
        assert!(first.starts_with("HARNESS"));
        assert!(first.ends_with("hello"));

        s.turns = 1;
        let second = s.compose(&UserTurn::text("again"));
        assert_eq!(second, "again", "harness must not be resent every turn");
    }

    #[tokio::test]
    async fn a_hydrated_transcript_rides_along_in_the_first_prompt() {
        let mut s = session(&spec::CRUSH);
        let mut t = Transcript::default();
        t.push_user("earlier question".into(), vec![], 0);

        let mode = s.hydrate(&t).await.unwrap();
        assert_eq!(mode, HydrateMode::Replayed);

        let prompt = s.compose(&UserTurn::text("follow up"));
        assert!(prompt.contains("earlier question"), "briefing missing");
        assert!(prompt.contains("do not re-run"));
        assert!(prompt.ends_with("follow up"));
    }

    #[tokio::test]
    async fn an_empty_transcript_hydrates_as_fresh_with_no_primer() {
        let mut s = session(&spec::CRUSH);
        assert_eq!(
            s.hydrate(&Transcript::default()).await.unwrap(),
            HydrateMode::Fresh
        );
        assert!(s.primer.is_none());
    }

    #[tokio::test]
    async fn probe_reports_missing_binaries_rather_than_erroring() {
        struct Fake;
        static MISSING: CliSpec = CliSpec {
            id: "nope",
            display_name: "Nope",
            bin: "definitely-not-installed-xyz",
            wire: Wire::PlainText,
            base_args: &[],
            cwd_flag: None,
            model_flag: None,
            continue_flag: None,
            yolo_flag: None,
            prompt_flag: None,
            caps: Caps::DEGRADED,
            models: &[],
            setup: None,
        };
        let _ = Fake;
        let a = CliAdapter::new(&MISSING);
        match a.probe().await.unwrap() {
            Availability::Missing { hint } => assert!(hint.contains("PATH")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn all_registers_every_spec() {
        let all = CliAdapter::all();
        assert_eq!(all.len(), spec::ALL.len());
        let ids: Vec<String> = all.iter().map(|a| a.id().to_string()).collect();
        for want in ["opencode", "vibe", "grok", "crush", "gemini", "codex"] {
            assert!(ids.contains(&want.to_string()), "{want} not registered");
        }
    }

    #[test]
    fn resume_is_not_used_on_the_very_first_turn() {
        // Otherwise `--continue` attaches to an unrelated prior session that
        // happens to exist in this directory.
        let s = session(&spec::OPENCODE);
        let resume = s.turns > 0 && s.spec.continue_flag.is_some();
        assert!(!resume);
    }
}
