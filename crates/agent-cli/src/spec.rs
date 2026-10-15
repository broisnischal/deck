//! Declarative descriptions of the coding CLIs we drive.
//!
//! Every flag here was read off the tool's own `--help` on a machine with it
//! installed, not guessed. Where a CLI offers a structured output mode whose
//! event schema we have actually observed, we use it; where we have not observed
//! it, we deliberately use plain-text mode and declare `structured: false`
//! rather than shipping a decoder written against an imagined format.

use agent_core::{Caps, ModelSpec, PermissionMode};

/// How to read a backend's stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// NDJSON in opencode's `run --format json` shape. Verified against the real
    /// CLI.
    OpenCodeJson,
    /// Everything the process prints is the reply. Used for CLIs whose JSON
    /// event schema we have not verified — a wrong decoder is worse than no
    /// decoder, because it silently drops content.
    PlainText,
}

#[derive(Debug, Clone)]
pub struct CliSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub bin: &'static str,
    pub wire: Wire,
    /// Argv before the prompt. `{}` placeholders are substituted at spawn.
    pub base_args: &'static [&'static str],
    /// Flag pair for the working directory, if the CLI takes one.
    pub cwd_flag: Option<&'static str>,
    /// Flag pair for the model.
    pub model_flag: Option<&'static str>,
    /// Flag that resumes/continues the previous session.
    pub continue_flag: Option<&'static str>,
    /// Flag that skips tool approval prompts. Required for headless use: without
    /// it these CLIs either hang waiting for a TTY or refuse every tool.
    pub yolo_flag: Option<&'static str>,
    /// How the prompt is passed: as a trailing positional, or behind a flag.
    pub prompt_flag: Option<&'static str>,
    pub caps: Caps,
    pub models: &'static [ModelSpec],
    /// How to install or sign in, when the CLI is missing. Taken from each
    /// project's own published instructions, not invented.
    pub setup: Option<agent_core::Setup>,
}

impl CliSpec {
    /// Argv for one turn.
    pub fn args(
        &self,
        prompt: &str,
        cwd: &str,
        model: Option<&str>,
        resume: bool,
        permission: PermissionMode,
    ) -> Vec<String> {
        let mut a: Vec<String> = self.base_args.iter().map(|s| s.to_string()).collect();
        if let Some(f) = self.cwd_flag {
            a.push(f.into());
            a.push(cwd.into());
        }
        if let (Some(f), Some(m)) = (self.model_flag, model) {
            a.push(f.into());
            a.push(m.into());
        }
        if resume {
            if let Some(f) = self.continue_flag {
                a.push(f.into());
            }
        }
        // Anything but `Ask` means the user has accepted unattended tool use;
        // these CLIs have no per-call hook for us to answer.
        if permission != PermissionMode::Ask {
            if let Some(f) = self.yolo_flag {
                a.push(f.into());
            }
        }
        match self.prompt_flag {
            Some(f) => {
                a.push(f.into());
                a.push(prompt.into());
            }
            None => a.push(prompt.into()),
        }
        a
    }
}

/// Caps for a CLI we drive one turn at a time over plain text.
const PLAIN_CAPS: Caps = Caps {
    native_mcp: false,
    native_skills: false,
    native_resume: false,
    permission_hook: false,
    images: false,
    streaming_tools: false,
    usage: false,
    soft_interrupt: false,
    queues_input: false,
    structured: false,
};

pub const OPENCODE: CliSpec = CliSpec {
    id: "opencode",
    display_name: "OpenCode",
    bin: "opencode",
    wire: Wire::OpenCodeJson,
    base_args: &["run", "--format", "json"],
    cwd_flag: Some("--dir"),
    model_flag: Some("--model"),
    continue_flag: Some("--continue"),
    yolo_flag: Some("--auto"),
    prompt_flag: None,
    caps: Caps {
        // Verified: emits per-part events with token counts and a session id.
        structured: true,
        usage: true,
        native_resume: true,
        native_mcp: true,
        streaming_tools: false,
        permission_hook: false,
        native_skills: false,
        images: false,
        soft_interrupt: false,
        queues_input: false,
    },
    models: &[],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g opencode-ai",
        source: "opencode.ai",
    }),
};

pub const VIBE: CliSpec = CliSpec {
    id: "vibe",
    display_name: "Vibe",
    bin: "vibe",
    wire: Wire::PlainText,
    // `--output streaming` exists but its event schema is unverified here, so
    // text mode is used until it can be observed against the real CLI.
    base_args: &["--output", "text"],
    cwd_flag: Some("--workdir"),
    model_flag: None,
    continue_flag: Some("--resume"),
    yolo_flag: Some("--auto-approve"),
    prompt_flag: Some("-p"),
    caps: PLAIN_CAPS,
    models: &[],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g @vibe-kit/vibe",
        source: "the Vibe docs",
    }),
};

pub const GROK: CliSpec = CliSpec {
    id: "grok",
    display_name: "Grok",
    bin: "grok",
    wire: Wire::PlainText,
    base_args: &["--output-format", "plain"],
    cwd_flag: Some("--cwd"),
    model_flag: None,
    continue_flag: Some("--continue"),
    yolo_flag: Some("--always-approve"),
    prompt_flag: Some("-p"),
    caps: PLAIN_CAPS,
    models: &[],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g @vibe-kit/grok-cli",
        source: "the grok-cli README",
    }),
};

pub const CRUSH: CliSpec = CliSpec {
    id: "crush",
    display_name: "Crush",
    bin: "crush",
    wire: Wire::PlainText,
    base_args: &["run"],
    cwd_flag: Some("--cwd"),
    model_flag: None,
    continue_flag: Some("--continue"),
    yolo_flag: Some("--yolo"),
    prompt_flag: None,
    caps: PLAIN_CAPS,
    models: &[],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g @charmland/crush",
        source: "charmbracelet/crush",
    }),
};

pub const GEMINI: CliSpec = CliSpec {
    id: "gemini",
    display_name: "Gemini",
    bin: "gemini",
    wire: Wire::PlainText,
    base_args: &[],
    cwd_flag: None,
    model_flag: Some("--model"),
    continue_flag: None,
    yolo_flag: Some("--yolo"),
    prompt_flag: Some("--prompt"),
    caps: PLAIN_CAPS,
    models: &[
        ModelSpec {
            id: "gemini-3-pro",
            label: "Gemini 3 Pro",
            note: "most capable",
        },
        ModelSpec {
            id: "gemini-3-flash",
            label: "Gemini 3 Flash",
            note: "fastest",
        },
    ],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g @google/gemini-cli",
        source: "the Gemini CLI README",
    }),
};

pub const CODEX: CliSpec = CliSpec {
    id: "codex",
    display_name: "Codex",
    bin: "codex",
    wire: Wire::PlainText,
    base_args: &["exec"],
    cwd_flag: Some("--cd"),
    model_flag: Some("--model"),
    continue_flag: None,
    yolo_flag: Some("--dangerously-bypass-approvals-and-sandbox"),
    prompt_flag: None,
    caps: PLAIN_CAPS,
    models: &[],
    setup: Some(agent_core::Setup {
        verb: "Install",
        command: "npm install -g @openai/codex",
        source: "the Codex README",
    }),
};

pub const ALL: &[&CliSpec] = &[&OPENCODE, &VIBE, &GROK, &CRUSH, &GEMINI, &CODEX];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_argv_matches_the_verified_cli_surface() {
        let a = OPENCODE.args(
            "hi",
            "/repo",
            Some("anthropic/x"),
            true,
            PermissionMode::Yolo,
        );
        assert_eq!(
            a,
            [
                "run",
                "--format",
                "json",
                "--dir",
                "/repo",
                "--model",
                "anthropic/x",
                "--continue",
                "--auto",
                "hi"
            ]
        );
    }

    #[test]
    fn prompt_is_positional_or_flagged_per_cli() {
        // crush takes the prompt positionally...
        let a = CRUSH.args("do it", "/r", None, false, PermissionMode::Ask);
        assert_eq!(a.last().unwrap(), "do it");
        assert!(!a.contains(&"-p".to_string()));

        // ...vibe takes it behind -p.
        let a = VIBE.args("do it", "/r", None, false, PermissionMode::Ask);
        let i = a.iter().position(|x| x == "-p").unwrap();
        assert_eq!(a[i + 1], "do it");
    }

    #[test]
    fn ask_mode_never_passes_a_yolo_flag() {
        for spec in ALL {
            let a = spec.args("x", "/r", None, false, PermissionMode::Ask);
            if let Some(flag) = spec.yolo_flag {
                assert!(
                    !a.contains(&flag.to_string()),
                    "{} leaked {flag} in Ask mode",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn non_ask_modes_do_pass_the_approval_flag() {
        for spec in ALL {
            let a = spec.args("x", "/r", None, false, PermissionMode::Yolo);
            if let Some(flag) = spec.yolo_flag {
                assert!(a.contains(&flag.to_string()), "{} missing {flag}", spec.id);
            }
        }
    }

    #[test]
    fn resume_flag_only_appears_when_resuming() {
        let a = OPENCODE.args("x", "/r", None, false, PermissionMode::Ask);
        assert!(!a.contains(&"--continue".to_string()));
        let a = OPENCODE.args("x", "/r", None, true, PermissionMode::Ask);
        assert!(a.contains(&"--continue".to_string()));
    }

    #[test]
    fn model_flag_is_omitted_when_no_model_is_chosen() {
        let a = GEMINI.args("x", "/r", None, false, PermissionMode::Ask);
        assert!(!a.contains(&"--model".to_string()));
    }

    #[test]
    fn plain_text_backends_declare_themselves_unstructured() {
        // The UI marks these sessions degraded; that must not drift silently.
        for spec in ALL {
            if spec.wire == Wire::PlainText {
                assert!(!spec.caps.structured, "{} claims structured", spec.id);
                assert!(!spec.caps.permission_hook, "{}", spec.id);
            }
        }
    }

    #[test]
    fn every_spec_has_a_unique_id() {
        let mut ids: Vec<&str> = ALL.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate agent id");
    }
}
