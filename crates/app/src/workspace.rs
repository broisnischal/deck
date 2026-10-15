//! The window: vertical tabs on the left, the active tab in the middle (a
//! split tree of terminal and browser panes), the inspector on the right, and
//! the control-plane dispatcher that lets anything in a pane drive the rest.

use crate::browser::{BrowserEvent, BrowserPane};
use crate::chat::{ChatEnv, ChatEvent, ChatPane, ChatStatus};
use crate::control::{self, Reply};
use crate::icon::{icon, shimmer, spinner, thinking, Icon};
use crate::inspect;
use crate::inspector::{self, Inspector};
use crate::launcher::{Choice, LauncherEvent, LauncherPane, Pick};
use crate::settings_pane::{SettingsEvent, SettingsPane};
use crate::terminal::{TerminalEvent, TerminalPane};
use crate::theme::{self, u};
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    actions, div, px, rgb, rgba, AnyElement, AnyView, App, Context, ElementId, Entity, FontWeight,
    KeyBinding, Menu, MenuItem, MouseButton, OsAction, SharedString, StyleRefinement, Subscription,
    Task, WeakEntity, Window,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    deck,
    [
        NewTab,
        NewTerminal,
        NewBrowser,
        NewChat,
        OpenSettings,
        ToggleAgents,
        SplitRight,
        SplitDown,
        ClosePane,
        NextTab,
        PrevTab,
        NextPane,
        PrevPane,
        FocusAddress,
        Reload,
        ToggleDevTools,
        ToggleSidebar,
        ToggleInspector,
        Quit,
        Copy,
        Cut,
        Paste,
        SelectAll,
        Undo,
        Redo,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ClearTerminal,
        Find,
        FindNext,
        FindPrevious,
        Tab1,
        Tab2,
        Tab3,
        Tab4,
        Tab5,
        Tab6,
        Tab7,
        Tab8,
        Tab9
    ]
);

pub fn install(cx: &mut App) {
    let mut keys = vec![
        KeyBinding::new("cmd-t", NewTab, None),
        KeyBinding::new("alt-cmd-t", NewTerminal, None),
        KeyBinding::new("cmd-n", NewChat, None),
        KeyBinding::new("cmd-shift-t", NewBrowser, None),
        KeyBinding::new("cmd-shift-a", ToggleAgents, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-d", SplitRight, None),
        KeyBinding::new("cmd-shift-d", SplitDown, None),
        KeyBinding::new("cmd-w", ClosePane, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PrevTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("cmd-]", NextPane, None),
        KeyBinding::new("cmd-[", PrevPane, None),
        KeyBinding::new("cmd-l", FocusAddress, None),
        KeyBinding::new("cmd-r", Reload, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-i", ToggleInspector, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        KeyBinding::new("cmd-k", ClearTerminal, Some("Terminal")),
        KeyBinding::new("cmd-f", Find, Some("Terminal")),
        KeyBinding::new("cmd-g", FindNext, Some("Terminal")),
        KeyBinding::new("cmd-shift-g", FindPrevious, Some("Terminal")),
        // Anywhere else, find goes to the focused page.
        KeyBinding::new("cmd-f", Find, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrevious, None),
        KeyBinding::new("alt-cmd-i", ToggleDevTools, None),
    ];
    for ctx in ["Terminal", "Field"] {
        keys.push(KeyBinding::new("cmd-c", Copy, Some(ctx)));
        keys.push(KeyBinding::new("cmd-v", Paste, Some(ctx)));
    }
    keys.push(KeyBinding::new("cmd-x", Cut, Some("Field")));
    keys.push(KeyBinding::new("cmd-a", SelectAll, Some("Field")));
    keys.push(KeyBinding::new("cmd-a", SelectAll, Some("Terminal")));
    keys.push(KeyBinding::new("cmd-z", Undo, Some("Field")));
    keys.push(KeyBinding::new("cmd-shift-z", Redo, Some("Field")));
    keys.extend([
        KeyBinding::new("cmd-1", Tab1, None),
        KeyBinding::new("cmd-2", Tab2, None),
        KeyBinding::new("cmd-3", Tab3, None),
        KeyBinding::new("cmd-4", Tab4, None),
        KeyBinding::new("cmd-5", Tab5, None),
        KeyBinding::new("cmd-6", Tab6, None),
        KeyBinding::new("cmd-7", Tab7, None),
        KeyBinding::new("cmd-8", Tab8, None),
        KeyBinding::new("cmd-9", Tab9, None),
    ]);
    cx.bind_keys(keys);
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Menus are not decoration here: AppKit routes ⌘ shortcuts through them,
    // so they keep working while a webview holds the keyboard.
    cx.set_menus(vec![
        Menu {
            name: "Deck".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Deck", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Chat", NewChat),
                MenuItem::action("New Tab…", NewTab),
                MenuItem::action("New Terminal Tab", NewTerminal),
                MenuItem::action("New Browser Tab", NewBrowser),
                MenuItem::action("New Agent…", ToggleAgents),
                MenuItem::separator(),
                MenuItem::action("Close", ClosePane),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", Find),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Inspector", ToggleInspector),
                MenuItem::separator(),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Actual Size", ZoomReset),
                MenuItem::separator(),
                MenuItem::action("Split Right", SplitRight),
                MenuItem::action("Split Down", SplitDown),
                MenuItem::separator(),
                MenuItem::action("Open Location…", FocusAddress),
                MenuItem::action("Reload Page", Reload),
                MenuItem::action("Web Inspector", ToggleDevTools),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
                MenuItem::action("Next Pane", NextPane),
                MenuItem::action("Previous Pane", PrevPane),
                // Every global shortcut needs a menu item: a focused web page
                // takes ⌘-keys first and hands back only what the menu claims.
                MenuItem::separator(),
                MenuItem::action("Control", Tab1),
                MenuItem::action("Tab 2", Tab2),
                MenuItem::action("Tab 3", Tab3),
                MenuItem::action("Tab 4", Tab4),
                MenuItem::action("Tab 5", Tab5),
                MenuItem::action("Tab 6", Tab6),
                MenuItem::action("Tab 7", Tab7),
                MenuItem::action("Tab 8", Tab8),
                MenuItem::action("Last Tab", Tab9),
            ],
        },
    ]);
}

/// Coding agents we know how to start. Only the installed ones are offered.
pub struct Agent {
    pub name: &'static str,
    pub cmd: &'static str,
}

pub const AGENTS: &[Agent] = &[
    Agent {
        name: "Claude Code",
        cmd: "claude",
    },
    Agent {
        name: "Codex",
        cmd: "codex",
    },
    Agent {
        name: "Gemini CLI",
        cmd: "gemini",
    },
    Agent {
        name: "OpenCode",
        cmd: "opencode",
    },
    Agent {
        name: "Cursor Agent",
        cmd: "cursor-agent",
    },
    Agent {
        name: "Grok",
        cmd: "grok",
    },
    Agent {
        name: "Amp",
        cmd: "amp",
    },
    Agent {
        name: "Aider",
        cmd: "aider",
    },
];

pub type PaneId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Leaf(PaneId),
    Split(Axis, Vec<Node>),
}

impl Node {
    /// Put `new` beside `target`. Joins the parent run when it already flows
    /// the same way, as i3 does, so three splits right are three equal columns
    /// rather than a staircase.
    fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split(axis, vec![Node::Leaf(target), Node::Leaf(new)]);
                true
            }
            Node::Leaf(_) => false,
            Node::Split(a, children) => {
                if *a == axis {
                    if let Some(i) = children.iter().position(|c| *c == Node::Leaf(target)) {
                        children.insert(i + 1, Node::Leaf(new));
                        return true;
                    }
                }
                children.iter_mut().any(|c| c.split(target, new, axis))
            }
        }
    }

    /// Remove a leaf from a split, collapsing any split left with one child.
    fn remove(&mut self, target: PaneId) -> bool {
        let Node::Split(_, children) = self else {
            return false;
        };
        let before = children.len();
        children.retain(|c| *c != Node::Leaf(target));
        let removed = children.len() != before || children.iter_mut().any(|c| c.remove(target));
        if children.len() == 1 {
            *self = children.pop().expect("one child");
        }
        removed
    }

    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split(_, children) => children.iter().for_each(|c| c.leaves(out)),
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(x) => *x == id,
            Node::Split(_, children) => children.iter().any(|c| c.contains(id)),
        }
    }

    /// Put `new` where `old` is, keeping the layout around it.
    fn replace(&mut self, old: PaneId, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == old => {
                *id = new;
                true
            }
            Node::Leaf(_) => false,
            Node::Split(_, children) => children.iter_mut().any(|c| c.replace(old, new)),
        }
    }
}

enum PaneKind {
    Terminal(Entity<TerminalPane>),
    Browser(Entity<BrowserPane>),
    Chat(Entity<ChatPane>),
    /// The one Settings tab (⌘,).
    Settings(Entity<SettingsPane>),
    /// A new tab that has not been told what to be yet (⌘T).
    Launcher(Entity<LauncherPane>),
}

struct Pane {
    kind: PaneKind,
    /// Terminals and chats get one: they are where callers run.
    token: Option<String>,
    agent: Option<&'static Agent>,
    _sub: Subscription,
}

struct Tab {
    root: Node,
    focused: PaneId,
    /// The control chat: first in the sidebar, never closed by accident.
    pinned: bool,
}

impl Tab {
    fn leaves(&self) -> Vec<PaneId> {
        let mut v = Vec::new();
        self.root.leaves(&mut v);
        v
    }
}

pub enum Open {
    Terminal {
        cmd: Option<String>,
        cwd: Option<PathBuf>,
    },
    Browser {
        url: String,
    },
    /// A structured chat with an agent (by registry id, e.g. "claude"); `None`
    /// picks the default one.
    Chat {
        agent: Option<String>,
        cwd: Option<PathBuf>,
    },
    /// A chat thread saved by an earlier launch.
    Thread(Box<crate::threads::Saved>),
    Settings,
    Launcher,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Working,
    Idle,
    Attention,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// Chats and agent TUIs: things doing work for you.
    Threads,
    /// Terminals and pages.
    Workspace,
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    panes: HashMap<PaneId, Pane>,
    next_id: PaneId,
    cwd: PathBuf,
    installed: Vec<&'static Agent>,
    picker: bool,
    sidebar: bool,
    pub inspector: Inspector,
    refreshing: bool,
    /// Agent runtime for chats; probed in the background at startup.
    chats: Option<Arc<ChatEnv>>,
    /// Set once that probe has finished, whatever it found.
    probed: bool,
    settings: crate::settings::Settings,
    /// Last agent statuses drawn, so the 1s tick only repaints on a change.
    statuses: Vec<(PaneId, Status)>,
    /// Threads that finished while you were looking elsewhere: "Done".
    unseen: std::collections::HashSet<PaneId>,
    ticks: u64,
    /// Set by whoever launched the app (tests, scripts): a token with a
    /// pane's rights, acting as the focused pane.
    automation: Option<String>,
    control: Option<control::Server>,
    /// The right-click menu of a sidebar row, while open.
    menu: Option<crate::menu::Open>,
    /// The sidebar and the inspector, each its own cached view.
    islands: Option<(Entity<Island>, Entity<Island>)>,
    /// Where each terminal and chat is working, refreshed by the tick.
    places: HashMap<PaneId, Place>,
    /// Terminals running a command that is printing right now.
    busy: std::collections::HashSet<PaneId>,
    _requests: Option<Task<()>>,
    _tick: Task<()>,
}

impl Workspace {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if this
                .update_in(cx, |ws, window, cx| ws.tick(window, cx))
                .is_err()
            {
                return;
            }
        });
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            panes: HashMap::new(),
            next_id: 1,
            cwd,
            installed: AGENTS
                .iter()
                .filter(|a| term::which(a.cmd).is_some())
                .collect(),
            picker: false,
            sidebar: true,
            inspector: Inspector {
                // A narrow (tiled) window cannot afford a third column.
                open: f32::from(window.viewport_size().width) >= 1200.,
                ..Default::default()
            },
            refreshing: false,
            chats: None,
            probed: false,
            settings: {
                let s = crate::settings::load();
                crate::browser::set_search_engine(s.search);
                s
            },
            statuses: Vec::new(),
            unseen: Default::default(),
            islands: None,
            places: HashMap::new(),
            busy: Default::default(),
            ticks: 0,
            automation: std::env::var("DECK_AUTOMATION_TOKEN")
                .ok()
                .filter(|t| t.len() >= 16),
            control: None,
            menu: None,
            _requests: None,
            _tick: tick,
        };
        // Probing every agent CLI takes a moment; the window does not wait.
        // When it is ready, the control chat opens as the first tab.
        cx.spawn_in(window, async move |this, cx| {
            let (env, saved) = cx
                .background_executor()
                .spawn(async { (ChatEnv::start(), crate::threads::load_all()) })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                match env {
                    Ok(env) => ws.chats = Some(Arc::new(env)),
                    Err(e) => tracing::error!("agent runtime unavailable: {e}"),
                }
                ws.probed = true;
                let agents = ws.ready_agents();
                let settings = ws.settings.clone();
                for pane in ws.panes.values() {
                    if let PaneKind::Settings(s) = &pane.kind {
                        let agents = agents.clone();
                        s.update(cx, |s, cx| s.sync(&settings, Some(agents), cx));
                    }
                }
                ws.open_control(saved, window, cx);
            });
        })
        .detach();
        match control::listen() {
            Ok((server, mut rx)) => {
                tracing::info!(socket = %server.path.display(), "control plane listening");
                ws.control = Some(server);
                ws._requests = Some(cx.spawn_in(window, async move |this, cx| {
                    while let Some((req, reply)) = rx.next().await {
                        let res =
                            this.update_in(cx, |ws, window, cx| ws.handle(req, reply, window, cx));
                        if res.is_err() {
                            return;
                        }
                    }
                }));
            }
            Err(e) => tracing::error!("control plane unavailable: {e}"),
        }
        ws
    }

    // -- panes and tabs -----------------------------------------------------

    /// Create a pane (not yet placed in any tab).
    fn make_pane(
        &mut self,
        what: Open,
        agent: Option<&'static Agent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<PaneId> {
        let id = self.next_id;
        // The fallible half (PTY, webview) happens before `cx.new`, which
        // cannot return an error.
        let (kind, token, sub) = match what {
            Open::Terminal { cmd, cwd } => {
                let token = new_token();
                let env = self.pane_env(id, &token);
                let cwd = cwd.unwrap_or_else(|| self.cwd.clone());
                let pty = TerminalPane::spawn(&cwd, cmd.as_deref(), env)?;
                let title = agent
                    .map(|a| a.name.to_string())
                    .or(cmd)
                    .or_else(|| {
                        std::env::var("SHELL")
                            .ok()
                            .map(|s| s.rsplit('/').next().unwrap_or("shell").to_string())
                    })
                    .unwrap_or_else(|| "shell".into());
                let look = self.settings.term_look();
                let t = cx.new(|cx| TerminalPane::new(pty, title, look, window, cx));
                let sub =
                    cx.subscribe_in(&t, window, move |ws, _, ev: &TerminalEvent, window, cx| {
                        match ev {
                            TerminalEvent::Focused => ws.mark_focused(id, cx),
                            // The sidebar draws only when the workspace
                            // notifies, so a new title has to say so.
                            TerminalEvent::Retitled => cx.notify(),
                            TerminalEvent::Exited => ws.close_pane(id, window, cx),
                            TerminalEvent::OpenUrl(url) => {
                                let url = url.clone();
                                ws.open_url_near(id, &url, window, cx);
                            }
                        }
                    });
                (PaneKind::Terminal(t), Some(token), sub)
            }
            Open::Chat { .. } | Open::Thread(_) => {
                let env = self.chats.clone().ok_or_else(|| {
                    anyhow::anyhow!("agents are still being detected; try again in a moment")
                })?;
                let token = new_token();
                let agent_env: Vec<(String, String)> =
                    self.pane_env(id, &token).into_iter().collect();
                let c = match what {
                    Open::Thread(saved) => {
                        cx.new(|cx| ChatPane::restore(env, *saved, agent_env, cx))
                    }
                    Open::Chat { agent, cwd } => {
                        let agent = match agent {
                            Some(a) => agent_core::AgentId::new(a),
                            None => self.default_agent().ok_or_else(|| {
                                anyhow::anyhow!("no agent CLI is installed and signed in")
                            })?,
                        };
                        let cwd = cwd.unwrap_or_else(|| self.cwd.clone());
                        let c = cx.new(|cx| ChatPane::new(env, agent, cwd, agent_env, cx));
                        let mode = self.settings.access.mode();
                        c.update(cx, |c, _| c.set_mode(mode));
                        c
                    }
                    _ => unreachable!(),
                };
                let sub =
                    cx.subscribe_in(
                        &c,
                        window,
                        move |ws, _, ev: &ChatEvent, window, cx| match ev {
                            ChatEvent::Focused => ws.mark_focused(id, cx),
                            ChatEvent::OpenUrl(url) => {
                                let url = url.clone();
                                ws.open_url_near(id, &url, window, cx);
                            }
                        },
                    );
                (PaneKind::Chat(c), Some(token), sub)
            }
            Open::Browser { url } => {
                let (webview, events, url) = BrowserPane::build(&url, window)?;
                let b = cx.new(|cx| BrowserPane::new(webview, events, url, window, cx));
                let zoom = self.settings.zoom as f64;
                b.update(cx, |b, _| b.set_zoom(zoom));
                let sub =
                    cx.subscribe_in(&b, window, move |ws, _, ev: &BrowserEvent, window, cx| {
                        match ev {
                            BrowserEvent::Focused => {
                                // The page has the keyboard now; drop gpui's focus so
                                // no field or terminal still looks focused.
                                window.blur();
                                ws.mark_focused(id, cx);
                            }
                            BrowserEvent::OpenTab(url) => {
                                let url = url.clone();
                                let _ = ws.new_tab(Open::Browser { url }, None, window, cx);
                            }
                            BrowserEvent::Retitled => cx.notify(),
                            BrowserEvent::Close => ws.close_pane(id, window, cx),
                        }
                    });
                (PaneKind::Browser(b), None, sub)
            }
            Open::Settings => self.make_settings(window, cx),
            Open::Launcher => self.make_launcher(id, window, cx),
        };
        self.next_id += 1;
        self.panes.insert(
            id,
            Pane {
                kind,
                token,
                agent,
                _sub: sub,
            },
        );
        Ok(id)
    }

    /// Put the control chat first. Without any agent there is nothing to talk
    /// to, so the window opens on a terminal instead of an empty screen.
    /// Open the pinned control chat: the one saved last time if there is
    /// one, else a new one. Other saved threads come back as tabs after it.
    fn open_control(
        &mut self,
        mut saved: Vec<crate::threads::Saved>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.iter().any(|t| t.pinned) {
            return;
        }
        let control = saved
            .iter()
            .rposition(|s| s.control)
            .map(|i| saved.remove(i));
        let agent = self.default_agent();
        let what = match control {
            Some(s) => Some(Open::Thread(Box::new(s))),
            None => agent.map(|a| Open::Chat {
                agent: Some(a.as_str().into()),
                cwd: None,
            }),
        };
        let made = what.and_then(|what| {
            self.make_pane(what, None, window, cx)
                .map_err(|e| tracing::error!("control chat: {e}"))
                .ok()
        });
        match made {
            Some(id) => {
                if let Some(Pane {
                    kind: PaneKind::Chat(c),
                    ..
                }) = self.panes.get(&id)
                {
                    c.update(cx, |c, _| c.control = true);
                }
                let fresh = self.tabs.is_empty();
                self.tabs.insert(
                    0,
                    Tab {
                        root: Node::Leaf(id),
                        focused: id,
                        pinned: true,
                    },
                );
                // Don't yank focus from something the user already opened.
                if fresh {
                    self.focus_pane(id, window, cx);
                } else {
                    self.active += 1;
                }
            }
            None if self.tabs.is_empty() => {
                if let Err(e) = self.new_tab(
                    Open::Terminal {
                        cmd: None,
                        cwd: None,
                    },
                    None,
                    window,
                    cx,
                ) {
                    tracing::error!("could not start a shell: {e}");
                }
            }
            None => {}
        }
        for mut s in saved {
            // Only one control chat; an older one is an ordinary thread now.
            s.control = false;
            match self.make_pane(Open::Thread(Box::new(s)), None, window, cx) {
                Ok(id) => self.tabs.push(Tab {
                    root: Node::Leaf(id),
                    focused: id,
                    pinned: false,
                }),
                Err(e) => tracing::warn!("could not restore a thread: {e}"),
            }
        }
        cx.notify();
    }

    fn control_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.pinned)
    }

    fn new_tab(
        &mut self,
        what: Open,
        agent: Option<&'static Agent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<PaneId> {
        let id = self.make_pane(what, agent, window, cx)?;
        self.tabs.push(Tab {
            root: Node::Leaf(id),
            focused: id,
            pinned: false,
        });
        self.picker = false;
        self.focus_pane(id, window, cx);
        Ok(id)
    }

    fn split(
        &mut self,
        beside: PaneId,
        what: Open,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<PaneId> {
        let Some(ix) = self.tab_of(beside) else {
            return self.new_tab(what, None, window, cx);
        };
        let id = self.make_pane(what, None, window, cx)?;
        self.tabs[ix].root.split(beside, id, axis);
        self.focus_pane(id, window, cx);
        Ok(id)
    }

    fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.root.contains(pane))
    }

    fn focused(&self) -> Option<PaneId> {
        self.tabs.get(self.active).map(|t| t.focused)
    }

    fn pane_env(&self, id: PaneId, token: &str) -> HashMap<String, String> {
        let mut env = HashMap::new();
        if let Some(server) = &self.control {
            env.insert("DECK_SOCKET".into(), server.path.display().to_string());
            env.insert("DECK_TOKEN".into(), token.to_string());
        }
        env.insert("DECK_PANE".into(), id.to_string());
        env.insert("TERM_PROGRAM".into(), "deck".into());
        // Panes get their own token; the launcher's must not leak into them.
        env.insert("DECK_AUTOMATION_TOKEN".into(), String::new());
        // Launched from Finder there is no locale at all, and zsh then treats
        // every non-ASCII byte as garbage. Never override one the user set.
        if ["LANG", "LC_ALL", "LC_CTYPE"]
            .iter()
            .all(|k| std::env::var_os(k).is_none())
        {
            env.insert("LANG".into(), "en_US.UTF-8".into());
        }
        if let Ok(exe) = std::env::current_exe() {
            // So `deck …` resolves to this very binary inside its own panes,
            // and tools that honour $BROWSER (gh, python, cargo doc --open)
            // open links in a pane here instead of another app.
            if let Some(dir) = exe.parent() {
                let path = std::env::var("PATH").unwrap_or_default();
                env.insert("PATH".into(), format!("{}:{path}", dir.display()));
            }
            env.insert("BROWSER".into(), format!("{} open", exe.display()));
        }
        env
    }

    /// Record focus that already happened natively (a click in the pane).
    fn mark_focused(&mut self, id: PaneId, cx: &mut Context<Self>) {
        if let Some(ix) = self.tab_of(id) {
            if self.tabs[ix].focused != id || self.active != ix {
                self.tabs[ix].focused = id;
                self.active = ix;
                self.refresh_inspector(cx);
                cx.notify();
            }
        }
    }

    fn focus_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tab_of(id) else {
            return;
        };
        for p in self.tabs[ix].leaves() {
            self.unseen.remove(&p);
        }
        self.active = ix;
        self.tabs[ix].focused = id;
        match self.panes.get(&id).map(|p| &p.kind) {
            Some(PaneKind::Terminal(t)) => {
                window.focus(&t.read(cx).focus);
                crate::browser::reclaim_keyboard(window);
            }
            Some(PaneKind::Browser(b)) => {
                b.update(cx, |b, _| b.set_visible(true));
                window.blur();
                b.read(cx).focus_native();
            }
            Some(PaneKind::Chat(c)) => {
                window.focus(&c.read(cx).composer.read(cx).focus);
                crate::browser::reclaim_keyboard(window);
            }
            Some(PaneKind::Settings(s)) => {
                window.focus(&s.read(cx).focus);
                crate::browser::reclaim_keyboard(window);
            }
            Some(PaneKind::Launcher(l)) => {
                window.focus(&l.read(cx).focus);
                crate::browser::reclaim_keyboard(window);
            }
            None => {}
        }
        self.inspector.data = Default::default();
        self.refresh_inspector(cx);
        cx.notify();
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get(ix) {
            let id = t.focused;
            self.focus_pane(id, window, cx);
        }
    }

    /// Drop a pane for good. A closed chat thread is archived, not lost.
    fn discard(&mut self, id: PaneId, cx: &mut Context<Self>) {
        if let Some(Pane {
            kind: PaneKind::Chat(c),
            ..
        }) = self.panes.remove(&id)
        {
            c.update(cx, |c, _| c.archive());
        }
    }

    fn close_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tab_of(id) else {
            self.discard(id, cx);
            return;
        };
        // The control chat stays; panes split beside it still close.
        if self.tabs[ix].pinned && self.tabs[ix].root == Node::Leaf(id) {
            return;
        }
        self.discard(id, cx);
        let tab = &mut self.tabs[ix];
        if tab.root == Node::Leaf(id) {
            self.tabs.remove(ix);
            if self.active > ix || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
            }
        } else {
            tab.root.remove(id);
            if tab.focused == id {
                tab.focused = tab.leaves()[0];
            }
        }
        if !self.tabs.is_empty() {
            self.activate(self.active, window, cx);
        }
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        if tab.pinned {
            return;
        }
        for id in tab.leaves() {
            self.discard(id, cx);
        }
        self.tabs.remove(ix);
        if self.active > ix || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if !self.tabs.is_empty() {
            self.activate(self.active, window, cx);
        }
        cx.notify();
    }

    /// Every other tab of the same kind as `keep` (threads or workspace),
    /// pinned ones aside. A terminal's "Close Other Tabs" leaves your chats.
    fn close_other_tabs(&mut self, keep: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(keep) else {
            return;
        };
        let (kept, group) = (tab.focused, self.group(tab));
        for ix in (0..self.tabs.len()).rev() {
            if ix == keep || self.tabs[ix].pinned || self.group(&self.tabs[ix]) != group {
                continue;
            }
            for id in self.tabs[ix].leaves() {
                self.discard(id, cx);
            }
            self.tabs.remove(ix);
        }
        if let Some(ix) = self.tab_of(kept) {
            self.activate(ix, window, cx);
        }
        cx.notify();
    }

    /// A sidebar row's right-click menu. Items find their tab again by pane
    /// when they run, since tabs can open or close while the menu is up.
    fn open_tab_menu(
        &mut self,
        ix: usize,
        mut at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::menu::{Entry, Item};
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        let pane = tab.focused;
        let threads = self.group(tab) == Group::Threads;
        let url = match self.panes.get(&pane).map(|p| &p.kind) {
            Some(PaneKind::Browser(b)) => {
                Some(b.read(cx).url.to_string()).filter(|u| !u.is_empty())
            }
            _ => None,
        };
        let ws = cx.entity().downgrade();
        let mut entries: Vec<Entry> = Vec::new();
        if let Some(url) = url {
            entries.push(
                Item::new("Copy URL", move |_, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()))
                })
                .icon(Icon::Link)
                .into(),
            );
            entries.push(Entry::Separator);
        }
        let close = ws.clone();
        entries.push(
            Item::new(
                if threads { "Close Thread" } else { "Close Tab" },
                move |window, cx| {
                    let _ = close.update(cx, |ws, cx| {
                        if let Some(ix) = ws.tab_of(pane) {
                            ws.close_tab(ix, window, cx);
                        }
                    });
                },
            )
            .icon(Icon::Close)
            .disabled(tab.pinned)
            .into(),
        );
        entries.push(
            Item::new(
                if threads {
                    "Close Other Threads"
                } else {
                    "Close Other Tabs"
                },
                move |window, cx| {
                    let _ = ws.update(cx, |ws, cx| {
                        if let Some(ix) = ws.tab_of(pane) {
                            ws.close_other_tabs(ix, window, cx);
                        }
                    });
                },
            )
            .icon(Icon::CloseOthers)
            .into(),
        );
        // A page on screen is a native view the menu cannot cover, so keep
        // the menu over the sidebar.
        if self.browser_in_tab(self.active).is_some() {
            let zoom = self.settings.zoom;
            let room = (theme::SIDEBAR_PX - crate::menu::MIN_W - theme::GUTTER) * zoom;
            at.x = at.x.min(px(room));
        }
        let restore = window.focused(cx);
        self.menu = Some(crate::menu::open(
            entries,
            at,
            restore,
            window,
            cx,
            |ws: &mut Self, window, cx| {
                ws.menu = None;
                // A page had the keyboard before the menu took it.
                if window.focused(cx).is_none() {
                    if let Some((_, b)) = ws.browser_in_tab(ws.active) {
                        b.read(cx).focus_native();
                    }
                }
            },
        ));
        cx.notify();
    }

    fn focused_browser_pane(&self) -> Option<Entity<BrowserPane>> {
        match &self.panes.get(&self.focused()?)?.kind {
            PaneKind::Browser(b) => Some(b.clone()),
            _ => None,
        }
    }

    fn browser_in_tab(&self, ix: usize) -> Option<(PaneId, Entity<BrowserPane>)> {
        let tab = self.tabs.get(ix)?;
        // Prefer the focused pane, then the first browser in the tab.
        std::iter::once(tab.focused)
            .chain(tab.leaves())
            .find_map(|id| match &self.panes.get(&id)?.kind {
                PaneKind::Browser(b) => Some((id, b.clone())),
                _ => None,
            })
    }

    /// Open `url` next to `pane`: in its tab's browser if it has one, else in a
    /// new browser split beside it. This is what makes a dev server's URL, a
    /// ⌘-clicked link, or `$BROWSER` land right next to the terminal.
    pub fn open_url_near(
        &mut self,
        pane: PaneId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        if let Some((id, b)) = self.tab_of(pane).and_then(|ix| self.browser_in_tab(ix)) {
            let _ = b.update(cx, |b, cx| b.navigate(url, cx));
            self.focus_pane(id, window, cx);
            return Some(id);
        }
        self.split(
            pane,
            Open::Browser { url: url.into() },
            Axis::Row,
            window,
            cx,
        )
        .ok()
    }

    pub fn open_url_beside(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused() {
            Some(p) => {
                self.open_url_near(p, url, window, cx);
            }
            None => {
                let _ = self.new_tab(Open::Browser { url: url.into() }, None, window, cx);
            }
        }
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let b = match self.browser_in_tab(self.active) {
            Some((id, b)) => {
                self.focus_pane(id, window, cx);
                b
            }
            None => {
                let id = match self.focused() {
                    Some(p) => self.split(
                        p,
                        Open::Browser {
                            url: "about:blank".into(),
                        },
                        Axis::Row,
                        window,
                        cx,
                    ),
                    None => self.new_tab(
                        Open::Browser {
                            url: "about:blank".into(),
                        },
                        None,
                        window,
                        cx,
                    ),
                };
                match id.ok().and_then(|id| self.panes.get(&id)) {
                    Some(Pane {
                        kind: PaneKind::Browser(b),
                        ..
                    }) => b.clone(),
                    _ => return,
                }
            }
        };
        b.update(cx, |b, cx| b.focus_address(window, cx));
        self.refresh_servers(cx);
    }

    fn cycle_pane(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let order = tab.leaves();
        let at = order.iter().position(|&p| p == tab.focused).unwrap_or(0) as isize;
        let next = order[(at + step).rem_euclid(order.len() as isize) as usize];
        self.focus_pane(next, window, cx);
    }

    fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let n = self.tabs.len() as isize;
        self.activate(
            (self.active as isize + step).rem_euclid(n) as usize,
            window,
            cx,
        );
    }

    /// Once a second. Does nothing that costs a process spawn or a repaint
    /// unless something visible changed.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ticks += 1;
        // Agent dots go idle on a timer, not on an event; repaint only when
        // one of them actually changes.
        let statuses: Vec<(PaneId, Status)> = self
            .panes
            .keys()
            .filter_map(|&id| self.status(id, cx).map(|s| (id, s)))
            .collect();
        let mut sorted = statuses;
        sorted.sort_by_key(|(id, _)| *id);
        let focused = self.focused();
        for (id, now) in &sorted {
            let was = self.statuses.iter().find(|(p, _)| p == id).map(|(_, s)| *s);
            if was == Some(Status::Working) && *now != Status::Working && focused != Some(*id) {
                self.unseen.insert(*id);
            }
        }
        // Repaint when a state changed, and every second while something is
        // working, so its timer moves.
        let working = sorted.iter().any(|(_, s)| *s == Status::Working);
        if sorted != self.statuses || working {
            self.statuses = sorted;
            cx.notify();
        }
        if window.is_window_active() && self.ticks.is_multiple_of(2) {
            self.refresh_inspector(cx);
        }
        self.refresh_servers(cx);
        self.refresh_places(cx);
    }

    /// The project each terminal and chat is in, and what a terminal is
    /// running, for the tab titles. libproc only.
    fn refresh_places(&mut self, cx: &mut Context<Self>) {
        let mut next = HashMap::new();
        for (&id, pane) in &self.panes {
            let place = match &pane.kind {
                PaneKind::Terminal(t) => {
                    let procs = inspect::descendants(t.read(cx).pid());
                    let cwd = procs.iter().rev().find_map(|p| inspect::cwd(p.pid));
                    cwd.map(|cwd| Place {
                        front: procs.last().map(|p| p.name.clone()),
                        ..place_of(&cwd)
                    })
                }
                PaneKind::Chat(c) => Some(place_of(c.read(cx).cwd())),
                _ => None,
            };
            if let Some(place) = place {
                next.insert(id, place);
            }
        }
        // Busy: something other than the shell is in front and printing.
        let shell = std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_string))
            .unwrap_or_else(|| "zsh".into());
        let busy: std::collections::HashSet<PaneId> = self
            .panes
            .iter()
            .filter(|(id, p)| match &p.kind {
                PaneKind::Terminal(t) => {
                    t.read(cx).last_output.elapsed() < Duration::from_millis(1500)
                        && next
                            .get(id)
                            .and_then(|pl| pl.front.as_deref())
                            .is_some_and(|f| f != shell && !f.starts_with('-'))
                }
                _ => false,
            })
            .map(|(id, _)| *id)
            .collect();
        if next != self.places || busy != self.busy {
            self.places = next;
            self.busy = busy;
            cx.notify();
        }
    }

    /// The sidebar and inspector islands, made on first use.
    fn islands(&mut self, cx: &mut Context<Self>) -> (Entity<Island>, Entity<Island>) {
        if let Some(i) = &self.islands {
            return i.clone();
        }
        let ws = cx.entity();
        let sidebar = Island::new(&ws, |ws, _, cx| ws.render_sidebar(cx), cx);
        let inspector = Island::new(
            &ws,
            |ws, _, cx| {
                let title = ws
                    .focused()
                    .map(|id| ws.pane_title(id, cx))
                    .unwrap_or_default();
                inspector::render(ws, title, cx)
            },
            cx,
        );
        self.islands = Some((sidebar.clone(), inspector.clone()));
        (sidebar, inspector)
    }

    /// Dev servers for any blank browser tab on screen: every port a process
    /// under one of our terminals listens on. libproc only, no spawning.
    fn refresh_servers(&mut self, cx: &mut Context<Self>) {
        let Some((_, b)) = self.browser_in_tab(self.active) else {
            return;
        };
        if !b.read(cx).is_blank() {
            return;
        }
        let pids: Vec<u32> = self
            .panes
            .values()
            .filter_map(|p| match &p.kind {
                PaneKind::Terminal(t) => Some(t.read(cx).pid()),
                _ => None,
            })
            .flat_map(|pid| inspect::descendants(pid).into_iter().map(|p| p.pid))
            .collect();
        let mut servers: Vec<(u16, String)> = inspect::listening_ports(&pids)
            .into_iter()
            .map(|p| (p.port, p.process))
            .collect();
        servers.dedup_by_key(|(port, _)| *port);
        b.update(cx, |b, cx| {
            if b.servers != servers {
                b.servers = servers;
                cx.notify();
            }
        });
    }

    pub fn refresh_inspector(&mut self, cx: &mut Context<Self>) {
        if !self.inspector.open || self.refreshing {
            return;
        }
        let Some(PaneKind::Terminal(t)) = self
            .focused()
            .and_then(|id| self.panes.get(&id))
            .map(|p| &p.kind)
        else {
            return;
        };
        let pid = t.read(cx).pid();
        let fallback = self.cwd.clone();
        let expanded: Vec<PathBuf> = self.inspector.expanded.iter().cloned().collect();
        // git status spawns a process: every 6s, or when the data is fresh.
        let previous = (self.inspector.data.root.is_some() && !self.ticks.is_multiple_of(6))
            .then(|| self.inspector.data.git.clone());
        self.refreshing = true;
        cx.spawn(async move |this, cx| {
            let data = cx
                .background_executor()
                .spawn(async move { inspector::collect(pid, fallback, expanded, previous) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.refreshing = false;
                ws.inspector.data = data;
                cx.notify();
            });
        })
        .detach();
    }

    /// The directory the focused terminal is in (its deepest process: the
    /// foreground job, or the shell), so a new terminal opens there.
    fn focused_cwd(&self, cx: &App) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active)?;
        std::iter::once(tab.focused)
            .chain(tab.leaves())
            .find_map(|id| match &self.panes.get(&id)?.kind {
                PaneKind::Terminal(t) => {
                    let procs = inspect::descendants(t.read(cx).pid());
                    procs.iter().rev().find_map(|p| inspect::cwd(p.pid))
                }
                PaneKind::Launcher(l) => l.read(cx).cwd.clone(),
                PaneKind::Browser(_) | PaneKind::Chat(_) | PaneKind::Settings(_) => None,
            })
    }

    fn shell_here(&self, cx: &App) -> Open {
        Open::Terminal {
            cmd: None,
            cwd: self.focused_cwd(cx),
        }
    }

    /// ⌘+ / ⌘− / ⌘0: one zoom for the whole app (UI, terminals, pages),
    /// wherever the focus is, remembered across launches.
    fn zoom(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.settings.clone();
        next.zoom = self.settings.stepped(step);
        self.apply_settings(next, window, cx);
    }

    /// Take new settings from anywhere (the Settings tab, ⌘+), save them, and
    /// push what changed to every open pane, so nothing waits for a relaunch.
    fn apply_settings(
        &mut self,
        next: crate::settings::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if next == self.settings {
            return;
        }
        let zoomed = next.zoom != self.settings.zoom;
        crate::browser::set_search_engine(next.search);
        self.settings = next;
        crate::settings::save(&self.settings);
        let look = self.settings.term_look();
        let zoom = self.settings.zoom as f64;
        for pane in self.panes.values() {
            match &pane.kind {
                PaneKind::Terminal(t) => {
                    let look = look.clone();
                    t.update(cx, |t, cx| t.set_look(look, window, cx))
                }
                PaneKind::Browser(b) => b.update(cx, |b, _| b.set_zoom(zoom)),
                PaneKind::Settings(s) => {
                    let settings = self.settings.clone();
                    s.update(cx, |s, cx| s.sync(&settings, None, cx))
                }
                PaneKind::Chat(_) | PaneKind::Launcher(_) => {}
            }
        }
        if zoomed {
            // The UI follows through the rem size, set every frame in render.
            // Cached panes would keep their old layout: redraw everything once.
            window.refresh();
        }
        cx.notify();
    }

    /// The agent a new chat starts with: the one chosen in Settings if it is
    /// ready, else Claude if it is, else the first agent that is.
    fn default_agent(&self) -> Option<agent_core::AgentId> {
        let env = self.chats.as_ref()?;
        self.settings
            .default_agent
            .as_deref()
            .map(agent_core::AgentId::new)
            .filter(|id| env.registry.get(id).is_some_and(|e| e.is_ready()))
            .or_else(|| env.default_agent())
    }

    /// Agents the Settings tab offers: those ready to chat right now.
    fn ready_agents(&self) -> Vec<(SharedString, SharedString)> {
        self.chats
            .as_ref()
            .map(|env| {
                env.registry
                    .ready()
                    .map(|e| {
                        (
                            SharedString::from(e.id().as_str().to_string()),
                            SharedString::from(e.display_name().to_string()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn make_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (PaneKind, Option<String>, Subscription) {
        let agents = self.ready_agents();
        let settings = self.settings.clone();
        let s = cx.new(|cx| SettingsPane::new(settings, agents, window, cx));
        let sub = cx.subscribe_in(&s, window, |ws, _, ev: &SettingsEvent, window, cx| {
            let SettingsEvent::Changed(next) = ev;
            ws.apply_settings(next.clone(), window, cx)
        });
        (PaneKind::Settings(s), None, sub)
    }

    fn make_launcher(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (PaneKind, Option<String>, Subscription) {
        let cwd = self.focused_cwd(cx);
        let place = cwd.as_deref().map(|c| place_of(c).label());
        let mut choices = vec![
            Choice {
                pick: Pick::Terminal,
                label: "Terminal".into(),
                detail: "Your shell".into(),
                glyph: Icon::Terminal,
                enabled: true,
                tui: None,
            },
            Choice {
                pick: Pick::Browser,
                label: "Browser".into(),
                detail: "Search or open a page".into(),
                glyph: Icon::Globe,
                enabled: true,
                tui: None,
            },
        ];
        for agent in AGENTS {
            let installed = self.installed.iter().any(|a| a.cmd == agent.cmd);
            let chat = self
                .chats
                .as_ref()
                .and_then(|env| env.registry.get(&agent_core::AgentId::new(agent.cmd)))
                .is_some_and(|e| e.is_ready());
            choices.push(Choice {
                pick: if chat {
                    Pick::Chat(agent)
                } else {
                    Pick::Tui(agent)
                },
                label: agent.name.into(),
                detail: match (chat, installed) {
                    (true, _) => "Chat · or in a terminal",
                    (false, true) => "In a terminal",
                    (false, false) => "Not installed",
                }
                .into(),
                glyph: Icon::agent(agent.cmd),
                enabled: chat || installed,
                tui: (chat && installed).then_some(agent),
            });
        }
        let l = cx.new(|cx| LauncherPane::new(choices, cwd, place, cx));
        let sub = cx.subscribe_in(
            &l,
            window,
            move |ws, l, ev: &LauncherEvent, window, cx| match ev {
                LauncherEvent::Pick(pick) => {
                    let cwd = l.read(cx).cwd.clone();
                    ws.turn_into(id, *pick, cwd, window, cx);
                }
                LauncherEvent::Cancel => ws.close_pane(id, window, cx),
            },
        );
        (PaneKind::Launcher(l), None, sub)
    }

    /// Turn the launcher `id` into what was picked, in the same place.
    fn turn_into(
        &mut self,
        id: PaneId,
        pick: Pick,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (what, agent) = match pick {
            Pick::Terminal => (Open::Terminal { cmd: None, cwd }, None),
            Pick::Browser => (
                Open::Browser {
                    url: "about:blank".into(),
                },
                None,
            ),
            Pick::Chat(a) => (
                Open::Chat {
                    agent: Some(a.cmd.into()),
                    cwd,
                },
                None,
            ),
            Pick::Tui(a) => (
                Open::Terminal {
                    cmd: Some(a.cmd.into()),
                    cwd,
                },
                Some(a),
            ),
        };
        let Some(ix) = self.tab_of(id) else {
            return;
        };
        let new = match self.make_pane(what, agent, window, cx) {
            Ok(new) => new,
            Err(e) => {
                tracing::warn!("new tab: {e}");
                return;
            }
        };
        let tab = &mut self.tabs[ix];
        tab.root.replace(id, new);
        if tab.focused == id {
            tab.focused = new;
        }
        self.discard(id, cx);
        self.focus_pane(new, window, cx);
        if matches!(pick, Pick::Browser) {
            self.focus_address(window, cx);
        }
    }

    /// ⌘,: the one Settings tab, opened or brought forward.
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.panes.iter().find_map(|(id, p)| match p.kind {
            PaneKind::Settings(_) => Some(*id),
            _ => None,
        });
        match existing {
            Some(id) => self.focus_pane(id, window, cx),
            None => {
                if let Err(e) = self.new_tab(Open::Settings, None, window, cx) {
                    tracing::warn!("settings: {e}");
                }
            }
        }
    }

    /// Show or hide the inspector. Asking for it in a window too narrow for
    /// three columns trades the sidebar for it rather than doing nothing.
    fn toggle_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // In unzoomed units, so the breakpoints hold at any zoom.
        let width = f32::from(window.viewport_size().width) / self.settings.zoom;
        let fits = |sidebar: bool| {
            width - if sidebar { theme::SIDEBAR_PX } else { 0. } - theme::INSPECTOR_PX
                >= MIN_CONTENT_PX
        };
        if self.inspector.open && fits(self.sidebar) {
            self.inspector.open = false;
        } else {
            self.inspector.open = true;
            if !fits(self.sidebar) && fits(false) {
                self.sidebar = false;
            }
        }
        self.refresh_inspector(cx);
        cx.notify();
    }

    /// Title and URL when the focused pane is a browser.
    pub fn focused_browser(&self, cx: &App) -> Option<(SharedString, SharedString)> {
        let id = self.focused()?;
        let PaneKind::Browser(b) = &self.panes.get(&id)?.kind else {
            return None;
        };
        Some((self.pane_title(id, cx), b.read(cx).url.clone()))
    }

    fn status(&self, id: PaneId, cx: &App) -> Option<Status> {
        let pane = self.panes.get(&id)?;
        if let PaneKind::Chat(c) = &pane.kind {
            return Some(match c.read(cx).status {
                ChatStatus::Working => Status::Working,
                ChatStatus::Idle => Status::Idle,
                ChatStatus::Waiting | ChatStatus::Failed => Status::Attention,
            });
        }
        pane.agent?;
        let PaneKind::Terminal(t) = &pane.kind else {
            return None;
        };
        let t = t.read(cx);
        Some(if t.bell {
            Status::Attention
        } else if t.last_output.elapsed() < Duration::from_millis(1500) {
            Status::Working
        } else {
            Status::Idle
        })
    }

    fn pane_title(&self, id: PaneId, cx: &App) -> SharedString {
        match self.panes.get(&id) {
            // Claude Code and friends title the terminal with the task at
            // hand, which says more than the agent's name.
            Some(Pane {
                agent: Some(a),
                kind: PaneKind::Terminal(t),
                ..
            }) => clean_title(&t.read(cx).title)
                .filter(|t| !t.eq_ignore_ascii_case(a.name) && !t.eq_ignore_ascii_case(a.cmd))
                .unwrap_or_else(|| a.name.to_string())
                .into(),
            Some(Pane { agent: Some(a), .. }) => a.name.into(),
            Some(Pane {
                kind: PaneKind::Chat(c),
                ..
            }) => {
                let c = c.read(cx);
                if c.title.is_empty() {
                    c.agent_name()
                } else {
                    c.title.clone()
                }
            }
            Some(Pane {
                kind: PaneKind::Terminal(t),
                ..
            }) => clean_title(&t.read(cx).title)
                .or_else(|| self.places.get(&id).and_then(|p| p.front.clone()))
                .unwrap_or_else(|| short_title(&t.read(cx).title))
                .into(),
            Some(Pane {
                kind: PaneKind::Settings(_),
                ..
            }) => "Settings".into(),
            Some(Pane {
                kind: PaneKind::Launcher(_),
                ..
            }) => "New Tab".into(),
            Some(Pane {
                kind: PaneKind::Browser(b),
                ..
            }) => {
                let b = b.read(cx);
                if !b.title.is_empty() {
                    b.title.clone()
                } else if b.url.is_empty() || b.url.as_ref() == "about:blank" {
                    "New Tab".into()
                } else {
                    host(&b.url).into()
                }
            }
            None => "".into(),
        }
    }

    /// The second line of a tab: where a terminal or chat is working, the
    /// site a page is on.
    fn pane_subtitle(&self, id: PaneId, cx: &App) -> Option<SharedString> {
        match &self.panes.get(&id)?.kind {
            PaneKind::Browser(b) => {
                let b = b.read(cx);
                (!b.is_blank()).then(|| crate::history::host(&b.url).to_string().into())
            }
            _ => self.places.get(&id).map(|p| p.label().into()),
        }
    }

    fn group(&self, tab: &Tab) -> Group {
        let first = tab.leaves()[0];
        match self.panes.get(&first) {
            Some(Pane { agent: Some(_), .. })
            | Some(Pane {
                kind: PaneKind::Chat(_),
                ..
            }) => Group::Threads,
            _ => Group::Workspace,
        }
    }

    // -- control plane ------------------------------------------------------

    fn handle(
        &mut self,
        req: control::Request,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let automated =
            !req.token.is_empty() && self.automation.as_deref() == Some(req.token.as_str());
        let caller = self
            .panes
            .iter()
            .find(|(_, p)| p.token.as_deref() == Some(req.token.as_str()))
            .map(|(id, _)| *id)
            .or_else(|| self.focused().filter(|_| automated));
        let p = &req.params;
        let Some(caller) = caller else {
            // Opening a page is the one thing a caller from outside may do:
            // it is what `open -a` would let any process do anyway.
            if req.method == "browser.open" {
                let r = match str_param(p, "url") {
                    Some(url) => self
                        .new_tab(Open::Browser { url: url.into() }, None, window, cx)
                        .map(|id| json!({ "pane": id }))
                        .map_err(|e| e.to_string()),
                    None => Err("missing URL".into()),
                };
                return reply.send(r);
            }
            return reply.send(Err(
                "unknown or expired token: run deck from a terminal inside a deck window".into(),
            ));
        };
        let r = match req.method.as_str() {
            "pane.list" => Ok(self.list(caller, cx)),
            "pane.split" => self.rpc_split(p, caller, window, cx),
            "tab.new" => self.rpc_tab(p, window, cx),
            "pane.send" => self.rpc_send(p, cx),
            "pane.read" => return self.rpc_read(p, reply, cx),
            "pane.focus" => self.existing(p).map(|id| {
                self.focus_pane(id, window, cx);
                json!({ "pane": id })
            }),
            "pane.close" => self.existing(p).map(|id| {
                self.close_pane(id, window, cx);
                json!({ "closed": id })
            }),
            "browser.open" => self.rpc_open(p, caller, window, cx),
            "browser.eval" => return self.rpc_eval(p, reply, cx),
            m => Err(format!("unknown method {m:?}; run `deck help`")),
        };
        reply.send(r);
    }

    fn list(&self, caller: PaneId, cx: &App) -> Value {
        let mut panes = Vec::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            for id in tab.leaves() {
                let Some(pane) = self.panes.get(&id) else {
                    continue;
                };
                let focused = ix == self.active && tab.focused == id;
                let mut v = match &pane.kind {
                    PaneKind::Terminal(_) => json!({ "kind": "terminal" }),
                    PaneKind::Browser(b) => {
                        json!({ "kind": "browser", "url": b.read(cx).url.as_ref() })
                    }
                    PaneKind::Chat(c) => {
                        let c = c.read(cx);
                        json!({ "kind": "chat", "agent": c.agent.as_str(), "working": c.busy(), "needs_you": c.status == ChatStatus::Waiting })
                    }
                    PaneKind::Settings(_) => json!({ "kind": "settings" }),
                    PaneKind::Launcher(_) => json!({ "kind": "new-tab" }),
                };
                v["id"] = json!(id);
                v["tab"] = json!(ix + 1);
                v["title"] = json!(self.pane_title(id, cx).as_ref());
                v["focused"] = json!(focused);
                if let Some(a) = pane.agent {
                    v["agent"] = json!(a.cmd);
                }
                if tab.pinned && tab.root == Node::Leaf(id) {
                    v["control"] = json!(true);
                }
                if id == caller {
                    v["you"] = json!(true);
                }
                panes.push(v);
            }
        }
        json!({ "you": caller, "panes": panes })
    }

    fn existing(&self, p: &Value) -> Result<PaneId, String> {
        let id = p
            .get("pane")
            .and_then(Value::as_u64)
            .ok_or("missing --pane N (see `deck pane list`)")?;
        self.panes
            .contains_key(&id)
            .then_some(id)
            .ok_or_else(|| format!("no pane {id} in this window"))
    }

    fn open_from(p: &Value) -> Result<(Open, Option<&'static Agent>), String> {
        if let Some(agent) = p.get("chat") {
            return Ok((
                Open::Chat {
                    agent: agent.as_str().map(Into::into),
                    cwd: str_param(p, "cwd").map(Into::into),
                },
                None,
            ));
        }
        if let Some(name) = str_param(p, "agent") {
            let agent = AGENTS
                .iter()
                .find(|a| a.cmd == name)
                .ok_or_else(|| format!("unknown agent {name:?}"))?;
            return Ok((
                Open::Terminal {
                    cmd: Some(agent.cmd.into()),
                    cwd: str_param(p, "cwd").map(Into::into),
                },
                Some(agent),
            ));
        }
        Ok(match str_param(p, "kind").unwrap_or("terminal") {
            "terminal" => (
                Open::Terminal {
                    cmd: str_param(p, "cmd").map(Into::into),
                    cwd: str_param(p, "cwd").map(Into::into),
                },
                None,
            ),
            "browser" => (
                Open::Browser {
                    url: str_param(p, "url").unwrap_or("about:blank").into(),
                },
                None,
            ),
            "chat" => (
                Open::Chat {
                    agent: None,
                    cwd: str_param(p, "cwd").map(Into::into),
                },
                None,
            ),
            k => return Err(format!("--kind is terminal, browser or chat, got {k:?}")),
        })
    }

    fn rpc_split(
        &mut self,
        p: &Value,
        caller: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let beside = match p.get("pane") {
            Some(_) => self.existing(p)?,
            None => caller,
        };
        let axis = match str_param(p, "dir").unwrap_or("right") {
            "right" => Axis::Row,
            "down" => Axis::Column,
            d => return Err(format!("--dir is right or down, got {d:?}")),
        };
        let (what, _) = Self::open_from(p)?;
        let id = self
            .split(beside, what, axis, window, cx)
            .map_err(|e| e.to_string())?;
        Ok(json!({ "pane": id }))
    }

    fn rpc_tab(
        &mut self,
        p: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let (what, agent) = Self::open_from(p)?;
        let id = self
            .new_tab(what, agent, window, cx)
            .map_err(|e| e.to_string())?;
        Ok(json!({ "pane": id, "tab": self.active + 1 }))
    }

    fn rpc_send(&mut self, p: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let id = self.existing(p)?;
        let text = str_param(p, "text").ok_or("nothing to send")?;
        let enter = p.get("enter").and_then(Value::as_bool).unwrap_or(true);
        match &self.panes[&id].kind {
            PaneKind::Terminal(t) => t.update(cx, |t, cx| t.send(text, enter, cx)),
            // Messaging a chat is how one agent hands work to another.
            PaneKind::Chat(c) => c.update(cx, |c, cx| c.send(text.to_string(), cx)),
            PaneKind::Browser(_) => {
                return Err(format!("pane {id} is a browser; use `deck browser eval`"))
            }
            PaneKind::Settings(_) => return Err(format!("pane {id} is the Settings tab")),
            PaneKind::Launcher(_) => return Err(format!("pane {id} is an empty new tab")),
        }
        Ok(json!({ "pane": id }))
    }

    fn rpc_read(&mut self, p: &Value, reply: Reply, cx: &mut Context<Self>) {
        let id = match self.existing(p) {
            Ok(id) => id,
            Err(e) => return reply.send(Err(e)),
        };
        match &self.panes[&id].kind {
            PaneKind::Terminal(t) => {
                let text = t.update(cx, |t, _| t.read_text());
                reply.send(Ok(json!({ "pane": id, "text": text })));
            }
            PaneKind::Browser(b) => b
                .read(cx)
                .read_text(move |r| reply.send(r.map(|text| json!({ "pane": id, "text": text })))),
            PaneKind::Chat(c) => {
                let text = c.read(cx).read_text();
                reply.send(Ok(json!({ "pane": id, "text": text })));
            }
            PaneKind::Settings(_) => reply.send(Err(format!("pane {id} is the Settings tab"))),
            PaneKind::Launcher(_) => reply.send(Err(format!("pane {id} is an empty new tab"))),
        }
    }

    fn rpc_open(
        &mut self,
        p: &Value,
        caller: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let url = str_param(p, "url").ok_or("missing URL")?;
        if p.get("pane").is_some() {
            let id = self.existing(p)?;
            let PaneKind::Browser(b) = &self.panes[&id].kind else {
                return Err(format!("pane {id} is a terminal"));
            };
            b.update(cx, |b, cx| b.navigate(url, cx))
                .map_err(|e| e.to_string())?;
            return Ok(json!({ "pane": id }));
        }
        let id = self
            .open_url_near(caller, url, window, cx)
            .ok_or("could not open a browser pane")?;
        Ok(json!({ "pane": id }))
    }

    fn rpc_eval(&mut self, p: &Value, reply: Reply, cx: &mut Context<Self>) {
        let res = self.existing(p).and_then(|id| {
            let js = str_param(p, "js").ok_or("missing JS")?;
            match &self.panes[&id].kind {
                PaneKind::Browser(b) => Ok((b.clone(), js.to_string())),
                _ => Err(format!("pane {id} is not a browser")),
            }
        });
        match res {
            Ok((b, js)) => b
                .read(cx)
                .eval(&js, move |r| reply.send(r.map(|v| json!({ "value": v })))),
            Err(e) => reply.send(Err(e)),
        }
    }

    // -- rendering ----------------------------------------------------------

    fn render_node(&self, node: &Node, multi: bool, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(id) => self.render_leaf(*id, multi, cx),
            Node::Split(axis, children) => {
                // Each pane is its own card, a gutter apart, so the split
                // reads as separate surfaces rather than ruled cells.
                let mut d = div().flex().size_full().gap(u(theme::GUTTER));
                d = match axis {
                    Axis::Row => d.flex_row(),
                    Axis::Column => d.flex_col(),
                };
                for c in children {
                    d = d.child(
                        div()
                            .flex()
                            .flex_1()
                            .flex_basis(u(0.))
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(self.render_node(c, multi, cx)),
                    );
                }
                d.into_any_element()
            }
        }
    }

    fn render_leaf(&self, id: PaneId, multi: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(pane) = self.panes.get(&id) else {
            return div().into_any_element();
        };
        let focused = self.focused() == Some(id);
        // Cached: a pane redraws when it changes, not whenever anything in
        // the window does. Terminal output then costs that terminal's frame
        // alone, and a sidebar tick leaves every pane's layout untouched.
        let view: AnyView = match &pane.kind {
            PaneKind::Terminal(t) => t.clone().into(),
            PaneKind::Browser(b) => b.clone().into(),
            PaneKind::Chat(c) => c.clone().into(),
            PaneKind::Settings(s) => s.clone().into(),
            PaneKind::Launcher(l) => l.clone().into(),
        };
        let body = view
            .cached(StyleRefinement::default().size_full())
            .into_any_element();
        let dim = multi && !focused;
        if let PaneKind::Browser(b) = &pane.kind {
            // A page is a native view gpui cannot paint over, so it dims itself.
            b.update(cx, |b, _| b.set_dimmed(dim));
        }
        div()
            .relative()
            .size_full()
            .bg(rgb(theme::PANE))
            // In a split, the focused card has the brighter edge and the
            // others step back a little: where keys go, without a stripe.
            .when(multi, |d| {
                d.rounded(u(theme::CARD_RADIUS))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgba(if focused {
                        theme::FOCUS_EDGE
                    } else {
                        theme::HAIRLINE
                    }))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |ws, _, window, cx| {
                    if ws.focused() != Some(id) {
                        ws.focus_pane(id, window, cx)
                    }
                }),
            )
            // A right-click acts on the pane it lands in, so its menu does.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |ws, _, window, cx| {
                    if ws.focused() != Some(id) {
                        ws.focus_pane(id, window, cx)
                    }
                }),
            )
            .child(body)
            .when(dim && !matches!(pane.kind, PaneKind::Browser(_)), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(theme::UNFOCUSED_VEIL)),
                )
            })
            .into_any_element()
    }

    /// A page's own icon for its tab, once deck has it.
    fn tab_favicon(&self, first: PaneId, cx: &App) -> Option<Arc<gpui::Image>> {
        match &self.panes.get(&first)?.kind {
            PaneKind::Browser(b) => b.read(cx).favicon(),
            _ => None,
        }
    }

    fn tab_icon(&self, first: PaneId, cx: &App) -> (Icon, u32) {
        match self.panes.get(&first) {
            Some(Pane { agent: Some(a), .. }) => (Icon::agent(a.cmd), theme::TEXT),
            Some(Pane {
                kind: PaneKind::Chat(c),
                ..
            }) => (Icon::agent(c.read(cx).agent.as_str()), theme::TEXT),
            Some(Pane {
                kind: PaneKind::Browser(_),
                ..
            }) => (Icon::Globe, theme::TEXT),
            Some(Pane {
                kind: PaneKind::Settings(_),
                ..
            }) => (Icon::Settings, theme::TEXT),
            Some(Pane {
                kind: PaneKind::Launcher(_),
                ..
            }) => (Icon::Plus, theme::TEXT_2),
            _ => (Icon::Terminal, theme::TEXT_2),
        }
    }

    /// The sidebar is an inbox, as in T3 Code: the control chat first, then
    /// threads with what each one needs from you, then the terminals and
    /// pages they opened. Empty sections are not drawn.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut threads: Vec<(usize, Instant)> = Vec::new();
        let mut workspace: Vec<usize> = Vec::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            if tab.pinned {
                continue;
            }
            match self.group(tab) {
                Group::Threads => threads.push((ix, self.last_active(tab.leaves()[0], cx))),
                Group::Workspace => workspace.push(ix),
            }
        }
        // Most recent first: the thread that just finished is the one to read.
        threads.sort_by_key(|t| std::cmp::Reverse(t.1));

        let header = |label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(u(2.))
                .h(u(28.))
                .mt(u(14.))
                .pl(u(theme::ROW_PAD))
                .pr(u(2.))
                .text_size(u(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::DIM))
                .child(div().flex_1().child(label))
        };

        let mut list = div()
            .id("tabs")
            .flex()
            .flex_col()
            .flex_1()
            .gap(u(2.))
            .overflow_y_scroll()
            .px(u(theme::GUTTER))
            .pb(u(theme::GUTTER))
            .child(
                div()
                    .id("new-chat")
                    .flex()
                    .items_center()
                    .gap(u(8.))
                    .h(u(34.))
                    .px(u(10.))
                    .mb(u(6.))
                    .rounded(u(10.))
                    .bg(rgb(theme::RAISED))
                    .border_1()
                    .border_color(rgba(theme::HAIRLINE))
                    .text_size(u(13.))
                    .text_color(rgb(theme::TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SURFACE)))
                    .child(icon(Icon::Plus, 14., theme::TEXT))
                    .child(div().flex_1().child("New chat"))
                    .child(
                        div()
                            .text_size(u(11.))
                            .text_color(rgb(theme::DIM))
                            .child("⌘N"),
                    )
                    .on_click(cx.listener(|ws, _, window, cx| {
                        let what = Open::Chat {
                            agent: None,
                            cwd: ws.focused_cwd(cx),
                        };
                        if ws.new_tab(what, None, window, cx).is_err() {
                            ws.picker = true;
                            cx.notify();
                        }
                    })),
            )
            .child(self.render_control_row(cx));

        list = list.child(
            header("Threads").child(
                icon_button(
                    "agents-plus",
                    if self.picker {
                        Icon::ChevronDown
                    } else {
                        Icon::Plus
                    },
                )
                .size(u(24.))
                .on_click(cx.listener(|ws, _, _, cx| {
                    ws.picker = !ws.picker;
                    cx.notify();
                })),
            ),
        );
        if self.picker {
            list = list.child(self.render_picker(cx));
        }
        for (ix, _) in &threads {
            list = list.child(self.render_thread_row(*ix, cx));
        }

        if !workspace.is_empty() {
            list =
                list.child(
                    header("Workspace")
                        .child(
                            icon_button("ws-term", Icon::Terminal)
                                .size(u(24.))
                                .on_click(cx.listener(|ws, _, w, cx| {
                                    let what = ws.shell_here(cx);
                                    let _ = ws.new_tab(what, None, w, cx);
                                })),
                        )
                        .child(icon_button("ws-web", Icon::Globe).size(u(24.)).on_click(
                            cx.listener(|ws, _, w, cx| {
                                if ws
                                    .new_tab(
                                        Open::Browser {
                                            url: "about:blank".into(),
                                        },
                                        None,
                                        w,
                                        cx,
                                    )
                                    .is_ok()
                                {
                                    ws.focus_address(w, cx);
                                }
                            }),
                        )),
                );
            for ix in workspace {
                list = list.child(self.render_tab_row(ix, &self.tabs[ix], cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(u(theme::SIDEBAR_PX))
            .h_full()
            .child(
                // The traffic lights live here; the native titlebar under it
                // is the drag handle. Real pixels: the lights do not zoom.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .h(px(theme::TITLEBAR_PX))
                    .px(u(theme::GUTTER))
                    .on_mouse_down(MouseButton::Left, |ev, window, _| {
                        if ev.click_count == 2 {
                            window.titlebar_double_click();
                        }
                    })
                    .child(
                        icon_button("hide-sidebar", Icon::Sidebar).on_click(cx.listener(
                            |ws, _, _, cx| {
                                ws.sidebar = false;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(list)
            .child(
                div()
                    .flex_none()
                    .px(u(theme::GUTTER))
                    .pb(u(theme::GUTTER))
                    .child(
                        div()
                            .id("open-settings")
                            .flex()
                            .items_center()
                            .gap(u(10.))
                            .h(u(32.))
                            .px(u(theme::ROW_PAD))
                            .rounded(u(theme::ROW_RADIUS))
                            .text_size(u(13.))
                            .text_color(rgb(theme::TEXT_2))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(theme::ROW_HOVER)).text_color(rgb(theme::TEXT)))
                            .child(icon(Icon::Settings, 15., theme::TEXT_2))
                            .child(div().flex_1().child("Settings"))
                            .child(
                                div()
                                    .text_size(u(11.))
                                    .text_color(rgb(theme::DIM))
                                    .child("⌘,"),
                            )
                            .on_click(
                                cx.listener(|ws, _, window, cx| ws.open_settings(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// When the pane last did something, for ordering and "5m ago".
    fn last_active(&self, id: PaneId, cx: &App) -> Instant {
        match self.panes.get(&id).map(|p| &p.kind) {
            Some(PaneKind::Chat(c)) => c.read(cx).last_activity,
            Some(PaneKind::Terminal(t)) => t.read(cx).last_output,
            _ => Instant::now(),
        }
    }

    /// What a thread needs from you, in a few words, or how long ago it ran.
    fn thread_state(&self, id: PaneId, cx: &App) -> (SharedString, bool) {
        let unseen = self.unseen.contains(&id);
        match self.status(id, cx) {
            Some(Status::Working) => {
                let secs = match self.panes.get(&id).map(|p| &p.kind) {
                    Some(PaneKind::Chat(c)) => c.read(cx).working_for().map(|d| d.as_secs()),
                    _ => None,
                };
                (
                    secs.map_or("Working".into(), |s| {
                        format!("Working {}", short_secs(s)).into()
                    }),
                    true,
                )
            }
            Some(Status::Attention) => ("Needs you".into(), true),
            _ if unseen => ("Done".into(), true),
            _ => (ago(self.last_active(id, cx)).into(), false),
        }
    }

    /// The control plane's entry: always first, always there.
    fn render_control_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let ix = self.control_tab();
        let active = ix == Some(self.active);
        let pane = ix.and_then(|i| self.tabs.get(i)).map(|t| t.leaves()[0]);
        let agent = pane.and_then(|id| match &self.panes.get(&id)?.kind {
            PaneKind::Chat(c) => {
                let c = c.read(cx);
                Some((Icon::agent(c.agent.as_str()), c.agent_name()))
            }
            _ => None,
        });
        let (glyph, subtitle): (Icon, SharedString) = match agent {
            Some((g, name)) => (g, name),
            None if !self.probed => (Icon::GenericAgent, "Starting agents…".into()),
            None => (Icon::GenericAgent, "No agent CLI found".into()),
        };
        let state = pane.map(|id| self.thread_state(id, cx));
        let working = pane.and_then(|id| self.status(id, cx)) == Some(Status::Working);
        div()
            .id("control")
            .flex()
            .items_center()
            .gap(u(10.))
            .h(u(44.))
            .pl(u(6.))
            .pr(u(10.))
            .rounded(u(theme::ROW_RADIUS))
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(theme::SURFACE)))
            .when(!active, |d| d.hover(|s| s.bg(rgb(theme::ROW_HOVER))))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(u(28.))
                    .rounded(u(7.))
                    .bg(rgb(if active {
                        theme::SURFACE_HOVER
                    } else {
                        theme::SURFACE
                    }))
                    .child(if working {
                        thinking("control-think", glyph, 15., theme::TEXT)
                    } else {
                        icon(glyph, 15., theme::TEXT)
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(u(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::TEXT))
                            .child("Control"),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(u(11.))
                            .text_color(rgb(theme::DIM))
                            .child(subtitle),
                    ),
            )
            .when_some(state.filter(|(_, loud)| *loud), |d, (label, _)| {
                d.child(if working {
                    div()
                        .text_size(u(11.))
                        .child(shimmer("control-shimmer", label))
                        .into_any_element()
                } else {
                    pill(label).into_any_element()
                })
            })
            .on_click(
                cx.listener(move |ws, _, window, cx| match ws.control_tab() {
                    Some(i) => ws.activate(i, window, cx),
                    None => ws.open_control(Vec::new(), window, cx),
                }),
            )
            .into_any_element()
    }

    /// A thread: its title, then who is doing it and what it needs from you.
    fn render_thread_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.tabs.get(ix) else {
            return div().into_any_element();
        };
        let active = ix == self.active;
        let first = tab.leaves()[0];
        let (glyph, _) = self.tab_icon(first, cx);
        let project = self.places.get(&first).map(|p| p.project.clone());
        let who: SharedString = match self.panes.get(&first) {
            Some(Pane {
                kind: PaneKind::Chat(c),
                ..
            }) => match project {
                Some(p) => format!("{} · {p}", c.read(cx).agent_name()).into(),
                None => c.read(cx).agent_name(),
            },
            Some(Pane { agent: Some(a), .. }) => match project {
                Some(p) => format!("{} · {p}", a.name).into(),
                None => format!("{} · terminal", a.name).into(),
            },
            _ => "".into(),
        };
        let working = self.status(first, cx) == Some(Status::Working);
        let (state, loud) = self.thread_state(first, cx);
        let group = SharedString::from(format!("tab-{ix}"));
        div()
            .id(ElementId::NamedInteger("tab".into(), ix as u64))
            .group(group.clone())
            .flex()
            .items_center()
            .gap(u(10.))
            .h(u(44.))
            .pl(u(theme::ROW_PAD))
            .pr(u(6.))
            .rounded(u(theme::ROW_RADIUS))
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(theme::SURFACE)))
            .when(!active, |d| d.hover(|s| s.bg(rgb(theme::ROW_HOVER))))
            .child(if working {
                thinking(
                    ElementId::NamedInteger("thread-think".into(), ix as u64),
                    glyph,
                    15.,
                    theme::TEXT,
                )
            } else {
                icon(
                    glyph,
                    15.,
                    if active || loud {
                        theme::TEXT
                    } else {
                        theme::TEXT_2
                    },
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(u(13.))
                            .text_color(rgb(if active || loud {
                                theme::TEXT
                            } else {
                                theme::TEXT_2
                            }))
                            .child(self.pane_title(tab.focused, cx)),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(u(11.))
                            .text_color(rgb(theme::DIM))
                            .child(who),
                    ),
            )
            .child(
                // The state, swapped for the close button while hovered.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .relative()
                    .child(
                        div()
                            .group_hover(group.clone(), |s| s.opacity(0.))
                            .text_size(u(11.))
                            .child(if working {
                                shimmer(
                                    ElementId::NamedInteger("thread-shimmer".into(), ix as u64),
                                    state,
                                )
                            } else if loud {
                                pill(state).into_any_element()
                            } else {
                                div()
                                    .text_size(u(11.))
                                    .text_color(rgb(theme::DIM))
                                    .child(state)
                                    .into_any_element()
                            }),
                    )
                    .child(
                        icon_button(
                            ElementId::NamedInteger("close".into(), ix as u64),
                            Icon::Close,
                        )
                        .absolute()
                        .right_0()
                        .size(u(22.))
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            cx.stop_propagation();
                            ws.close_tab(ix, window, cx);
                        })),
                    ),
            )
            .on_click(cx.listener(move |ws, _, window, cx| ws.activate(ix, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |ws, ev: &gpui::MouseDownEvent, window, cx| {
                    ws.open_tab_menu(ix, ev.position, window, cx)
                }),
            )
            .into_any_element()
    }

    /// The agent launcher. A row starts a structured chat (the control
    /// plane) when deck can drive that agent; the terminal button beside it
    /// runs the agent's own TUI in a tab instead.
    fn render_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        // Concentric: rows are radius 6 inside 4px of padding, so the card is 10.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(u(2.))
            .p(u(4.))
            .rounded(u(10.))
            .bg(rgb(theme::PANE))
            .border_1()
            .border_color(rgb(theme::BORDER));
        for agent in AGENTS {
            let installed = self.installed.iter().any(|a| a.cmd == agent.cmd);
            let chat = self
                .chats
                .as_ref()
                .and_then(|env| env.registry.get(&agent_core::AgentId::new(agent.cmd)))
                .is_some_and(|e| e.is_ready());
            let note = match (chat, installed, self.chats.is_some()) {
                (true, _, _) => "chat",
                (false, true, false) => "…",
                (false, true, true) => "terminal",
                (false, false, _) => "not installed",
            };
            col = col.child(
                div()
                    .id(ElementId::Name(agent.cmd.into()))
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .h(u(32.))
                    .pl(u(8.))
                    .pr(u(3.))
                    .rounded(u(6.))
                    .text_size(u(13.))
                    .when(installed, |d| {
                        d.cursor_pointer()
                            .text_color(rgb(theme::TEXT))
                            .hover(|s| s.bg(rgb(theme::SURFACE)))
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                let what = if chat {
                                    Open::Chat {
                                        agent: Some(agent.cmd.into()),
                                        cwd: ws.focused_cwd(cx),
                                    }
                                } else {
                                    Open::Terminal {
                                        cmd: Some(agent.cmd.into()),
                                        cwd: ws.focused_cwd(cx),
                                    }
                                };
                                let tui = (!chat).then_some(agent);
                                let _ = ws.new_tab(what, tui, window, cx);
                            }))
                    })
                    .when(!installed, |d| d.text_color(rgb(theme::DIM)))
                    .child(icon(
                        Icon::agent(agent.cmd),
                        15.,
                        if installed { theme::TEXT } else { theme::DIM },
                    ))
                    .child(div().flex_1().child(agent.name))
                    .child(
                        div()
                            .text_size(u(11.))
                            .text_color(rgb(theme::DIM))
                            .child(note),
                    )
                    .when(installed && chat, |d| {
                        d.child(
                            icon_button(
                                ElementId::Name(format!("tui-{}", agent.cmd).into()),
                                Icon::Terminal,
                            )
                            .size(u(24.))
                            .on_click(cx.listener(
                                move |ws, _, window, cx| {
                                    cx.stop_propagation();
                                    let what = Open::Terminal {
                                        cmd: Some(agent.cmd.into()),
                                        cwd: ws.focused_cwd(cx),
                                    };
                                    let _ = ws.new_tab(what, Some(agent), window, cx);
                                },
                            )),
                        )
                    }),
            );
        }
        col.into_any_element()
    }

    fn render_tab_row(&self, ix: usize, tab: &Tab, cx: &mut Context<Self>) -> AnyElement {
        let active = ix == self.active;
        let first = tab.leaves()[0];
        let (glyph, tint) = self.tab_icon(first, cx);
        let status = self.status(first, cx);
        let count = tab.leaves().len();
        let group = SharedString::from(format!("tab-{ix}"));
        let subtitle = self.pane_subtitle(tab.focused, cx);
        let is_page = matches!(
            self.panes.get(&first).map(|p| &p.kind),
            Some(PaneKind::Browser(_))
        );
        let tone = if active { tint } else { theme::TEXT_2 };
        // A loading page spins where its icon goes, as tab strips do; a busy
        // terminal spins at the end of its row.
        let loading = match self.panes.get(&first).map(|p| &p.kind) {
            Some(PaneKind::Browser(b)) => {
                let b = b.read(cx);
                b.loading && !b.is_blank()
            }
            _ => false,
        };
        let busy = status == Some(Status::Working) || self.busy.contains(&first);
        let lead = match (loading, self.tab_favicon(first, cx)) {
            (true, _) => spinner(
                ElementId::NamedInteger("tab-load".into(), ix as u64),
                14.,
                theme::TEXT_2,
            ),
            (_, Some(image)) => crate::browser::site_image(image, 15.),
            _ => icon(glyph, 15., tone),
        };

        div()
            .id(ElementId::NamedInteger("tab".into(), ix as u64))
            .group(group.clone())
            .flex()
            .items_center()
            .gap(u(10.))
            .h(u(if subtitle.is_some() { 42. } else { 32. }))
            .pl(u(theme::ROW_PAD))
            .pr(u(4.))
            .rounded(u(theme::ROW_RADIUS))
            .text_size(u(13.))
            .cursor_pointer()
            .when(active, |d| {
                d.bg(rgb(theme::SURFACE)).text_color(rgb(theme::TEXT))
            })
            .when(!active, |d| {
                d.text_color(rgb(theme::TEXT_2))
                    .hover(|s| s.bg(rgb(theme::ROW_HOVER)))
            })
            .child(lead)
            // Definite-width boxes, so gpui knows where to put the ellipsis.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .child(self.pane_title(tab.focused, cx)),
                    )
                    .children(subtitle.map(|sub| {
                        div()
                            .w_full()
                            .truncate()
                            .text_size(u(11.))
                            .text_color(rgb(theme::DIM))
                            .child(sub)
                    })),
            )
            .when(count > 1, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(u(3.))
                        .text_size(u(11.))
                        .text_color(rgb(theme::DIM))
                        .child(icon(Icon::Split, 11., theme::DIM))
                        .child(format!("{count}")),
                )
            })
            .when(busy && !is_page, |d| {
                d.child(spinner(
                    ElementId::NamedInteger("tab-busy".into(), ix as u64),
                    12.,
                    theme::TEXT_2,
                ))
            })
            // A white ring: something needs you. Idle needs no mark.
            .when(status == Some(Status::Attention), |d| {
                d.child(
                    div()
                        .flex_none()
                        .size(u(7.))
                        .rounded(u(4.))
                        .border_1()
                        .border_color(rgb(theme::TEXT)),
                )
            })
            .child(
                icon_button(
                    ElementId::NamedInteger("close".into(), ix as u64),
                    Icon::Close,
                )
                .size(u(22.))
                .opacity(0.)
                .group_hover(group, |s| s.opacity(1.))
                .on_click(cx.listener(move |ws, _, window, cx| {
                    cx.stop_propagation();
                    ws.close_tab(ix, window, cx);
                })),
            )
            .on_click(cx.listener(move |ws, _, window, cx| ws.activate(ix, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |ws, ev: &gpui::MouseDownEvent, window, cx| {
                    ws.open_tab_menu(ix, ev.position, window, cx)
                }),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.probed {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(u(13.))
                .text_color(rgb(theme::DIM))
                .child("Starting agents…")
                .into_any_element();
        }
        let mut col = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(u(8.))
            .size_full()
            .text_color(rgb(theme::TEXT_2))
            .child(
                div()
                    .pb(u(8.))
                    .text_size(u(15.))
                    .text_color(rgb(theme::TEXT))
                    .child("Nothing open"),
            );
        let actions: [(&'static str, &'static str, Icon, u8); 2] = [
            ("New Terminal", "empty-term", Icon::Terminal, 0),
            ("New Browser", "empty-web", Icon::Globe, 1),
        ];
        for (label, id, glyph, kind) in actions {
            col = col.child(
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(u(8.))
                    .w(u(220.))
                    .h(u(34.))
                    .px(u(12.))
                    .rounded(u(8.))
                    .bg(rgb(theme::SURFACE))
                    .text_size(u(13.))
                    .text_color(rgb(theme::TEXT))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SURFACE_HOVER)))
                    .child(icon(glyph, 15., theme::TEXT_2))
                    .child(label)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(u(12.))
                            .text_color(rgb(theme::DIM))
                            .child(if kind == 0 { "⌘T" } else { "⌘⇧T" }),
                    )
                    .on_click(cx.listener(move |ws, _, window, cx| {
                        let what = if kind == 0 {
                            ws.shell_here(cx)
                        } else {
                            Open::Browser {
                                url: "about:blank".into(),
                            }
                        };
                        if ws.new_tab(what, None, window, cx).is_ok() && kind == 1 {
                            ws.focus_address(window, cx);
                        }
                    })),
            );
        }
        col.into_any_element()
    }
}

pub fn icon_button(id: impl Into<ElementId>, glyph: Icon) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(u(26.))
        .rounded(u(6.))
        .text_color(rgb(theme::TEXT_2))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::SURFACE)).text_color(rgb(theme::TEXT)))
        .child(icon(glyph, 15., theme::TEXT_2))
}

/// The content area never gets narrower than this to make room for chrome:
/// past it the inspector folds away (and comes back when there is room).
const MIN_CONTENT_PX: f32 = 520.;

/// A part of the window chrome drawn as its own cached view.
///
/// gpui redraws every ancestor of a view that changed, and the workspace is
/// the root, so each frame of a streaming agent in a terminal used to rebuild
/// the whole sidebar and inspector too. As islands they draw again only when
/// the workspace itself notifies, not whenever a pane inside it repaints.
pub struct Island {
    ws: WeakEntity<Workspace>,
    draw: fn(&mut Workspace, &mut Window, &mut Context<Workspace>) -> AnyElement,
    _observe: Subscription,
}

impl Island {
    fn new(
        ws: &Entity<Workspace>,
        draw: fn(&mut Workspace, &mut Window, &mut Context<Workspace>) -> AnyElement,
        cx: &mut Context<Workspace>,
    ) -> Entity<Island> {
        let weak = ws.downgrade();
        let ws = ws.clone();
        cx.new(|cx| Island {
            ws: weak,
            draw,
            _observe: cx.observe(&ws, |_, _, cx| cx.notify()),
        })
    }
}

impl Render for Island {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draw = self.draw;
        self.ws
            .update(cx, |ws, cx| draw(ws, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// Where a pane is working: its project (the git root's name, else the
/// folder's) and the folder below that root.
#[derive(Debug, Clone, Default, PartialEq)]
struct Place {
    project: String,
    /// The path inside the project, `""` at its root.
    sub: String,
    /// What a terminal is running in front ("zsh", "nvim", "cargo").
    front: Option<String>,
}

impl Place {
    fn label(&self) -> String {
        if self.sub.is_empty() {
            self.project.clone()
        } else {
            format!("{} / {}", self.project, self.sub)
        }
    }
}

fn place_of(cwd: &std::path::Path) -> Place {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.as_deref() == Some(cwd) {
        return Place {
            project: "~".into(),
            ..Default::default()
        };
    }
    let name = |p: &std::path::Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string())
    };
    // Stop at home: a dotfiles repo there would swallow every project.
    let root = cwd
        .ancestors()
        .take_while(|d| Some(*d) != home.as_deref())
        .find(|d| d.join(".git").exists());
    match root {
        Some(root) => Place {
            project: name(root),
            sub: cwd
                .strip_prefix(root)
                .map(|r| r.display().to_string())
                .unwrap_or_default(),
            front: None,
        },
        None => Place {
            project: name(cwd),
            ..Default::default()
        },
    }
}

/// A terminal title worth showing: a program's own ("✳ Fix tab titles" from
/// Claude Code, "vim main.rs"), without the spinner glyph agents put in
/// front. The shell's default `user@host:dir` says less than the tab already
/// does, so it is dropped.
fn clean_title(t: &str) -> Option<String> {
    let t = t.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '~' && c != '/');
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    if let Some((user, rest)) = t.split_once('@') {
        if !user.contains(' ') && rest.contains(':') && !rest.contains(' ') {
            return None;
        }
    }
    Some(t.to_string())
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (sidebar, inspector) = self.islands(cx);
        window.set_rem_size(px(theme::REM_PX * self.settings.zoom));
        let width = f32::from(window.viewport_size().width) / self.settings.zoom;
        let sidebar_w = if self.sidebar { theme::SIDEBAR_PX } else { 0. };
        let inspector_shown =
            self.inspector.open && width - sidebar_w - theme::INSPECTOR_PX >= MIN_CONTENT_PX;
        // Native webviews in hidden tabs must be hidden explicitly.
        for (ix, tab) in self.tabs.iter().enumerate() {
            for id in tab.leaves() {
                if let Some(Pane {
                    kind: PaneKind::Browser(b),
                    ..
                }) = self.panes.get(&id)
                {
                    b.update(cx, |b, _| b.set_visible(ix == self.active));
                }
            }
        }

        let title = self
            .focused()
            .map(|id| self.pane_title(id, cx))
            .unwrap_or_default();
        let multi = self
            .tabs
            .get(self.active)
            .is_some_and(|t| matches!(t.root, Node::Split(..)));
        let content = match self.tabs.get(self.active) {
            Some(tab) => {
                let root = tab.root.clone();
                self.render_node(&root, multi, cx)
            }
            None => self.render_empty(cx),
        };

        let left = if self.sidebar { 0. } else { theme::GUTTER };
        let topbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(2.))
            .h(px(theme::TITLEBAR_PX))
            .pr(u(theme::GUTTER))
            // The title sits on the same edge as the text in the card below.
            .when(self.sidebar, |d| d.pl(u(theme::PANE_PAD_X + 1.)))
            // Clear of the traffic lights, which do not zoom.
            .when(!self.sidebar, |d| d.pl(px(80.)))
            .on_mouse_down(MouseButton::Left, |ev, window, _| {
                if ev.click_count == 2 {
                    window.titlebar_double_click();
                }
            })
            .when(!self.sidebar, |d| {
                d.child(
                    icon_button("show-sidebar", Icon::Sidebar)
                        .mr(u(6.))
                        .on_click(cx.listener(|ws, _, _, cx| {
                            ws.sidebar = true;
                            cx.notify();
                        })),
                )
            })
            .child(
                div().flex_1().min_w_0().child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(u(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::TEXT))
                        .child(title.clone()),
                ),
            )
            .child(
                icon_button("split-right", Icon::Split).on_click(cx.listener(|ws, _, w, cx| {
                    if let Some(p) = ws.focused() {
                        let what = ws.shell_here(cx);
                        let _ = ws.split(p, what, Axis::Row, w, cx);
                    }
                })),
            )
            .child(
                icon_button("open-web", Icon::Globe)
                    .on_click(cx.listener(|ws, _, w, cx| ws.focus_address(w, cx))),
            )
            .child(
                icon_button("inspector", Icon::PanelRight)
                    .when(inspector_shown, |d| {
                        d.bg(rgb(theme::SURFACE)).text_color(rgb(theme::TEXT))
                    })
                    .on_click(cx.listener(|ws, _, w, cx| ws.toggle_inspector(w, cx))),
            );

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(topbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .ml(u(left))
                    .mr(u(if inspector_shown { 0. } else { theme::GUTTER }))
                    .mb(u(theme::GUTTER))
                    // A split draws a card per pane instead.
                    .when(!multi, |d| {
                        d.rounded(u(theme::CARD_RADIUS))
                            .overflow_hidden()
                            .border_1()
                            .border_color(rgba(theme::HAIRLINE))
                            .bg(rgb(theme::PANE))
                    })
                    .child(content),
            );

        div()
            .size_full()
            .flex()
            .bg(rgb(theme::SIDEBAR))
            .font_family(theme::UI)
            .text_color(rgb(theme::TEXT))
            .on_action(cx.listener(|ws, _: &NewTab, w, cx| {
                let _ = ws.new_tab(Open::Launcher, None, w, cx);
            }))
            .on_action(cx.listener(|ws, _: &NewTerminal, w, cx| {
                let what = ws.shell_here(cx);
                let _ = ws.new_tab(what, None, w, cx);
            }))
            .on_action(cx.listener(|ws, _: &NewChat, w, cx| {
                let what = Open::Chat {
                    agent: None,
                    cwd: ws.focused_cwd(cx),
                };
                if let Err(e) = ws.new_tab(what, None, w, cx) {
                    tracing::warn!("new chat: {e}");
                    ws.picker = true;
                    ws.sidebar = true;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|ws, _: &NewBrowser, w, cx| {
                if ws
                    .new_tab(
                        Open::Browser {
                            url: "about:blank".into(),
                        },
                        None,
                        w,
                        cx,
                    )
                    .is_ok()
                {
                    ws.focus_address(w, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &OpenSettings, w, cx| ws.open_settings(w, cx)))
            .on_action(cx.listener(|ws, _: &ToggleAgents, _, cx| {
                ws.picker = !ws.picker;
                ws.sidebar = true;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &SplitRight, w, cx| {
                if let Some(p) = ws.focused() {
                    let what = ws.shell_here(cx);
                    let _ = ws.split(p, what, Axis::Row, w, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &SplitDown, w, cx| {
                if let Some(p) = ws.focused() {
                    let what = ws.shell_here(cx);
                    let _ = ws.split(p, what, Axis::Column, w, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &ClosePane, w, cx| {
                if let Some(p) = ws.focused() {
                    ws.close_pane(p, w, cx);
                }
            }))
            .on_action(cx.listener(|ws, _: &NextTab, w, cx| ws.cycle_tab(1, w, cx)))
            .on_action(cx.listener(|ws, _: &PrevTab, w, cx| ws.cycle_tab(-1, w, cx)))
            .on_action(cx.listener(|ws, _: &NextPane, w, cx| ws.cycle_pane(1, w, cx)))
            .on_action(cx.listener(|ws, _: &PrevPane, w, cx| ws.cycle_pane(-1, w, cx)))
            .on_action(cx.listener(|ws, _: &FocusAddress, w, cx| ws.focus_address(w, cx)))
            .on_action(cx.listener(|ws, _: &Reload, _, cx| {
                if let Some((_, b)) = ws.browser_in_tab(ws.active) {
                    b.update(cx, |b, cx| {
                        b.reload();
                        cx.notify();
                    });
                }
            }))
            .on_action(cx.listener(|ws, _: &Find, w, cx| {
                if let Some(b) = ws.focused_browser_pane() {
                    b.update(cx, |b, cx| b.open_find(w, cx));
                }
            }))
            .on_action(cx.listener(|ws, _: &FindNext, _, cx| {
                if let Some(b) = ws.focused_browser_pane() {
                    b.update(cx, |b, cx| b.find_step(false, cx));
                }
            }))
            .on_action(cx.listener(|ws, _: &FindPrevious, _, cx| {
                if let Some(b) = ws.focused_browser_pane() {
                    b.update(cx, |b, cx| b.find_step(true, cx));
                }
            }))
            .on_action(cx.listener(|ws, _: &ToggleDevTools, _, cx| {
                if let Some((_, b)) = ws.browser_in_tab(ws.active) {
                    b.read(cx).toggle_devtools();
                }
            }))
            .on_action(cx.listener(|ws, _: &ToggleSidebar, _, cx| {
                ws.sidebar = !ws.sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|ws, _: &ToggleInspector, w, cx| ws.toggle_inspector(w, cx)))
            .on_action(cx.listener(|ws, _: &ZoomIn, w, cx| ws.zoom(1, w, cx)))
            .on_action(cx.listener(|ws, _: &ZoomOut, w, cx| ws.zoom(-1, w, cx)))
            .on_action(cx.listener(|ws, _: &ZoomReset, w, cx| ws.zoom(0, w, cx)))
            .on_action(cx.listener(|ws, _: &ClearTerminal, _, cx| {
                if let Some(PaneKind::Terminal(t)) = ws
                    .focused()
                    .and_then(|id| ws.panes.get(&id))
                    .map(|p| &p.kind)
                {
                    t.update(cx, |t, cx| t.clear(cx));
                }
            }))
            .on_action(cx.listener(|ws, _: &Tab1, w, cx| ws.activate(0, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab2, w, cx| ws.activate(1, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab3, w, cx| ws.activate(2, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab4, w, cx| ws.activate(3, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab5, w, cx| ws.activate(4, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab6, w, cx| ws.activate(5, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab7, w, cx| ws.activate(6, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab8, w, cx| ws.activate(7, w, cx)))
            .on_action(cx.listener(|ws, _: &Tab9, w, cx| {
                // ⌘9 is "last tab" everywhere else, so here too.
                let last = ws.tabs.len().saturating_sub(1);
                ws.activate(last, w, cx)
            }))
            .when(self.sidebar, |d| {
                d.child(
                    AnyView::from(sidebar).cached(
                        StyleRefinement::default()
                            .flex_none()
                            .w(u(theme::SIDEBAR_PX))
                            .h_full(),
                    ),
                )
            })
            .child(main)
            .when(inspector_shown, |d| {
                d.child(
                    AnyView::from(inspector).cached(
                        StyleRefinement::default()
                            .flex_none()
                            .w(u(theme::INSPECTOR_PX))
                            .h_full(),
                    ),
                )
            })
            .children(self.menu.as_ref().map(crate::menu::Open::render))
    }
}

/// A status in a quiet pill: what a thread needs, without colour.
fn pill(label: SharedString) -> gpui::Div {
    div()
        .flex_none()
        .px(u(7.))
        .h(u(20.))
        .flex()
        .items_center()
        .rounded(u(10.))
        .bg(rgb(theme::SURFACE_HOVER))
        .text_size(u(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::TEXT))
        .child(label)
}

fn short_secs(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m", s / 60)
    }
}

/// "now", "5m", "2h", "3d": how long since something happened.
fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

/// zsh's default title is `user@host:~/dir`; the directory is the useful part.
fn short_title(t: &str) -> String {
    match t.split_once('@').and_then(|(_, rest)| rest.split_once(':')) {
        Some((_, dir)) if !dir.is_empty() => dir.to_string(),
        _ => t.to_string(),
    }
}

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .to_string()
}

fn new_token() -> String {
    let mut bytes = [0u8; 16];
    // /dev/urandom cannot run dry; failing to open it means a broken system,
    // and an all-zero token would be guessable, so that is fatal.
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("read /dev/urandom");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: &Node) -> Vec<PaneId> {
        let mut v = Vec::new();
        n.leaves(&mut v);
        v
    }

    #[test]
    fn splits_join_same_axis_runs_and_nest_across_axes() {
        let mut t = Node::Leaf(1);
        assert!(t.split(1, 2, Axis::Row));
        assert!(t.split(2, 3, Axis::Row));
        assert_eq!(
            t,
            Node::Split(Axis::Row, vec![Node::Leaf(1), Node::Leaf(2), Node::Leaf(3)])
        );
        assert!(t.split(2, 4, Axis::Column));
        assert_eq!(leaves(&t), vec![1, 2, 4, 3]);
        assert!(!t.split(99, 5, Axis::Row));
        assert!(t.contains(4) && !t.contains(5));
    }

    #[test]
    fn removing_collapses_single_child_splits() {
        let mut t = Node::Leaf(1);
        t.split(1, 2, Axis::Row);
        t.split(2, 3, Axis::Column);
        assert!(t.remove(3));
        assert_eq!(
            t,
            Node::Split(Axis::Row, vec![Node::Leaf(1), Node::Leaf(2)])
        );
        assert!(t.remove(1));
        assert_eq!(t, Node::Leaf(2));
        assert!(!t.remove(7));
    }

    #[test]
    fn titles_and_hosts_shorten() {
        assert_eq!(short_title("nees@MacBook-Pro:~/deck"), "~/deck");
        assert_eq!(short_title("vim"), "vim");
        assert_eq!(host("https://example.com/a?b"), "example.com");
        assert_eq!(host("http://localhost:5173/"), "localhost:5173");
    }
}
