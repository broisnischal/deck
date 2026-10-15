//! Which agents are installed, and which are usable right now.

use agent_core::{AgentAdapter, AgentId, Availability, Caps};
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentEntry {
    pub adapter: Arc<dyn AgentAdapter>,
    pub availability: Availability,
}

impl AgentEntry {
    pub fn id(&self) -> AgentId {
        self.adapter.id()
    }

    pub fn display_name(&self) -> &str {
        self.adapter.display_name()
    }

    pub fn caps(&self) -> Caps {
        self.adapter.caps()
    }

    pub fn models(&self) -> &'static [agent_core::ModelSpec] {
        self.adapter.models()
    }

    pub fn is_ready(&self) -> bool {
        self.availability.is_ready()
    }

    /// How to install or sign in, when this backend is not usable.
    pub fn setup(&self) -> Option<agent_core::Setup> {
        self.adapter.setup()
    }
}

#[derive(Clone, Default)]
pub struct Registry {
    entries: Vec<AgentEntry>,
}

impl Registry {
    /// Probe every adapter concurrently and keep the results.
    ///
    /// Unavailable agents are retained rather than dropped: the picker shows
    /// them greyed out with the reason, which is far better UX than an agent
    /// silently missing from the list.
    pub async fn probe_all(adapters: Vec<Arc<dyn AgentAdapter>>) -> Self {
        let probes = adapters.into_iter().map(|adapter| async move {
            let availability = match adapter.probe().await {
                Ok(a) => a,
                Err(e) => Availability::Missing {
                    hint: e.to_string(),
                },
            };
            AgentEntry {
                adapter,
                availability,
            }
        });
        Self {
            entries: futures::future::join_all(probes).await,
        }
    }

    pub fn all(&self) -> &[AgentEntry] {
        &self.entries
    }

    pub fn ready(&self) -> impl Iterator<Item = &AgentEntry> {
        self.entries.iter().filter(|e| e.is_ready())
    }

    pub fn get(&self, id: &AgentId) -> Option<&AgentEntry> {
        self.entries.iter().find(|e| &e.id() == id)
    }

    /// The agent a new session should default to: the first ready one, falling
    /// back to the first known one so the UI always has something selected.
    pub fn default_agent(&self) -> Option<&AgentEntry> {
        self.ready().next().or_else(|| self.entries.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentError, AgentSession, SessionCtx};
    use async_trait::async_trait;

    struct Stub {
        id: &'static str,
        available: Availability,
    }

    #[async_trait]
    impl AgentAdapter for Stub {
        fn id(&self) -> AgentId {
            AgentId::new(self.id)
        }
        fn display_name(&self) -> &str {
            self.id
        }
        fn caps(&self) -> Caps {
            Caps::FULL
        }
        async fn probe(&self) -> Result<Availability, AgentError> {
            Ok(match &self.available {
                Availability::Ready { version } => Availability::Ready {
                    version: version.clone(),
                },
                Availability::NeedsAuth { hint } => Availability::NeedsAuth { hint: hint.clone() },
                Availability::Missing { hint } => Availability::Missing { hint: hint.clone() },
            })
        }
        async fn spawn(&self, _c: SessionCtx) -> Result<Box<dyn AgentSession>, AgentError> {
            Err(AgentError::Unsupported("stub"))
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    fn registry() -> Registry {
        rt().block_on(Registry::probe_all(vec![
            Arc::new(Stub {
                id: "missing",
                available: Availability::Missing {
                    hint: "no bin".into(),
                },
            }),
            Arc::new(Stub {
                id: "ready",
                available: Availability::Ready { version: None },
            }),
        ]))
    }

    #[test]
    fn unavailable_agents_are_kept_so_the_picker_can_explain_them() {
        let r = registry();
        assert_eq!(r.all().len(), 2);
        assert_eq!(r.ready().count(), 1);
        let missing = r.get(&AgentId::new("missing")).unwrap();
        assert!(matches!(
            &missing.availability,
            Availability::Missing { hint } if hint == "no bin"
        ));
    }

    #[test]
    fn default_agent_prefers_a_ready_one_over_document_order() {
        let r = registry();
        assert_eq!(r.default_agent().unwrap().id(), AgentId::new("ready"));
    }

    #[test]
    fn default_agent_falls_back_when_nothing_is_ready() {
        let r = rt().block_on(Registry::probe_all(vec![Arc::new(Stub {
            id: "only",
            available: Availability::NeedsAuth {
                hint: "login".into(),
            },
        })]));
        assert_eq!(r.default_agent().unwrap().id(), AgentId::new("only"));
    }
}
