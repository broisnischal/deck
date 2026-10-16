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
use crate::palette::{Item, Kind, Palette, PaletteEvent};
use crate::review::{ReviewEvent, ReviewPane};
use crate::settings_pane::{SettingsEvent, SettingsPane};
use crate::terminal::{TerminalEvent, TerminalPane};
use crate::theme::{self, u, OneLine};
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    actions, canvas, div, px, rgb, rgba, AnyElement, AnyView, App, Context, ElementId, Entity,
    FontWeight, KeyBinding, Menu, MenuItem, MouseButton, OsAction, SharedString, StyleRefinement,
    Subscription, Task, WeakEntity, Window,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
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
        CommandPalette,
        InsertSnippet,
        SaveSnippet,
        EditSnippets,
        LockDeck,
        RewritePrompt,
        MyWords,
        NewControl,
        RestartControl,
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

/// A shortcut you can change in Settings › Keyboard.
pub struct Shortcut {
    /// Its key in settings.json's `keys`.
    pub id: &'static str,
    pub name: &'static str,
    /// The section Settings lists it under.
    pub group: &'static str,
    /// What Deck ships with; the first is the one shown.
    pub defaults: &'static [&'static str],
    context: Option<&'static str>,
    bind: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! shortcut {
    ($id:literal, $name:literal, $group:literal, [$($k:literal),*], $action:expr) => {
        shortcut!($id, $name, $group, [$($k),*], $action, None)
    };
    ($id:literal, $name:literal, $group:literal, [$($k:literal),*], $action:expr, $ctx:expr) => {
        Shortcut {
            id: $id,
            name: $name,
            group: $group,
            defaults: &[$($k),*],
            context: $ctx,
            bind: |keys, ctx| KeyBinding::new(keys, $action, ctx),
        }
    };
}

/// ⌘N the new tab page, ⌘T a shell, ⌘L the browser: only ⌘ by default,
/// since in a terminal ⌃L clears the screen and ⌃N / ⌃T are editing keys.
pub static SHORTCUTS: &[Shortcut] = &[
    shortcut!("new-tab", "New tab page", "Tabs", ["cmd-n"], NewTab),
    shortcut!(
        "new-terminal",
        "New terminal",
        "Tabs",
        ["cmd-t"],
        NewTerminal
    ),
    shortcut!("new-chat", "New chat", "Tabs", ["cmd-shift-n"], NewChat),
    shortcut!(
        "new-browser",
        "New browser tab",
        "Tabs",
        ["cmd-shift-t"],
        NewBrowser
    ),
    shortcut!("agents", "Agents", "Tabs", ["cmd-shift-a"], ToggleAgents),
    shortcut!("close", "Close pane", "Tabs", ["cmd-w"], ClosePane),
    shortcut!(
        "next-tab",
        "Next tab",
        "Tabs",
        ["cmd-shift-]", "ctrl-tab"],
        NextTab
    ),
    shortcut!(
        "prev-tab",
        "Previous tab",
        "Tabs",
        ["cmd-shift-[", "ctrl-shift-tab"],
        PrevTab
    ),
    shortcut!("split-right", "Split right", "Panes", ["cmd-d"], SplitRight),
    shortcut!(
        "split-down",
        "Split down",
        "Panes",
        ["cmd-shift-d"],
        SplitDown
    ),
    shortcut!("next-pane", "Next pane", "Panes", ["cmd-]"], NextPane),
    shortcut!("prev-pane", "Previous pane", "Panes", ["cmd-["], PrevPane),
    shortcut!(
        "palette",
        "Command palette",
        "Window",
        ["cmd-k"],
        CommandPalette
    ),
    shortcut!(
        "snippet",
        "Insert snippet",
        "Window",
        ["cmd-;"],
        InsertSnippet
    ),
    shortcut!("settings", "Settings", "Window", ["cmd-,"], OpenSettings),
    shortcut!(
        "sidebar",
        "Show or hide the sidebar",
        "Window",
        ["cmd-b"],
        ToggleSidebar
    ),
    shortcut!(
        "inspector",
        "Show or hide the inspector",
        "Window",
        ["cmd-i"],
        ToggleInspector
    ),
    shortcut!("zoom-in", "Zoom in", "Window", ["cmd-=", "cmd-+"], ZoomIn),
    shortcut!("zoom-out", "Zoom out", "Window", ["cmd--"], ZoomOut),
    shortcut!("zoom-reset", "Actual size", "Window", ["cmd-0"], ZoomReset),
    shortcut!("lock", "Lock Deck", "Window", ["cmd-shift-l"], LockDeck),
    shortcut!("address", "Address bar", "Browser", ["cmd-l"], FocusAddress),
    shortcut!("reload", "Reload page", "Browser", ["cmd-r"], Reload),
    shortcut!(
        "devtools",
        "Web inspector",
        "Browser",
        ["alt-cmd-i"],
        ToggleDevTools
    ),
    shortcut!(
        "clear",
        "Clear terminal",
        "Terminal",
        ["cmd-shift-k"],
        ClearTerminal,
        Some("Terminal")
    ),
    shortcut!(
        "rewrite",
        "Rewrite the prompt",
        "Chat",
        ["ctrl-o"],
        RewritePrompt,
        Some("Composer")
    ),
    shortcut!(
        "my-words",
        "Bring back my words",
        "Chat",
        ["ctrl-z"],
        MyWords,
        Some("Composer")
    ),
];

impl Shortcut {
    /// The keys it answers to: yours from Settings (empty: none), else
    /// Deck's.
    pub fn keys<'a>(
        &'a self,
        keys: &'a std::collections::BTreeMap<String, String>,
    ) -> Vec<&'a str> {
        match keys.get(self.id) {
            Some(k) if k.is_empty() => Vec::new(),
            Some(k) => vec![k.as_str()],
            None => self.defaults.to_vec(),
        }
    }
}

/// The shortcut for `id` as the UI shows it ("⇧⌘N"), or "" when it has none.
pub fn hint(id: &str, keys: &std::collections::BTreeMap<String, String>) -> SharedString {
    SHORTCUTS
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.keys(keys).first().map(|k| crate::keys::pretty(k)))
        .unwrap_or_default()
        .into()
}

pub fn install(cx: &mut App) {
    rebind(cx, &crate::settings::load().keys);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// Put the keymap in place with your shortcuts from Settings, and the menus
/// that show them. At launch, and whenever a shortcut changes.
pub fn rebind(cx: &mut App, mine: &std::collections::BTreeMap<String, String>) {
    cx.clear_key_bindings();
    let mut keys = Vec::new();
    for s in SHORTCUTS {
        for k in s.keys(mine) {
            // A key settings.json holds that does not parse is skipped, not
            // a crash.
            if gpui::Keystroke::parse(k).is_ok() {
                keys.push((s.bind)(k, s.context));
            }
        }
    }
    keys.extend([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-f", Find, Some("Terminal")),
        KeyBinding::new("cmd-g", FindNext, Some("Terminal")),
        KeyBinding::new("cmd-shift-g", FindPrevious, Some("Terminal")),
        // Anywhere else, find goes to the focused page.
        KeyBinding::new("cmd-f", Find, None),
        KeyBinding::new("cmd-g", FindNext, None),
        KeyBinding::new("cmd-shift-g", FindPrevious, None),
    ]);
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
    // Menus are not decoration here: AppKit routes ⌘ shortcuts through them,
    // so they keep working while a webview holds the keyboard.
    cx.set_menus(vec![
        Menu {
            name: "Deck".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("Lock Deck", LockDeck),
                MenuItem::separator(),
                MenuItem::action("Quit Deck", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Tab…", NewTab),
                MenuItem::action("New Terminal Tab", NewTerminal),
                MenuItem::action("New Chat", NewChat),
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
                MenuItem::separator(),
                MenuItem::action("Insert Snippet…", InsertSnippet),
                MenuItem::action("Save as Snippet", SaveSnippet),
                MenuItem::action("Edit Snippets…", EditSnippets),
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
                MenuItem::separator(),
                MenuItem::action("Command Palette…", CommandPalette),
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
    /// Children side by side along the axis, each given its share of the
    /// space by weight (dragging a divider moves weight between two).
    Split(Axis, Vec<Node>, Vec<f32>),
}

impl Node {
    /// Put `new` beside `target`. Joins the parent run when it already flows
    /// the same way, as i3 does, so three splits right are three equal columns
    /// rather than a staircase.
    fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split(
                    axis,
                    vec![Node::Leaf(target), Node::Leaf(new)],
                    vec![1., 1.],
                );
                true
            }
            Node::Leaf(_) => false,
            Node::Split(a, children, sizes) => {
                if *a == axis {
                    if let Some(i) = children.iter().position(|c| *c == Node::Leaf(target)) {
                        // The new pane takes half of the one it splits.
                        let half = sizes[i] / 2.;
                        sizes[i] = half;
                        sizes.insert(i + 1, half);
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
        let Node::Split(_, children, sizes) = self else {
            return false;
        };
        let removed = match children.iter().position(|c| *c == Node::Leaf(target)) {
            Some(i) => {
                children.remove(i);
                sizes.remove(i);
                true
            }
            None => children.iter_mut().any(|c| c.remove(target)),
        };
        if children.len() == 1 {
            *self = children.pop().expect("one child");
        }
        removed
    }

    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split(_, children, _) => children.iter().for_each(|c| c.leaves(out)),
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(x) => *x == id,
            Node::Split(_, children, _) => children.iter().any(|c| c.contains(id)),
        }
    }

    /// The pane a subtree starts with; with its axis, it names a split's
    /// child for as long as the drag on its divider lasts.
    fn first(&self) -> PaneId {
        match self {
            Node::Leaf(id) => *id,
            Node::Split(_, children, _) => children[0].first(),
        }
    }

    /// Set the weights of the two neighbours whose subtrees start with `a`
    /// and `b`, wherever that split is.
    fn resize(&mut self, a: PaneId, b: PaneId, wa: f32, wb: f32) -> bool {
        let Node::Split(_, children, sizes) = self else {
            return false;
        };
        if let Some(i) = (0..children.len().saturating_sub(1))
            .find(|&i| children[i].first() == a && children[i + 1].first() == b)
        {
            sizes[i] = wa;
            sizes[i + 1] = wb;
            return true;
        }
        children.iter_mut().any(|c| c.resize(a, b, wa, wb))
    }

    /// The current weights of those neighbours.
    fn weights(&self, a: PaneId, b: PaneId) -> Option<(f32, f32)> {
        let Node::Split(_, children, sizes) = self else {
            return None;
        };
        (0..children.len().saturating_sub(1))
            .find(|&i| children[i].first() == a && children[i + 1].first() == b)
            .map(|i| (sizes[i], sizes[i + 1]))
            .or_else(|| children.iter().find_map(|c| c.weights(a, b)))
    }

    /// Even out a split's children again (double-click on a divider).
    fn even(&mut self, a: PaneId, b: PaneId) -> bool {
        let Node::Split(_, children, sizes) = self else {
            return false;
        };
        if (0..children.len().saturating_sub(1))
            .any(|i| children[i].first() == a && children[i + 1].first() == b)
        {
            sizes.iter_mut().for_each(|w| *w = 1.);
            return true;
        }
        children.iter_mut().any(|c| c.even(a, b))
    }

    /// Put `new` where `old` is, keeping the layout around it.
    fn replace(&mut self, old: PaneId, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == old => {
                *id = new;
                true
            }
            Node::Leaf(_) => false,
            Node::Split(_, children, _) => children.iter_mut().any(|c| c.replace(old, new)),
        }
    }
}

enum PaneKind {
    Terminal(Entity<TerminalPane>),
    Browser(Entity<BrowserPane>),
    Chat(Entity<ChatPane>),
    /// The one Settings tab (⌘,).
    Settings(Entity<SettingsPane>),
    /// A new tab that has not been told what to be yet (⌘N).
    Launcher(Entity<LauncherPane>),
    /// A task's changes, to merge, open as a PR or discard.
    Review(Entity<ReviewPane>),
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
    /// The review of the task with this id.
    Review(String),
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
    /// A divider being dragged.
    resizing: Option<Resize>,
    /// Claude Code terminals deck has unstuck from an API wait: how many
    /// times in a row, and when last.
    stalls: HashMap<PaneId, (u32, Instant)>,
    /// When each agent started its current stretch of work, so finishing a
    /// long one can say so.
    working_since: HashMap<PaneId, Instant>,
    /// Where each split cell was last laid out, by the pane it starts with.
    cells: std::rc::Rc<std::cell::RefCell<HashMap<PaneId, gpui::Bounds<gpui::Pixels>>>>,
    /// The last notification each pane posted, so one prompt is one banner.
    alerted: HashMap<PaneId, (Instant, String)>,
    /// What the Dock badge says: how many panes want you.
    badge: usize,
    /// Set once the last session's tabs are back; until then nothing is
    /// saved, so a slow start cannot overwrite the layout with an empty one.
    restored: bool,
    /// The layout as last written, so an unchanged one is not written again.
    saved_session: Option<crate::session::Session>,
    /// What each restored pane was saved as, and when the restore happened:
    /// an agent takes a moment to start, and until it is seen running the
    /// saved state stands in for it, so an early save cannot drop it.
    restored_as: HashMap<PaneId, crate::session::PaneState>,
    restored_at: Option<Instant>,
    /// Every task, open or finished, as kept in tasks.json.
    tasks: Vec<crate::tasks::Task>,
    /// What each open task has changed so far, refreshed by the tick.
    task_changes: HashMap<String, crate::tasks::Changes>,
    /// The command palette while it is open, and the pane that had the
    /// keyboard before it.
    palette: Option<(Entity<Palette>, Option<PaneId>, Subscription)>,
    /// A one-line message at the foot of the window, and when it goes.
    toast: Option<(SharedString, Instant)>,
    /// A card in the corner that asks something (make deck the default
    /// terminal, restart for an update) until it is answered or closed.
    notice: Option<Notice>,
    /// Locked (⇧⌘L): only the lock screen is drawn until you unlock.
    locked: bool,
    /// The browser is open on the sign-in page.
    signing_in: bool,
    /// The last account problem, shown on the sign-in screen.
    account_note: Option<SharedString>,
    /// Pages are hidden for the lock or sign-in screen.
    screen_cover: bool,
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
                crate::refine::set(s.refine, &s.refine_model);
                crate::retry::set_enabled(s.auto_retry);
                wear(s.theme, window);
                s
            },
            statuses: Vec::new(),
            unseen: Default::default(),
            islands: None,
            places: HashMap::new(),
            busy: Default::default(),
            resizing: None,
            working_since: HashMap::new(),
            stalls: HashMap::new(),
            cells: Default::default(),
            alerted: HashMap::new(),
            badge: 0,
            restored: false,
            saved_session: None,
            restored_as: HashMap::new(),
            tasks: crate::tasks::load_all(),
            task_changes: HashMap::new(),
            palette: None,
            toast: None,
            restored_at: None,
            ticks: 0,
            automation: std::env::var("DECK_AUTOMATION_TOKEN")
                .ok()
                .filter(|t| t.len() >= 16),
            control: None,
            menu: None,
            notice: None,
            locked: false,
            signing_in: false,
            account_note: None,
            screen_cover: false,
            _requests: None,
            _tick: tick,
        };
        // Scripts and links macOS opened deck with, now and later.
        if let Some(mut opened) = crate::handler::receiver() {
            cx.spawn_in(window, async move |this, cx| {
                use futures::StreamExt;
                while let Some(url) = opened.next().await {
                    let ok = this.update_in(cx, |ws, window, cx| ws.open_handled(&url, window, cx));
                    if ok.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // Probing every agent CLI takes a moment; the window does not wait.
        // When it is ready, the control chat opens as the first tab.
        cx.spawn_in(window, async move |this, cx| {
            let (env, saved) = cx
                .background_executor()
                .spawn(async {
                    let loaded = (ChatEnv::start(), crate::threads::load_all());
                    crate::threads::prune_attachments();
                    loaded
                })
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
                ws.restore(saved, window, cx);
                ws.offer_default_terminal(cx);
                ws.start_account(cx);
                ws.start_update_checks(cx);
            });
        })
        .detach();
        // The System theme follows macOS from light to dark and back.
        cx.observe_window_appearance(window, |ws, window, cx| {
            if ws.settings.theme == crate::theme::Theme::System {
                wear(ws.settings.theme, window);
                ws.restyle(window, cx);
            }
        })
        .detach();
        // A click on one of deck's notifications brings its pane forward.
        if let Some(mut clicks) = crate::notify::start() {
            cx.spawn_in(window, async move |this, cx| {
                while let Some(pane) = clicks.next().await {
                    let _ = this.update_in(cx, |ws, window, cx| {
                        if ws.panes.contains_key(&pane) {
                            window.activate_window();
                            ws.focus_pane(pane, window, cx);
                        }
                    });
                }
            })
            .detach();
        }
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
                            TerminalEvent::Alert(said) => {
                                let said = said.clone();
                                ws.alert(id, said, window, cx);
                            }
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
            Open::Review(task) => self.make_review(id, &task, window, cx)?,
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
        // A restored layout may already hold the control chat; then the
        // other threads still come back as tabs of their own.
        let pinned = self.tabs.iter().any(|t| t.pinned);
        let control = saved
            .iter()
            .rposition(|s| s.control && !pinned)
            .map(|i| saved.remove(i));
        let agent = self.default_agent();
        let what = match control {
            Some(s) => Some(Open::Thread(Box::new(s))),
            None if pinned => None,
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
            None if self.tabs.is_empty() && !pinned => {
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

    /// Bring back the last session's tabs, splits and focus, then the
    /// threads it did not mention (and the control chat, if it had none).
    fn restore(
        &mut self,
        saved: Vec<crate::threads::Saved>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::session;
        let Some(last) = session::load() else {
            self.open_control(saved, window, cx);
            self.restored = true;
            return;
        };
        let mut threads: HashMap<String, crate::threads::Saved> =
            saved.into_iter().map(|s| (s.id.clone(), s)).collect();
        let mut back: Vec<Tab> = Vec::new();
        // The tab to land on, by its focused pane: indices move as the
        // control chat is put first.
        let mut active = None;
        for (ix, tab) in last.tabs.iter().enumerate() {
            let mut leaves = Vec::new();
            let Some(root) = self.rebuild(&tab.root, &mut threads, &mut leaves, window, cx) else {
                continue;
            };
            // The control chat is the pinned tab only if its thread is.
            let pinned = tab.pinned
                && leaves
                    .iter()
                    .any(|id| match self.panes.get(id).map(|p| &p.kind) {
                        Some(PaneKind::Chat(c)) => c.read(cx).control,
                        _ => false,
                    });
            let focused = leaves
                .get(tab.focused)
                .or(leaves.first())
                .copied()
                .unwrap_or(0);
            if ix <= last.active {
                active = Some(focused);
            }
            back.push(Tab {
                focused,
                root,
                pinned,
            });
        }
        // Tabs opened before the restore finished (`deck open`) stay, after.
        let opened = std::mem::take(&mut self.tabs);
        let had_any = !back.is_empty();
        self.tabs = back;
        self.tabs.extend(opened);
        self.sidebar = last.sidebar;
        self.inspector.open = last.inspector;
        let mut rest: Vec<_> = threads.into_values().collect();
        rest.sort_by_key(|s| s.updated);
        self.open_control(rest, window, cx);
        // The pinned control chat goes first, wherever it was saved.
        if let Some(i) = self.tabs.iter().position(|t| t.pinned) {
            if i != 0 {
                let t = self.tabs.remove(i);
                self.tabs.insert(0, t);
            }
        }
        if let Some(ix) = active.filter(|_| had_any).and_then(|id| self.tab_of(id)) {
            self.activate(ix, window, cx);
        }
        self.restored = true;
        self.restored_at = Some(Instant::now());
        cx.notify();
    }

    /// One saved node back as panes; a pane that cannot come back (its
    /// thread was archived, a page with no address) is left out, and a split
    /// left with one child becomes that child.
    fn rebuild(
        &mut self,
        node: &crate::session::NodeState,
        threads: &mut HashMap<String, crate::threads::Saved>,
        leaves: &mut Vec<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Node> {
        use crate::session::{NodeState, PaneState};
        match node {
            NodeState::Split {
                row,
                children,
                sizes,
            } => {
                let mut kids: Vec<Node> = Vec::new();
                let mut weights: Vec<f32> = Vec::new();
                for (i, c) in children.iter().enumerate() {
                    if let Some(k) = self.rebuild(c, threads, leaves, window, cx) {
                        kids.push(k);
                        weights.push(
                            sizes
                                .get(i)
                                .copied()
                                .filter(|w| w.is_finite() && *w > 0.)
                                .unwrap_or(1.),
                        );
                    }
                }
                match kids.len() {
                    0 => None,
                    1 => kids.pop(),
                    _ => Some(Node::Split(
                        if *row { Axis::Row } else { Axis::Column },
                        kids,
                        weights,
                    )),
                }
            }
            NodeState::Pane(p) => {
                let (what, agent) = match p {
                    PaneState::Terminal { cwd, agent } => {
                        let cwd = cwd.clone().filter(|c| c.is_dir());
                        let tab = agent
                            .as_ref()
                            .filter(|a| a.tab)
                            .and_then(|a| AGENTS.iter().find(|x| x.cmd == a.cmd));
                        (
                            Open::Terminal {
                                cmd: agent.as_ref().map(crate::session::resume_line),
                                cwd,
                            },
                            tab,
                        )
                    }
                    PaneState::Browser { url } => (Open::Browser { url: url.clone() }, None),
                    PaneState::Chat { thread } => {
                        let saved = threads.remove(thread)?;
                        (Open::Thread(Box::new(saved)), None)
                    }
                    PaneState::Settings => (Open::Settings, None),
                };
                let control = matches!(&what, Open::Thread(s) if s.control);
                let id = self
                    .make_pane(what, agent, window, cx)
                    .map_err(|e| tracing::warn!("could not restore a pane: {e}"))
                    .ok()?;
                self.restored_as.insert(id, p.clone());
                if control {
                    if let Some(PaneKind::Chat(c)) = self.panes.get(&id).map(|p| &p.kind) {
                        c.update(cx, |c, _| c.control = true);
                    }
                }
                leaves.push(id);
                Some(Node::Leaf(id))
            }
        }
    }

    /// The layout as it stands, in the form it is saved.
    fn snapshot(&self, cx: &App) -> crate::session::Session {
        use crate::session::{AgentRun, NodeState, PaneState, Session, TabState};
        // Several Claude Code tabs in one folder: the one that printed last
        // is the conversation written last.
        let mut claude: HashMap<PathBuf, Vec<(Instant, PaneId)>> = HashMap::new();
        for (&id, pane) in &self.panes {
            if let (PaneKind::Terminal(t), Some(place)) = (&pane.kind, self.places.get(&id)) {
                if matches!(place.agent, Some(("claude", _))) {
                    claude
                        .entry(place.cwd.clone())
                        .or_default()
                        .push((t.read(cx).last_output, id));
                }
            }
        }
        // deck's own chats run Claude Code too, and write their
        // conversations to the same folder; those are not a terminal's.
        let chats: std::collections::HashSet<String> = self
            .panes
            .values()
            .filter_map(|p| match &p.kind {
                PaneKind::Chat(c) => c.read(cx).native_id().map(str::to_string),
                _ => None,
            })
            .collect();
        let mut conversation: HashMap<PaneId, String> = HashMap::new();
        for (cwd, mut panes) in claude {
            panes.sort_by_key(|p| std::cmp::Reverse(p.0));
            let sessions = crate::session::claude_sessions(&cwd)
                .into_iter()
                .filter(|s| !chats.contains(s));
            for ((_, id), session) in panes.into_iter().zip(sessions) {
                conversation.insert(id, session);
            }
        }

        // Right after a restore, a terminal not yet seen with its agent
        // running is still what it was saved as.
        let settling = self
            .restored_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(20));
        let pane_state = |id: PaneId| -> Option<PaneState> {
            let pane = self.panes.get(&id)?;
            Some(match &pane.kind {
                PaneKind::Terminal(_) => {
                    let place = self.places.get(&id);
                    let saved = self.restored_as.get(&id);
                    let waiting = matches!(saved, Some(PaneState::Terminal { agent: Some(_), .. }))
                        && settling
                        && place.is_none_or(|p| p.agent.is_none());
                    if let (true, Some(saved)) = (place.is_none() || waiting, saved) {
                        return Some(saved.clone());
                    }
                    PaneState::Terminal {
                        cwd: place.map(|p| p.cwd.clone()),
                        agent: place
                            .and_then(|p| p.agent.as_ref())
                            .map(|(cmd, argv)| AgentRun {
                                cmd: cmd.to_string(),
                                flags: crate::session::keep_flags(cmd, argv),
                                session: conversation.get(&id).cloned(),
                                tab: pane.agent.is_some(),
                            }),
                    }
                }
                PaneKind::Browser(b) => PaneState::Browser {
                    url: b.read(cx).url.to_string(),
                },
                PaneKind::Chat(c) => PaneState::Chat {
                    thread: c.read(cx).thread_id().to_string(),
                },
                PaneKind::Settings(_) => PaneState::Settings,
                // An empty new tab is not worth bringing back.
                PaneKind::Launcher(_) | PaneKind::Review(_) => return None,
            })
        };
        fn node_state(
            node: &Node,
            pane: &dyn Fn(PaneId) -> Option<PaneState>,
            kept: &mut Vec<PaneId>,
        ) -> Option<NodeState> {
            match node {
                Node::Leaf(id) => {
                    let p = pane(*id)?;
                    kept.push(*id);
                    Some(NodeState::Pane(p))
                }
                Node::Split(axis, children, weights) => {
                    let mut kids: Vec<NodeState> = Vec::new();
                    let mut sizes: Vec<f32> = Vec::new();
                    for (c, w) in children.iter().zip(weights) {
                        if let Some(k) = node_state(c, pane, kept) {
                            kids.push(k);
                            sizes.push(*w);
                        }
                    }
                    match kids.len() {
                        0 => None,
                        1 => kids.pop(),
                        _ => Some(NodeState::Split {
                            row: *axis == Axis::Row,
                            children: kids,
                            sizes,
                        }),
                    }
                }
            }
        }

        let mut tabs = Vec::new();
        let mut active = 0;
        for (ix, tab) in self.tabs.iter().enumerate() {
            let mut kept = Vec::new();
            let Some(root) = node_state(&tab.root, &pane_state, &mut kept) else {
                continue;
            };
            if ix <= self.active {
                active = tabs.len();
            }
            tabs.push(TabState {
                pinned: tab.pinned,
                focused: kept.iter().position(|&p| p == tab.focused).unwrap_or(0),
                root,
            });
        }
        Session {
            tabs,
            active,
            sidebar: self.sidebar,
            inspector: self.inspector.open,
        }
    }

    /// Write the layout when it has changed (checked once a second).
    fn save_session(&mut self, cx: &App) {
        if !self.restored {
            return;
        }
        let now = self.snapshot(cx);
        if self.saved_session.as_ref() != Some(&now) {
            self.saved_session = Some(now.clone());
            std::thread::spawn(move || crate::session::save(&now));
        }
    }

    /// Start the control chat over: its conversation is filed away like a
    /// closed thread, and a fresh one opens in its place.
    fn new_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.control_tab() {
            self.close_tab(ix, window, cx);
        }
        self.open_control(Vec::new(), window, cx);
        if let Some(ix) = self.control_tab() {
            self.activate(ix, window, cx);
        }
    }

    /// Restart the control chat's agent: the same conversation, a new
    /// process (Claude resumes its own session).
    fn restart_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.control_tab() else {
            return self.open_control(Vec::new(), window, cx);
        };
        let old = self.tabs[ix].root.first();
        let Some(PaneKind::Chat(c)) = self.panes.get(&old).map(|p| &p.kind) else {
            return;
        };
        let mut saved = c.read(cx).saved(cx);
        saved.control = true;
        let new = match self.make_pane(Open::Thread(Box::new(saved)), None, window, cx) {
            Ok(id) => id,
            Err(e) => return self.flash(format!("Could not restart the agent: {e}"), cx),
        };
        if let Some(PaneKind::Chat(c)) = self.panes.get(&new).map(|p| &p.kind) {
            c.update(cx, |c, _| c.control = true);
        }
        let tab = &mut self.tabs[ix];
        tab.root.replace(old, new);
        if tab.focused == old {
            tab.focused = new;
        }
        // Not archived: the new pane carries the same thread on.
        self.panes.remove(&old);
        self.focus_pane(new, window, cx);
        self.flash("Agent restarted", cx);
    }

    fn open_control_menu(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::menu::{Entry, Item};
        let ws = cx.entity().downgrade();
        let open = self.control_tab().is_some();
        let (fresh, again, close) = (ws.clone(), ws.clone(), ws);
        let entries: Vec<Entry> = vec![
            Item::new("New Conversation", move |window, cx| {
                let _ = fresh.update(cx, |ws, cx| ws.new_control(window, cx));
            })
            .icon(Icon::Plus)
            .into(),
            Item::new("Restart Agent", move |window, cx| {
                let _ = again.update(cx, |ws, cx| ws.restart_control(window, cx));
            })
            .icon(Icon::Refresh)
            .disabled(!open)
            .into(),
            Entry::Separator,
            Item::new("Close", move |window, cx| {
                let _ = close.update(cx, |ws, cx| {
                    if let Some(ix) = ws.control_tab() {
                        ws.close_tab(ix, window, cx);
                    }
                });
            })
            .icon(Icon::Close)
            .shortcut("⌘W")
            .disabled(!open)
            .into(),
        ];
        let restore = window.focused(cx);
        self.menu = Some(crate::menu::open(
            entries,
            at,
            restore,
            window,
            cx,
            |ws: &mut Self, _, cx| {
                ws.menu = None;
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn control_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.pinned)
    }

    /// The Settings tab while it is open on its own. The sidebar's footer
    /// stands for it then, rather than a second Settings row above.
    fn settings_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| match t.root {
            Node::Leaf(id) => matches!(
                self.panes.get(&id).map(|p| &p.kind),
                Some(PaneKind::Settings(_))
            ),
            Node::Split(..) => false,
        })
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
                env.insert(
                    "PATH".into(),
                    crate::tidy_path(&format!("{}:{path}", dir.display())),
                );
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
                t.update(cx, |t, cx| t.seen(cx));
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
            Some(PaneKind::Review(r)) => {
                window.focus(&r.read(cx).focus);
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
        self.working_since.remove(&id);
        self.restored_as.remove(&id);
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
        let looking = window.is_window_active();
        let mut ask: Vec<(PaneId, String)> = Vec::new();
        for (id, now) in &sorted {
            let was = self.statuses.iter().find(|(p, _)| p == id).map(|(_, s)| *s);
            if was == Some(Status::Working) && *now != Status::Working && focused != Some(*id) {
                self.unseen.insert(*id);
            }
            // A chat that started waiting on you, while you look elsewhere.
            if *now == Status::Attention
                && was != Some(Status::Attention)
                && !(looking && focused == Some(*id))
            {
                if let Some(PaneKind::Chat(c)) = self.panes.get(id).map(|p| &p.kind) {
                    let c = c.read(cx);
                    if let Some(why) = c.waiting_for() {
                        ask.push((*id, format!("{}: {why}", c.agent_name())));
                    }
                }
            }
        }
        // Agents (chats and agent terminals alike) that just stopped
        // working, after long enough that you may have looked away.
        let mut done: Vec<(PaneId, String)> = Vec::new();
        for (id, now) in &sorted {
            let was = self.statuses.iter().find(|(p, _)| p == id).map(|(_, s)| *s);
            if *now == Status::Working {
                self.working_since.entry(*id).or_insert_with(Instant::now);
            } else if let Some(since) = self.working_since.remove(id) {
                let long = since.elapsed() >= Duration::from_secs(10);
                if long
                    && was == Some(Status::Working)
                    && *now == Status::Idle
                    && !(looking && focused == Some(*id))
                {
                    done.push((*id, self.finished_line(*id, cx)));
                }
            }
        }
        for (id, now) in &sorted {
            let was = self.statuses.iter().find(|(p, _)| p == id).map(|(_, s)| *s);
            if was == Some(Status::Working) && *now == Status::Idle {
                self.report_to_parent(*id, cx);
            }
        }
        if self.settings.notify_attention {
            for (id, body) in ask {
                self.post(id, body, cx);
            }
        }
        if self.settings.notify_done {
            for (id, body) in done {
                self.post(id, body, cx);
            }
        }
        let wanting = sorted
            .iter()
            .filter(|(_, s)| *s == Status::Attention)
            .count();
        if wanting != self.badge {
            self.badge = wanting;
            set_badge(wanting);
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
        self.save_session(cx);
        if self.ticks.is_multiple_of(3) {
            self.refresh_task_changes(cx);
            self.unstick_agents(cx);
        }
    }

    /// The project each terminal and chat is in, and what a terminal is
    /// running, for the tab titles. libproc only.
    fn refresh_places(&mut self, cx: &mut Context<Self>) {
        let mut next = HashMap::new();
        for (&id, pane) in &self.panes {
            let place = match &pane.kind {
                PaneKind::Terminal(t) => {
                    // What is in front is the terminal's foreground process
                    // group, not the deepest process: an agent's helpers
                    // (MCP servers, `caffeinate`) sit below it.
                    let shell = t.read(cx).pid();
                    let fg = inspect::foreground(shell).unwrap_or(shell);
                    let mut front = (fg, inspect::process_name(fg));
                    let mut agent = agent_of(fg, &front.1);
                    // An agent tab runs its CLI under `sh -c`, which has no job
                    // control: the CLI shares the shell's group, so the group
                    // leader is the shell. Look below it for the agent.
                    if agent.is_none() && is_shell(&front.1) {
                        if let Some((p, a)) = inspect::descendants(fg)
                            .into_iter()
                            .skip(1)
                            .find_map(|p| agent_of(p.pid, &p.name).map(|a| (p, a)))
                        {
                            front = (p.pid, p.name);
                            agent = Some(a);
                        }
                    }
                    let cwd = inspect::cwd(front.0).or_else(|| inspect::cwd(shell));
                    cwd.map(|cwd| Place {
                        agent: agent.map(|(a, argv)| (a.cmd, argv)),
                        front: Some(front.1),
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
                // What the program says beats a guess: OSC 9;4 progress, or
                // Claude Code's title. Its screen keeps changing while idle
                // (the status line), so recent output alone is not work.
                PaneKind::Terminal(t) => match t.read(cx).activity() {
                    Some(working) => working,
                    None => {
                        t.read(cx).last_output.elapsed() < Duration::from_millis(1500)
                            && next
                                .get(id)
                                .and_then(|pl| pl.front.as_deref())
                                .is_some_and(|f| f != shell && !f.starts_with('-'))
                    }
                },
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
                PaneKind::Review(r) => Some(r.read(cx).task.worktree.clone()),
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
        let themed = next.theme != self.settings.theme;
        if next.keys != self.settings.keys {
            rebind(cx, &next.keys);
        }
        crate::browser::set_search_engine(next.search);
        crate::refine::set(next.refine, &next.refine_model);
        crate::retry::set_enabled(next.auto_retry);
        if themed {
            wear(next.theme, window);
        }
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
                PaneKind::Chat(_) | PaneKind::Launcher(_) | PaneKind::Review(_) => {}
            }
        }
        if zoomed || themed {
            // The UI follows through the rem size and the theme, both read
            // as it draws; cached panes would keep their old frame, so
            // everything is drawn once more.
            window.refresh();
        }
        cx.notify();
    }

    /// The theme changed under us (System, with macOS switching): the
    /// terminals' palette follows, then everything redraws.
    fn restyle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let look = self.settings.term_look();
        for pane in self.panes.values() {
            if let PaneKind::Terminal(t) = &pane.kind {
                let look = look.clone();
                t.update(cx, |t, cx| t.set_look(look, window, cx));
            }
        }
        window.refresh();
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
        let sub = cx.subscribe_in(
            &s,
            window,
            |ws, _, ev: &SettingsEvent, window, cx| match ev {
                SettingsEvent::Changed(next) => ws.apply_settings(next.clone(), window, cx),
                SettingsEvent::DefaultTerminal(on) => ws.make_default_terminal(*on, window, cx),
                SettingsEvent::Account(a) => ws.account_action(*a, cx),
                SettingsEvent::Update(a) => ws.update_action(*a, cx),
            },
        );
        (PaneKind::Settings(s), None, sub)
    }

    // -- command palette -----------------------------------------------------

    /// ⌘K: open the palette over everything (pages step aside, since a
    /// native view would cover it), or close it if it is open.
    /// ⌘K, or with `snippets_only` ⌘;: the snippets alone.
    fn toggle_palette(&mut self, snippets_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        let from = self.focused();
        let cwd = self.focused_cwd(cx).unwrap_or_else(|| self.cwd.clone());
        let in_repo = cwd.ancestors().any(|d| d.join(".git").exists());
        let palette = if snippets_only {
            let items = snippet_items(Some(&crate::inspect::project_root(&cwd)));
            cx.new(|cx| Palette::snippets(items, cx))
        } else {
            let items = self.palette_items(cx);
            cx.new(|cx| Palette::new(items, in_repo, cx))
        };
        let sub = cx.subscribe_in(
            &palette,
            window,
            move |ws, _, ev: &PaletteEvent, window, cx| match ev {
                PaletteEvent::Close => ws.close_palette(window, cx),
                PaletteEvent::Run(pick) => {
                    let pick = pick.clone();
                    ws.close_palette(window, cx);
                    ws.run_pick(pick, window, cx);
                }
                PaletteEvent::Query(_) if snippets_only => {}
                PaletteEvent::Query(q) => {
                    let pages = crate::history::search(q, 6)
                        .into_iter()
                        .map(|v| Item {
                            kind: Kind::Page,
                            title: if v.title.is_empty() {
                                crate::history::strip(&v.url).to_string().into()
                            } else {
                                v.title.clone().into()
                            },
                            detail: crate::history::strip(&v.url).to_string().into(),
                            icon: Icon::Globe,
                            hint: None,
                            pick: crate::palette::Pick::Page(v.url),
                        })
                        .collect();
                    if let Some((p, ..)) = &ws.palette {
                        p.update(cx, |p, cx| p.set_pages(pages, cx));
                    }
                }
            },
        );
        // The project's files arrive once git has listed them.
        let root = crate::inspect::project_root(&cwd);
        let weak = palette.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            if snippets_only {
                return;
            }
            let files = cx
                .background_executor()
                .spawn({
                    let root = root.clone();
                    async move { crate::chat::project_files_cached(&root) }
                })
                .await;
            let items: Vec<Item> = files
                .iter()
                .filter(|f| !f.ends_with('/'))
                .map(|f| {
                    let name = f.rsplit('/').next().unwrap_or(f).to_string();
                    let dir = f
                        .rsplit_once('/')
                        .map(|(d, _)| d.to_string())
                        .unwrap_or_default();
                    Item {
                        kind: Kind::File,
                        title: name.into(),
                        detail: dir.into(),
                        icon: Icon::File,
                        hint: None,
                        pick: crate::palette::Pick::File(root.join(f)),
                    }
                })
                .collect();
            let _ = weak.update(cx, |p, cx| p.add(items, cx));
        })
        .detach();
        let focus = palette.read(cx).focus(cx);
        self.palette = Some((palette, from, sub));
        self.cover_pages(true, cx);
        window.focus(&focus);
        crate::browser::reclaim_keyboard(window);
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, from, _)) = self.palette.take() {
            self.cover_pages(false, cx);
            if let Some(id) = from.filter(|id| self.panes.contains_key(id)) {
                self.focus_pane(id, window, cx);
            }
            cx.notify();
        }
    }

    /// Pages are native views gpui cannot draw over: hide them while
    /// something floats over the window.
    fn cover_pages(&self, covered: bool, cx: &mut Context<Self>) {
        for pane in self.panes.values() {
            if let PaneKind::Browser(b) = &pane.kind {
                b.update(cx, |b, _| b.set_covered(covered));
            }
        }
    }

    fn palette_items(&self, cx: &App) -> Vec<Item> {
        use crate::palette::Pick as P;
        let mut items = Vec::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let id = tab.focused;
            let (glyph, _) = self.tab_icon(tab.leaves()[0], cx);
            const HINTS: [&str; 9] = ["⌘1", "⌘2", "⌘3", "⌘4", "⌘5", "⌘6", "⌘7", "⌘8", "⌘9"];
            items.push(Item {
                kind: Kind::Tab,
                title: self.pane_title(id, cx),
                detail: self.pane_subtitle(id, cx).unwrap_or_default(),
                icon: glyph,
                hint: HINTS.get(ix).map(|h| SharedString::from(*h)),
                pick: P::Pane(id),
            });
        }
        for t in self
            .tasks
            .iter()
            .filter(|t| t.state == crate::tasks::State::Open)
        {
            items.push(Item {
                kind: Kind::Action,
                title: format!("Review: {}", t.title).into(),
                detail: t.branch.clone().into(),
                icon: Icon::Git,
                hint: None,
                pick: P::Review(t.id.clone()),
            });
        }
        // Shortcuts as they are now, yours from Settings or Deck's.
        let key = |id: &str| Some(hint(id, &self.settings.keys)).filter(|h| !h.is_empty());
        let action = |title: &'static str,
                      detail: &'static str,
                      icon: Icon,
                      hint: Option<SharedString>,
                      a: Box<dyn gpui::Action>| Item {
            kind: Kind::Action,
            title: title.into(),
            detail: detail.into(),
            icon,
            hint,
            pick: P::Action(a),
        };
        items.extend([
            action(
                "New Tab",
                "A terminal, a page or an agent",
                Icon::Plus,
                key("new-tab"),
                Box::new(NewTab),
            ),
            action(
                "New Terminal",
                "",
                Icon::Terminal,
                key("new-terminal"),
                Box::new(NewTerminal),
            ),
            action(
                "New Chat",
                "With the agent new chats start with",
                Icon::Claude,
                key("new-chat"),
                Box::new(NewChat),
            ),
            action(
                "New Control Conversation",
                "Start the control chat over",
                Icon::Plus,
                None,
                Box::new(NewControl),
            ),
            action(
                "Restart Control Agent",
                "The same conversation, a fresh agent",
                Icon::Refresh,
                None,
                Box::new(RestartControl),
            ),
            action(
                "Insert Snippet",
                "A saved message, into the composer or terminal",
                Icon::Snippet,
                key("snippet"),
                Box::new(InsertSnippet),
            ),
            action(
                "Save as Snippet",
                "What is in the composer, or the terminal selection",
                Icon::Snippet,
                None,
                Box::new(SaveSnippet),
            ),
            action(
                "Edit Snippets",
                "Name, change or remove saved messages",
                Icon::Pencil,
                None,
                Box::new(EditSnippets),
            ),
            action(
                "New Browser Tab",
                "",
                Icon::Globe,
                key("new-browser"),
                Box::new(NewBrowser),
            ),
            action(
                "Open Location",
                "The browser's address bar",
                Icon::Search,
                key("address"),
                Box::new(FocusAddress),
            ),
            action(
                "Settings",
                "Theme, terminal, browser, agents",
                Icon::Settings,
                key("settings"),
                Box::new(OpenSettings),
            ),
            action(
                "Split Right",
                "",
                Icon::Split,
                key("split-right"),
                Box::new(SplitRight),
            ),
            action(
                "Split Down",
                "",
                Icon::SplitDown,
                key("split-down"),
                Box::new(SplitDown),
            ),
            action(
                "Close",
                "The focused pane",
                Icon::Close,
                key("close"),
                Box::new(ClosePane),
            ),
            action(
                "Find",
                "In the terminal or page",
                Icon::Search,
                Some("⌘F".into()),
                Box::new(Find),
            ),
            action(
                "Clear Terminal",
                "Screen and scrollback",
                Icon::Eraser,
                key("clear"),
                Box::new(ClearTerminal),
            ),
            action(
                "Reload Page",
                "",
                Icon::Refresh,
                key("reload"),
                Box::new(Reload),
            ),
            action(
                "Web Inspector",
                "For the focused page",
                Icon::Info,
                key("devtools"),
                Box::new(ToggleDevTools),
            ),
            action(
                "Toggle Sidebar",
                "",
                Icon::Sidebar,
                key("sidebar"),
                Box::new(ToggleSidebar),
            ),
            action(
                "Toggle Inspector",
                "",
                Icon::PanelRight,
                key("inspector"),
                Box::new(ToggleInspector),
            ),
            action("Zoom In", "", Icon::Plus, key("zoom-in"), Box::new(ZoomIn)),
            action(
                "Zoom Out",
                "",
                Icon::Plus,
                key("zoom-out"),
                Box::new(ZoomOut),
            ),
            action(
                "Actual Size",
                "Zoom back to 100%",
                Icon::Plus,
                key("zoom-reset"),
                Box::new(ZoomReset),
            ),
            action(
                "Next Tab",
                "",
                Icon::ChevronDown,
                key("next-tab"),
                Box::new(NextTab),
            ),
            action(
                "Previous Tab",
                "",
                Icon::ChevronUp,
                key("prev-tab"),
                Box::new(PrevTab),
            ),
        ]);
        let root = self
            .focused_cwd(cx)
            .map(|d| crate::inspect::project_root(&d));
        items.extend(snippet_items(root.as_deref()));
        items
    }

    fn run_pick(
        &mut self,
        pick: crate::palette::Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::palette::Pick as P;
        match pick {
            P::Pane(id) => self.focus_pane(id, window, cx),
            P::Action(a) => window.dispatch_action(a, cx),
            P::File(path) => cx.open_with_system(&path),
            P::Page(url) => self.open_url_beside(&url, window, cx),
            P::NewTask(brief) => self.start_task(brief, window, cx),
            P::Review(task) => self.open_review(&task, window, cx),
            P::Snippet(body) => self.insert_snippet(&body, window, cx),
            P::SaveSnippet(text) => {
                let name = crate::snippets::add(&text);
                self.flash(format!("Kept \"{name}\" as a snippet. ⌘; puts it in."), cx);
            }
        }
    }

    // -- snippets -------------------------------------------------------------

    /// Put a snippet's text where you are typing: at the caret in a chat's
    /// composer, or at a terminal's prompt (as a paste, so nothing runs until
    /// you press ⏎). Anywhere else it goes on the clipboard.
    fn insert_snippet(&mut self, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.expand_snippet(body, cx);
        let kind = self
            .focused()
            .and_then(|id| self.panes.get(&id))
            .map(|p| match &p.kind {
                PaneKind::Chat(c) => Ok(c.clone()),
                PaneKind::Terminal(t) => Err(Some(t.clone())),
                _ => Err(None),
            });
        match kind {
            Some(Ok(chat)) => {
                let composer = chat.read(cx).composer.clone();
                composer.update(cx, |f, cx| f.insert(&text, cx));
                let focus = composer.read(cx).focus.clone();
                window.focus(&focus);
            }
            Some(Err(Some(term))) => term.update(cx, |t, _| t.paste_text(&text)),
            _ => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                self.flash("Snippet copied. ⌘V to paste it.", cx);
            }
        }
    }

    /// A snippet's text with its fill-ins: the clipboard, what is selected
    /// in the focused pane, and that pane's branch and folder.
    fn expand_snippet(&self, body: &str, cx: &App) -> String {
        let pane = self.focused().and_then(|id| self.panes.get(&id));
        let cwd = self.focused_cwd(cx);
        crate::snippets::expand(body, |var| match var {
            "clipboard" => cx.read_from_clipboard().and_then(|c| c.text()),
            "selection" => pane.and_then(|p| match &p.kind {
                PaneKind::Terminal(t) => t.read(cx).selection_text(),
                PaneKind::Chat(c) => {
                    let c = c.read(cx);
                    c.selected_text()
                        .or_else(|| c.composer.read(cx).selected_text().map(str::to_string))
                }
                _ => None,
            }),
            "branch" => cwd.as_deref().and_then(crate::snippets::branch),
            "folder" => cwd.as_ref().map(|d| d.display().to_string()),
            _ => None,
        })
    }

    /// Keep what you are writing as a snippet: the composer's text, or what
    /// is selected in the terminal. With neither, open the snippets to add
    /// one by hand.
    fn save_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .focused()
            .and_then(|id| self.panes.get(&id))
            .and_then(|p| match &p.kind {
                PaneKind::Chat(c) => Some(c.read(cx).composer.read(cx).text().to_string()),
                PaneKind::Terminal(t) => t.read(cx).selection_text(),
                _ => None,
            })
            .filter(|t| !t.trim().is_empty());
        match text {
            Some(text) => {
                let name = crate::snippets::add(&text);
                self.flash(format!("Kept \"{name}\" as a snippet. ⌘; puts it in."), cx);
            }
            None => self.edit_snippets(window, cx),
        }
    }

    fn edit_snippets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
        let pane = self.panes.values().find_map(|p| match &p.kind {
            PaneKind::Settings(s) => Some(s.clone()),
            _ => None,
        });
        if let Some(s) = pane {
            s.update(cx, |s, cx| s.show(crate::settings_pane::Page::Snippets, cx));
        }
    }

    // -- default terminal ----------------------------------------------------------

    /// A script or terminal link macOS opened deck with: a new terminal tab
    /// running it, in front.
    fn open_handled(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if url.starts_with("deck://") {
            self.deck_url(url, cx);
            cx.activate(true);
            return;
        }
        let Some((cmd, cwd)) = crate::handler::command_for(url) else {
            tracing::info!("nothing opens {url}");
            return;
        };
        if let Err(e) = self.new_tab(
            Open::Terminal {
                cmd: Some(cmd),
                cwd,
            },
            None,
            window,
            cx,
        ) {
            self.flash(format!("Could not open {url}: {e}"), cx);
        }
        cx.activate(true);
    }

    /// Once, on a first launch: offer to make deck the Mac's terminal.
    fn offer_default_terminal(&mut self, cx: &mut Context<Self>) {
        if self.settings.asked_default_terminal || crate::handler::is_default() {
            return;
        }
        self.notice = Some(Notice {
            title: "Make Deck your default terminal?".into(),
            detail: Some(
                "Shell scripts, .command files and ssh: links will open in Deck. You can change this in Settings › General."
                    .into(),
            ),
            buttons: vec![
                (
                    "Make Default".into(),
                    Rc::new(|ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
                        ws.make_default_terminal(true, window, cx)
                    }),
                ),
                (
                    "Not Now".into(),
                    Rc::new(|ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
                        ws.mark_default_terminal_asked(window, cx)
                    }),
                ),
            ],
        });
        cx.notify();
    }

    fn mark_default_terminal_asked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.settings.clone();
        next.asked_default_terminal = true;
        self.apply_settings(next, window, cx);
    }

    /// Hand shell scripts and terminal links to deck (`on`) or back to
    /// Terminal.
    pub fn make_default_terminal(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let id = if on {
            crate::handler::BUNDLE_ID
        } else {
            crate::handler::TERMINAL_APP
        };
        let failed = crate::handler::set_default(id);
        self.mark_default_terminal_asked(window, cx);
        self.flash(
            match (on, failed) {
                (true, 0) => "Deck is now your default terminal.".to_string(),
                (false, 0) => "Terminal opens scripts and links again.".to_string(),
                (_, n) => format!("macOS refused {n} of the changes; try again from Settings."),
            },
            cx,
        );
        for pane in self.panes.values() {
            if let PaneKind::Settings(s) = &pane.kind {
                s.update(cx, |s, cx| s.refresh_system(cx));
            }
        }
    }

    fn flash(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast = Some((text.into(), Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |ws, cx| {
                if ws
                    .toast
                    .as_ref()
                    .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(6))
                {
                    ws.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // -- tasks ----------------------------------------------------------------

    /// The task's app running from its own worktree, beside its review: the
    /// project's dev command in a terminal below it, on a port of its own so
    /// it does not fight the server in your main checkout, and the page
    /// opening beside that once something there listens. With no command
    /// deck knows, the terminal is there to start it by hand and the page
    /// still follows.
    fn preview_task(
        &mut self,
        review: PaneId,
        task: &crate::tasks::Task,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let port = crate::tasks::free_port();
        let cmd = crate::tasks::dev_command(&task.worktree).map(|c| match port {
            Some(p) => format!("export PORT={p}; {c}"),
            None => c,
        });
        let known = cmd.is_some();
        let term = match self.split(
            review,
            Open::Terminal {
                cmd,
                cwd: Some(task.worktree.clone()),
            },
            Axis::Column,
            window,
            cx,
        ) {
            Ok(id) => id,
            Err(e) => {
                self.flash(format!("Could not start the preview: {e}"), cx);
                return;
            }
        };
        self.flash(
            if known {
                "Starting the task's dev server; its page opens once it listens."
            } else {
                "No dev script found: start the server in this terminal and its page opens once it listens."
            },
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            // Installing can take a while; give it a few minutes.
            for _ in 0..360 {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let Ok(found) = this.update(cx, |ws, cx| {
                    let Some(PaneKind::Terminal(t)) = ws.panes.get(&term).map(|p| &p.kind) else {
                        return Err(());
                    };
                    let pids: Vec<u32> = inspect::descendants(t.read(cx).pid())
                        .into_iter()
                        .map(|p| p.pid)
                        .collect();
                    let ports: Vec<u16> = inspect::listening_ports(&pids)
                        .into_iter()
                        .map(|p| p.port)
                        .collect();
                    // The port it was given, else its lowest (a dev server's
                    // reload socket usually sits on a higher one).
                    Ok(port
                        .filter(|p| ports.contains(p))
                        .or_else(|| ports.iter().min().copied()))
                }) else {
                    return;
                };
                match found {
                    Err(()) => return, // the terminal was closed
                    Ok(None) => continue,
                    Ok(Some(p)) => {
                        let _ = this.update_in(cx, |ws, window, cx| {
                            let url = format!("http://localhost:{p}");
                            let _ = ws.split(term, Open::Browser { url }, Axis::Row, window, cx);
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// A task from a brief: its own branch and worktree off the project you
    /// are in, and a chat with the default agent working in it.
    fn start_task(&mut self, brief: String, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.focused_cwd(cx).unwrap_or_else(|| self.cwd.clone());
        self.spawn_task(brief, dir, None, None, None, window, cx);
    }

    /// Start a task in the repository `dir` is in. `agent` picks who works
    /// on it (the default when `None`), `parent` is the chat thread that
    /// asked for it, and `reply` answers a `deck task new` once it runs.
    #[allow(clippy::too_many_arguments)]
    fn spawn_task(
        &mut self,
        brief: String,
        dir: PathBuf,
        agent: Option<String>,
        parent: Option<String>,
        reply: Option<Reply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flash("Starting a task…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let made = cx
                .background_executor()
                .spawn({
                    let brief = brief.clone();
                    async move { crate::tasks::create(&dir, &brief) }
                })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| match made {
                Ok(mut task) => {
                    task.parent = parent;
                    let agent =
                        agent.or_else(|| ws.default_agent().map(|a| a.as_str().to_string()));
                    let opened = ws.new_tab(
                        Open::Chat {
                            agent,
                            cwd: Some(task.worktree.clone()),
                        },
                        None,
                        window,
                        cx,
                    );
                    match opened {
                        Ok(id) => {
                            if let Some(PaneKind::Chat(c)) = ws.panes.get(&id).map(|p| &p.kind) {
                                task.thread = Some(c.read(cx).thread_id().to_string());
                                let brief = task.brief.clone();
                                c.update(cx, |c, cx| c.send(brief, cx));
                            }
                        }
                        // No chat agent ready: Claude Code itself, in a terminal there.
                        Err(_) => {
                            let line = format!("claude {}", shell_quote(&task.brief));
                            let claude = AGENTS.iter().find(|a| a.cmd == "claude");
                            let _ = ws.new_tab(
                                Open::Terminal {
                                    cmd: Some(line),
                                    cwd: Some(task.worktree.clone()),
                                },
                                claude,
                                window,
                                cx,
                            );
                        }
                    }
                    ws.flash(format!("Working on {} in {}", task.title, task.branch), cx);
                    let answer = ws.task_json(&task, cx);
                    ws.tasks.push(task);
                    crate::tasks::save_all(&ws.tasks);
                    ws.refresh_task_changes(cx);
                    if let Some(r) = reply {
                        r.send(Ok(answer));
                    }
                }
                Err(e) => {
                    ws.flash(format!("Could not start the task: {e}"), cx);
                    if let Some(r) = reply {
                        r.send(Err(e));
                    }
                }
            });
        })
        .detach();
    }

    /// `deck task new BRIEF`: a task in the project the caller works in,
    /// told back to the caller's chat when it finishes.
    fn rpc_task_new(
        &mut self,
        p: &Value,
        caller: PaneId,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(brief) = str_param(p, "text").filter(|b| !b.trim().is_empty()) else {
            return reply.send(Err("say what the task is: deck task new 'brief'".into()));
        };
        let brief = brief.to_string();
        let dir = str_param(p, "cwd")
            .map(PathBuf::from)
            .or_else(|| self.cwd_of(caller, cx))
            .unwrap_or_else(|| self.cwd.clone());
        let agent = str_param(p, "agent").map(str::to_string);
        let parent = match self.panes.get(&caller).map(|p| &p.kind) {
            Some(PaneKind::Chat(c)) => Some(c.read(cx).thread_id().to_string()),
            _ => None,
        };
        self.spawn_task(brief, dir, agent, parent, Some(reply), window, cx);
    }

    /// Where a pane works: a terminal's current folder, a chat's.
    fn cwd_of(&self, id: PaneId, cx: &App) -> Option<PathBuf> {
        match &self.panes.get(&id)?.kind {
            PaneKind::Chat(c) => Some(c.read(cx).cwd().to_path_buf()),
            PaneKind::Terminal(t) => self.places.get(&id).map(|p| p.cwd.clone()).or_else(|| {
                let procs = inspect::descendants(t.read(cx).pid());
                procs.iter().rev().find_map(|p| inspect::cwd(p.pid))
            }),
            _ => None,
        }
    }

    /// The task a call names with `--task ID` (a unique prefix will do).
    fn task_param(&self, p: &Value) -> Result<crate::tasks::Task, String> {
        let id = str_param(p, "task").ok_or("missing --task ID (see deck task list)")?;
        let open = self
            .tasks
            .iter()
            .filter(|t| t.state == crate::tasks::State::Open);
        let hits: Vec<&crate::tasks::Task> = open
            .clone()
            .filter(|t| t.id == id)
            .chain(open.filter(|t| t.id != id && t.id.starts_with(id)))
            .collect();
        match hits.as_slice() {
            [one, ..] if one.id == id => Ok((*one).clone()),
            [one] => Ok((*one).clone()),
            [] => Err(format!("no open task {id:?}")),
            _ => Err(format!("{id:?} names more than one task")),
        }
    }

    fn task_json(&self, t: &crate::tasks::Task, cx: &App) -> Value {
        let pane = self.task_pane(t, cx);
        let status = match pane.and_then(|p| self.status(p, cx)) {
            Some(Status::Working) => "working",
            Some(Status::Attention) => "needs-you",
            Some(Status::Idle) => "idle",
            None if pane.is_some() => "idle",
            None => "closed",
        };
        let c = self.task_changes.get(&t.id).cloned().unwrap_or_default();
        json!({
            "task": t.id,
            "title": t.title,
            "branch": t.branch,
            "base": t.base,
            "worktree": t.worktree,
            "status": status,
            "pane": pane,
            "files": c.files,
            "added": c.added,
            "removed": c.removed,
        })
    }

    /// A task's agent finished a stretch of work: tell the chat that handed
    /// it out, which reads it as soon as it is free.
    fn report_to_parent(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        let Some(task) = self
            .tasks
            .iter()
            .find(|t| t.state == crate::tasks::State::Open && self.task_pane(t, cx) == Some(pane))
            .cloned()
        else {
            return;
        };
        let Some(parent) = task.parent.as_deref() else {
            return;
        };
        let Some(PaneKind::Chat(c)) = self
            .panes
            .values()
            .map(|p| &p.kind)
            .find(|k| matches!(k, PaneKind::Chat(c) if c.read(cx).thread_id() == parent))
        else {
            return;
        };
        let said = match self.panes.get(&pane).map(|p| &p.kind) {
            Some(PaneKind::Chat(w)) => w.read(cx).last_reply(),
            _ => None,
        };
        let ch = self.task_changes.get(&task.id).cloned().unwrap_or_default();
        let msg = format!(
            "[deck] Sub-task \"{}\" ({}) finished: {}. It changed {} file(s), +{} -{}. \
             Review with `deck task diff --task {id}`, follow up with `deck task send --task {id} '…'`, \
             then `deck task merge --task {id}` (or `pr` / `discard`).",
            task.title,
            task.id,
            said.unwrap_or_else(|| "no reply text".into()),
            ch.files,
            ch.added,
            ch.removed,
            id = task.id,
        );
        let c = c.clone();
        c.update(cx, |c, cx| c.send(msg, cx));
    }

    fn task(&self, id: &str) -> Option<&crate::tasks::Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// The pane doing a task's work: its chat, or a terminal in its worktree.
    fn task_pane(&self, task: &crate::tasks::Task, cx: &App) -> Option<PaneId> {
        self.panes.iter().find_map(|(id, p)| match &p.kind {
            PaneKind::Chat(c) if task.thread.as_deref() == Some(c.read(cx).thread_id()) => {
                Some(*id)
            }
            PaneKind::Terminal(_)
                if self
                    .places
                    .get(id)
                    .is_some_and(|pl| pl.cwd.starts_with(&task.worktree)) =>
            {
                Some(*id)
            }
            _ => None,
        })
    }

    /// Whether a tab is doing a task's work, so the sidebar lists it there.
    fn is_task_tab(&self, tab: &Tab, cx: &App) -> bool {
        tab.leaves().iter().any(|id| {
            self.tasks
                .iter()
                .filter(|t| t.state == crate::tasks::State::Open)
                .any(|t| self.task_pane(t, cx) == Some(*id))
        })
    }

    /// Count what every open task has changed, off the main thread.
    fn refresh_task_changes(&mut self, cx: &mut Context<Self>) {
        let open: Vec<crate::tasks::Task> = self
            .tasks
            .iter()
            .filter(|t| t.state == crate::tasks::State::Open && t.worktree.exists())
            .cloned()
            .collect();
        if open.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let counted = cx
                .background_executor()
                .spawn(async move {
                    open.into_iter()
                        .filter_map(|t| crate::tasks::changes(&t).ok().map(|c| (t.id, c)))
                        .collect::<HashMap<_, _>>()
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                if counted != ws.task_changes {
                    ws.task_changes = counted;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A task's review, beside its work if that is open.
    fn open_review(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.panes.iter().find_map(|(pid, p)| match &p.kind {
            PaneKind::Review(r) if r.read(cx).task.id == id => Some(*pid),
            _ => None,
        });
        if let Some(pid) = existing {
            if let PaneKind::Review(r) = &self.panes[&pid].kind {
                r.update(cx, |r, cx| r.reload(cx));
            }
            self.focus_pane(pid, window, cx);
            return;
        }
        let beside = self.task(id).and_then(|t| self.task_pane(t, cx));
        let made = match beside {
            Some(p) => self.split(p, Open::Review(id.to_string()), Axis::Row, window, cx),
            None => self.new_tab(Open::Review(id.to_string()), None, window, cx),
        };
        if let Err(e) = made {
            self.flash(format!("Could not open the review: {e}"), cx);
        }
    }

    fn make_review(
        &mut self,
        pane: PaneId,
        task: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<(PaneKind, Option<String>, Subscription)> {
        let task = self
            .task(task)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no task {task}"))?;
        let r = cx.new(|cx| ReviewPane::new(task, cx));
        let sub = cx.subscribe_in(&r, window, move |ws, r, ev: &ReviewEvent, window, cx| {
            let task = r.read(cx).task.clone();
            match ev {
                ReviewEvent::Terminal => {
                    let _ = ws.split(
                        pane,
                        Open::Terminal {
                            cmd: None,
                            cwd: Some(task.worktree.clone()),
                        },
                        Axis::Column,
                        window,
                        cx,
                    );
                }
                ReviewEvent::Preview => ws.preview_task(pane, &task, window, cx),
                ReviewEvent::Merge => ws.finish_task(pane, task, Finish::Merge, window, cx),
                ReviewEvent::Discard => ws.finish_task(pane, task, Finish::Discard, window, cx),
                ReviewEvent::OpenPr => ws.finish_task(pane, task, Finish::Pr, window, cx),
            }
        });
        Ok((PaneKind::Review(r), None, sub))
    }

    /// Merge, discard or open a PR for a task, off the main thread; then
    /// close what was working on it (or, for a PR, open the PR beside).
    fn finish_task(
        &mut self,
        review: PaneId,
        task: crate::tasks::Task,
        how: Finish,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_task_with(Some(review), task, how, None, window, cx);
    }

    fn finish_task_with(
        &mut self,
        review: Option<PaneId>,
        task: crate::tasks::Task,
        how: Finish,
        reply: Option<Reply>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(PaneKind::Review(r)) = review.and_then(|r| self.panes.get(&r)).map(|p| &p.kind)
        {
            r.update(cx, |r, cx| {
                r.set_working(
                    Some(match how {
                        Finish::Merge => "Merging…",
                        Finish::Discard => "Discarding…",
                        Finish::Pr => "Pushing and opening a pull request…",
                    }),
                    cx,
                )
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            let done = cx
                .background_executor()
                .spawn({
                    let task = task.clone();
                    async move {
                        match how {
                            Finish::Merge => crate::tasks::merge(&task).map(|_| None),
                            Finish::Discard => crate::tasks::discard(&task).map(|_| None),
                            Finish::Pr => crate::tasks::open_pr(&task).map(Some),
                        }
                    }
                })
                .await;
            let _ = this.update_in(cx, |ws, window, cx| {
                let review_pane =
                    review
                        .and_then(|r| ws.panes.get(&r))
                        .and_then(|p| match &p.kind {
                            PaneKind::Review(r) => Some(r.clone()),
                            _ => None,
                        });
                match done {
                    Err(e) => {
                        if let Some(r) = review_pane {
                            let e = e.clone();
                            r.update(cx, |r, cx| r.fail(e, cx));
                        }
                        if let Some(r) = reply {
                            r.send(Err(e));
                        }
                    }
                    Ok(Some(url)) => {
                        if let Some(r) = review_pane {
                            r.update(cx, |r, cx| r.set_working(None, cx));
                        }
                        let near = review.or_else(|| ws.task_pane(&task, cx));
                        match near {
                            Some(p) => {
                                ws.open_url_near(p, &url, window, cx);
                            }
                            None => ws.open_url_beside(&url, window, cx),
                        }
                        ws.flash("Pull request opened", cx);
                        if let Some(r) = reply {
                            r.send(Ok(json!({ "task": task.id, "pr": url })));
                        }
                    }
                    Ok(None) => {
                        let worker = ws.task_pane(&task, cx);
                        if let Some(t) = ws.tasks.iter_mut().find(|t| t.id == task.id) {
                            t.state = if how == Finish::Merge {
                                crate::tasks::State::Merged
                            } else {
                                crate::tasks::State::Discarded
                            };
                        }
                        crate::tasks::save_all(&ws.tasks);
                        if let Some(review) = review {
                            ws.close_pane(review, window, cx);
                        }
                        if let Some(w) = worker {
                            ws.close_pane(w, window, cx);
                        }
                        if let Some(r) = reply {
                            r.send(Ok(json!({
                                "task": task.id,
                                "state": if how == Finish::Merge { "merged" } else { "discarded" },
                            })));
                        }
                        ws.flash(
                            if how == Finish::Merge {
                                format!("Merged “{}” into {}", task.title, task.base)
                            } else {
                                format!("Discarded “{}”", task.title)
                            },
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    fn render_task_row(&self, task: &crate::tasks::Task, cx: &mut Context<Self>) -> AnyElement {
        let pane = self.task_pane(task, cx);
        let status = pane.and_then(|p| self.status(p, cx));
        let active = pane.is_some_and(|p| self.focused() == Some(p));
        let ch = self.task_changes.get(&task.id).cloned().unwrap_or_default();
        let working = status == Some(Status::Working);
        let wants = status == Some(Status::Attention);
        let line: SharedString = match pane
            .filter(|_| working || wants)
            .and_then(|p| self.agent_activity(p, cx))
        {
            Some(doing) => doing.into(),
            None if ch.files == 0 => "No changes yet".into(),
            None => format!(
                "{} file{} · +{} −{}",
                ch.files,
                if ch.files == 1 { "" } else { "s" },
                ch.added,
                ch.removed
            )
            .into(),
        };
        let id = task.id.clone();
        let review_id = task.id.clone();
        let key = SharedString::from(format!("task-{}", task.id));
        sidebar_row(div().id(ElementId::Name(key.clone())), active)
            .group(key.clone())
            .h(u(ROW_TWO_LINE))
            .child(lead(if working {
                thinking(
                    ElementId::Name(format!("task-think-{}", task.id).into()),
                    Icon::Git,
                    15.,
                    theme::text(),
                )
            } else {
                icon(
                    Icon::Git,
                    15.,
                    if active {
                        theme::text()
                    } else {
                        theme::text_2()
                    },
                )
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .w_full()
                            .one_line()
                            .text_size(u(13.))
                            .text_color(rgb(if active {
                                theme::text()
                            } else {
                                theme::text_2()
                            }))
                            .child(task.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .one_line()
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
                            .child(line),
                    ),
            )
            .child(if wants {
                alert_pill().into_any_element()
            } else if working {
                div()
                    .text_size(u(11.))
                    .child(shimmer(
                        ElementId::Name(format!("task-sh-{}", task.id).into()),
                        "Working".into(),
                    ))
                    .into_any_element()
            } else {
                div()
                    .id(ElementId::Name(format!("task-review-{}", task.id).into()))
                    .flex_none()
                    .flex()
                    .items_center()
                    .h(u(22.))
                    .px(u(8.))
                    .rounded(u(6.))
                    .bg(rgb(theme::surface()))
                    .text_size(u(11.))
                    .text_color(rgb(theme::text()))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::surface_hover())))
                    .child("Review")
                    .on_click(cx.listener(move |ws, _, window, cx| {
                        cx.stop_propagation();
                        ws.open_review(&review_id, window, cx);
                    }))
                    .into_any_element()
            })
            .on_click(cx.listener(move |ws, _, window, cx| {
                let pane = ws.task(&id).and_then(|t| ws.task_pane(t, cx));
                match pane {
                    Some(p) => ws.focus_pane(p, window, cx),
                    None => ws.open_review(&id, window, cx),
                }
            }))
            .into_any_element()
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
        let PaneKind::Terminal(t) = &pane.kind else {
            return None;
        };
        // Anything ringing wants you; past that, only agents have a state.
        if t.read(cx).bell {
            return Some(Status::Attention);
        }
        let agent = pane.agent.is_some() || self.places.get(&id).is_some_and(|p| p.agent.is_some());
        agent.then(|| {
            if self.busy.contains(&id) {
                Status::Working
            } else {
                Status::Idle
            }
        })
    }

    /// A pane wants you. If you are looking at it, that is answered already;
    /// otherwise its tab says so and, for an agent, macOS does too.
    fn alert(
        &mut self,
        id: PaneId,
        said: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PaneKind::Terminal(t)) = self.panes.get(&id).map(|p| &p.kind) else {
            return;
        };
        if window.is_window_active() && self.focused() == Some(id) {
            t.update(cx, |t, cx| t.seen(cx));
            return;
        }
        let agent = self
            .places
            .get(&id)
            .and_then(|p| p.agent.as_ref())
            .and_then(|(cmd, _)| AGENTS.iter().find(|a| a.cmd == *cmd))
            .or(self.panes.get(&id).and_then(|p| p.agent));
        // A shell's bell (a failed completion) marks the tab and no more.
        let Some(agent) = agent else {
            cx.notify();
            return;
        };
        if self.settings.notify_attention {
            let body = said.unwrap_or_else(|| format!("{} needs you", agent.name));
            self.post(id, body, cx);
        }
        cx.notify();
    }

    /// Claude Code in a terminal waits out API errors itself ("Retrying in
    /// 18s · attempt 4/10") and can sit there a long time, or get no answer
    /// at all. Deck presses Esc and, five seconds later, Enter (if Esc put
    /// the message back in the box) or "continue". One Esc per stall: a
    /// second would open Claude Code's rewind menu.
    fn unstick_agents(&mut self, cx: &mut Context<Self>) {
        if !crate::retry::enabled() {
            return;
        }
        let now = Instant::now();
        let claude: Vec<(PaneId, Entity<TerminalPane>)> = self
            .panes
            .iter()
            .filter(|(id, _)| {
                matches!(
                    self.places.get(id).and_then(|p| p.agent.as_ref()),
                    Some(("claude", _))
                )
            })
            .filter_map(|(id, p)| match &p.kind {
                PaneKind::Terminal(t) => Some((*id, t.clone())),
                _ => None,
            })
            .collect();
        for (id, t) in claude {
            let screen = t.update(cx, |t, _| t.read_text());
            if crate::retry::stall_line(&screen).is_none() {
                // A quiet minute ends a streak.
                if self
                    .stalls
                    .get(&id)
                    .is_some_and(|(_, at)| now - *at > Duration::from_secs(60))
                {
                    self.stalls.remove(&id);
                }
                continue;
            }
            let (n, last) = self
                .stalls
                .get(&id)
                .copied()
                .unwrap_or((0, now - Duration::from_secs(3600)));
            if now - last < Duration::from_secs(30) || n >= crate::retry::MAX_TRIES {
                continue;
            }
            self.stalls.insert(id, (n + 1, now));
            t.update(cx, |t, cx| t.send("\x1b", false, cx));
            let weak = t.downgrade();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let _ = weak.update(cx, |t, cx| {
                    if crate::retry::input_has_text(&t.read_text()) {
                        t.send("", true, cx);
                    } else {
                        t.send("continue", true, cx);
                    }
                });
            })
            .detach();
            tracing::info!(
                pane = id,
                try_ = n + 1,
                "unstuck Claude Code from an API wait"
            );
        }
    }

    /// "Claude finished: <the opening of its reply>", or of its task.
    fn finished_line(&self, id: PaneId, cx: &App) -> String {
        match self.panes.get(&id).map(|p| &p.kind) {
            Some(PaneKind::Chat(c)) => {
                let c = c.read(cx);
                match c.last_reply() {
                    Some(r) => format!("{} finished: {r}", c.agent_name()),
                    None => format!("{} finished", c.agent_name()),
                }
            }
            _ => {
                let who = self
                    .places
                    .get(&id)
                    .and_then(|p| p.agent.as_ref())
                    .and_then(|(cmd, _)| AGENTS.iter().find(|a| a.cmd == *cmd))
                    .or(self.panes.get(&id).and_then(|p| p.agent))
                    .map_or("The agent", |a| a.name);
                format!("{who} finished")
            }
        }
    }

    /// What an agent is up to, for the second line of its row: the step it
    /// is on, or what it waits on you for.
    fn agent_activity(&self, id: PaneId, cx: &App) -> Option<String> {
        match &self.panes.get(&id)?.kind {
            PaneKind::Chat(c) => {
                let c = c.read(cx);
                c.activity().or_else(|| c.waiting_for())
            }
            PaneKind::Terminal(t) => t.read(cx).alert.clone(),
            _ => None,
        }
    }

    /// One notification per pane at a time: an agent that rings and posts
    /// for the same prompt shows up once.
    fn post(&mut self, id: PaneId, body: String, cx: &App) {
        let now = Instant::now();
        if self
            .alerted
            .get(&id)
            .is_some_and(|(at, said)| now - *at < Duration::from_secs(5) && *said == body)
        {
            return;
        }
        self.alerted.insert(id, (now, body.clone()));
        let title = self.pane_title(id, cx);
        crate::notify::post(id, &title, &body);
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
                kind: PaneKind::Review(r),
                ..
            }) => format!("Review: {}", r.read(cx).task.title).into(),
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
            "settings.open" => {
                use crate::settings_pane::Page;
                let page = match str_param(p, "page").unwrap_or("general") {
                    "general" | "account" | "updates" => Ok(Page::General),
                    "appearance" | "theme" => Ok(Page::Appearance),
                    "terminal" => Ok(Page::Terminal),
                    "browser" => Ok(Page::Browser),
                    "agents" => Ok(Page::Agents),
                    "keyboard" | "shortcuts" | "keys" => Ok(Page::Keyboard),
                    "snippets" => Ok(Page::Snippets),
                    other => Err(format!("no settings page {other}")),
                };
                page.map(|page| {
                    self.open_settings_at(page, window, cx);
                    json!({ "page": format!("{page:?}").to_lowercase() })
                })
            }
            "window.shot" => {
                let out = str_param(p, "out")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| control::socket_dir().join("shot.png"));
                crate::shot::capture(window, &out)
                    .map(|(w, h)| json!({ "path": out, "width": w, "height": h }))
            }
            "task.new" => return self.rpc_task_new(p, caller, reply, window, cx),
            "task.list" => Ok(json!({
                "tasks": self
                    .tasks
                    .iter()
                    .filter(|t| t.state == crate::tasks::State::Open)
                    .map(|t| self.task_json(t, cx))
                    .collect::<Vec<_>>()
            })),
            "task.show" => self.task_param(p).map(|t| {
                let mut v = self.task_json(&t, cx);
                if let Some(PaneKind::Chat(c)) = self
                    .task_pane(&t, cx)
                    .and_then(|id| self.panes.get(&id))
                    .map(|p| &p.kind)
                {
                    v["last_reply"] = json!(c.read(cx).last_reply());
                    v["waiting_for"] = json!(c.read(cx).waiting_for());
                }
                v
            }),
            "task.diff" => match self.task_param(p) {
                Ok(t) => {
                    cx.background_executor()
                        .spawn(async move {
                            reply.send(
                                crate::tasks::diff(&t).map(|d| json!({ "task": t.id, "diff": d })),
                            )
                        })
                        .detach();
                    return;
                }
                Err(e) => Err(e),
            },
            "task.send" => self.task_param(p).and_then(|t| {
                let text = str_param(p, "text")
                    .ok_or("missing the message")?
                    .to_string();
                let pane = self
                    .task_pane(&t, cx)
                    .ok_or("the task's agent is not open; open it from the sidebar")?;
                match &self.panes[&pane].kind {
                    PaneKind::Chat(c) => {
                        c.update(cx, |c, cx| c.send(text, cx));
                        Ok(json!({ "task": t.id, "pane": pane }))
                    }
                    PaneKind::Terminal(term) => {
                        term.update(cx, |term, cx| term.send(&text, true, cx));
                        Ok(json!({ "task": t.id, "pane": pane }))
                    }
                    _ => Err("the task's agent cannot take messages".into()),
                }
            }),
            "task.merge" | "task.discard" | "task.pr" => match self.task_param(p) {
                Ok(t) => {
                    let how = match req.method.as_str() {
                        "task.merge" => Finish::Merge,
                        "task.discard" => Finish::Discard,
                        _ => Finish::Pr,
                    };
                    let review = self.panes.iter().find_map(|(id, pane)| match &pane.kind {
                        PaneKind::Review(r) if r.read(cx).task.id == t.id => Some(*id),
                        _ => None,
                    });
                    return self.finish_task_with(review, t, how, Some(reply), window, cx);
                }
                Err(e) => Err(e),
            },
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
                    PaneKind::Review(r) => json!({ "kind": "review", "task": r.read(cx).task.id }),
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
            PaneKind::Review(_) => return Err(format!("pane {id} is a task review")),
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
            PaneKind::Review(_) => reply.send(Err(format!("pane {id} is a task review"))),
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
            Node::Split(axis, children, sizes) => {
                // Each pane is its own card, a gutter apart, so the split
                // reads as separate surfaces rather than ruled cells. The
                // gutter is the divider you drag.
                let mut d = div().flex().size_full();
                d = match axis {
                    Axis::Row => d.flex_row(),
                    Axis::Column => d.flex_col(),
                };
                for (i, c) in children.iter().enumerate() {
                    if i > 0 {
                        d = d.child(self.render_divider(
                            *axis,
                            children[i - 1].first(),
                            c.first(),
                            cx,
                        ));
                    }
                    // Where each cell lands, so a drag knows the pixels it
                    // is sharing out.
                    let key = c.first();
                    let cells = self.cells.clone();
                    let mut cell = div()
                        .relative()
                        .flex()
                        .flex_basis(u(0.))
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.render_node(c, multi, cx))
                        .child(
                            canvas(
                                move |bounds, _, _| {
                                    cells.borrow_mut().insert(key, bounds);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        );
                    cell.style().flex_grow = Some(sizes.get(i).copied().unwrap_or(1.));
                    cell.style().flex_shrink = Some(1.);
                    d = d.child(cell);
                }
                d.into_any_element()
            }
        }
    }

    /// The gutter between two panes of a split, as a handle: a short line
    /// shows on hover, a drag moves the boundary, a double-click evens the
    /// split out again.
    fn render_divider(
        &self,
        axis: Axis,
        a: PaneId,
        b: PaneId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = axis == Axis::Row;
        let active = self.resizing.as_ref().is_some_and(|r| r.a == a && r.b == b);
        let group = SharedString::from(format!("divider-{a}-{b}"));
        div()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .when(row, |d| d.w(u(theme::GUTTER)).h_full().cursor_col_resize())
            .when(!row, |d| d.h(u(theme::GUTTER)).w_full().cursor_row_resize())
            .child(
                div()
                    .rounded(u(1.))
                    .bg(rgba(theme::focus_edge()))
                    .when(row, |d| d.w(u(2.)).h(u(32.)))
                    .when(!row, |d| d.h(u(2.)).w(u(32.)))
                    .when(!active, |d| {
                        d.opacity(0.).group_hover(group, |s| s.opacity(1.))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |ws, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let Some(tab) = ws.tabs.get_mut(ws.active) else {
                        return;
                    };
                    if ev.click_count == 2 {
                        tab.root.even(a, b);
                        cx.notify();
                        return;
                    }
                    let Some((wa, wb)) = tab.root.weights(a, b) else {
                        return;
                    };
                    let cells = ws.cells.borrow();
                    let (Some(ba), Some(bb)) = (cells.get(&a), cells.get(&b)) else {
                        return;
                    };
                    let span = if row {
                        ba.size.width + bb.size.width
                    } else {
                        ba.size.height + bb.size.height
                    };
                    let start = if row { ev.position.x } else { ev.position.y };
                    let r = Resize {
                        row,
                        a,
                        b,
                        start: f32::from(start),
                        wa,
                        wb,
                        span: f32::from(span),
                    };
                    drop(cells);
                    ws.resizing = Some(r);
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    /// The pointer moved while a divider is held: share the two panes'
    /// pixels out again, keeping each at least a usable width.
    fn drag_divider(&mut self, ev: &gpui::MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = &self.resizing else {
            return;
        };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            cx.notify();
            return;
        }
        let at = f32::from(if r.row { ev.position.x } else { ev.position.y });
        let total = r.wa + r.wb;
        let least = (r.span * 0.15).min(160.);
        let a_px = (r.span * r.wa / total + at - r.start).clamp(least, r.span - least);
        let wa = total * a_px / r.span;
        let (a, b) = (r.a, r.b);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.root.resize(a, b, wa, total - wa) {
                cx.notify();
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
            PaneKind::Review(r) => r.clone().into(),
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
            .bg(rgb(theme::pane()))
            // In a split, the focused card has the brighter edge and the
            // others step back a little: where keys go, without a stripe.
            .when(multi, |d| {
                d.rounded(u(theme::CARD_RADIUS))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgba(if focused {
                        theme::focus_edge()
                    } else {
                        theme::hairline()
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
                        .bg(rgba(theme::veil())),
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
            Some(Pane { agent: Some(a), .. }) => (Icon::agent(a.cmd), theme::text()),
            Some(Pane {
                kind: PaneKind::Chat(c),
                ..
            }) => (Icon::agent(c.read(cx).agent.as_str()), theme::text()),
            Some(Pane {
                kind: PaneKind::Browser(_),
                ..
            }) => (Icon::Globe, theme::text()),
            Some(Pane {
                kind: PaneKind::Settings(_),
                ..
            }) => (Icon::Settings, theme::text()),
            Some(Pane {
                kind: PaneKind::Launcher(_),
                ..
            }) => (Icon::Plus, theme::text_2()),
            Some(Pane {
                kind: PaneKind::Review(_),
                ..
            }) => (Icon::Git, theme::text()),
            _ => (Icon::Terminal, theme::text_2()),
        }
    }

    /// The sidebar is an inbox, as in T3 Code: the control chat first, then
    /// threads with what each one needs from you, then the terminals and
    /// pages they opened. Empty sections are not drawn.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut threads: Vec<(usize, Instant)> = Vec::new();
        let mut workspace: Vec<usize> = Vec::new();
        let settings = self.settings_tab();
        for (ix, tab) in self.tabs.iter().enumerate() {
            // The control chat has its own row; a task's work, its task's;
            // Settings, the footer.
            if tab.pinned || self.is_task_tab(tab, cx) || Some(ix) == settings {
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
                .pr(u(ROW_END))
                .text_size(u(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::dim()))
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
                    .gap(u(ROW_GAP))
                    .h(u(34.))
                    .pl(u(theme::ROW_PAD - 1.))
                    .pr(u(ROW_END + 4.))
                    .mb(u(6.))
                    .rounded(u(10.))
                    .bg(rgb(theme::raised()))
                    .border_1()
                    .border_color(rgba(theme::hairline()))
                    .text_size(u(13.))
                    .text_color(rgb(theme::text()))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::surface())))
                    .child(lead(icon(Icon::Plus, 14., theme::text())))
                    .child(div().flex_1().child("New chat"))
                    .child(
                        div()
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
                            .child(hint("new-chat", &self.settings.keys)),
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

        let open: Vec<crate::tasks::Task> = self
            .tasks
            .iter()
            .filter(|t| t.state == crate::tasks::State::Open)
            .cloned()
            .collect();
        if !open.is_empty() {
            list = list.child(header("Tasks"));
            for t in &open {
                list = list.child(self.render_task_row(t, cx));
            }
        }

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
                .size(u(22.))
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
                                .size(u(22.))
                                .on_click(cx.listener(|ws, _, w, cx| {
                                    let what = ws.shell_here(cx);
                                    let _ = ws.new_tab(what, None, w, cx);
                                })),
                        )
                        .child(icon_button("ws-web", Icon::Globe).size(u(22.)).on_click(
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
                    .child(self.render_settings_row(settings, cx)),
            )
            .into_any_element()
    }

    /// Settings, at the foot of the sidebar: opens it, and while it is open
    /// is its row, selected like any other and closed the same way.
    fn render_settings_row(&self, tab: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        let active = tab == Some(self.active);
        let tone = if active {
            theme::text()
        } else {
            theme::text_2()
        };
        sidebar_row(div().id("open-settings"), active)
            .group("settings-row")
            .h(u(ROW_ONE_LINE))
            .text_size(u(13.))
            .text_color(rgb(tone))
            .child(lead(icon(Icon::Settings, 15., tone)))
            .child(div().flex_1().child("Settings"))
            .child(
                // The shortcut, swapped for close while hovered once open.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .relative()
                    .min_w(u(22.))
                    .h(u(22.))
                    .child(
                        div()
                            .when(tab.is_some(), |d| {
                                d.group_hover("settings-row", |s| s.opacity(0.))
                            })
                            .pr(u(4.))
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
                            .child(hint("settings", &self.settings.keys)),
                    )
                    .when_some(tab, |d, ix| {
                        d.child(
                            icon_button("close-settings", Icon::Close)
                                .absolute()
                                .right_0()
                                .size(u(22.))
                                .opacity(0.)
                                .group_hover("settings-row", |s| s.opacity(1.))
                                .on_click(cx.listener(move |ws, _, window, cx| {
                                    cx.stop_propagation();
                                    ws.close_tab(ix, window, cx);
                                })),
                        )
                    }),
            )
            .on_click(cx.listener(|ws, _, window, cx| ws.open_settings(window, cx)))
            .when_some(tab, |d, ix| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |ws, ev: &gpui::MouseDownEvent, window, cx| {
                        ws.open_tab_menu(ix, ev.position, window, cx)
                    }),
                )
            })
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
        let wants = pane.and_then(|id| self.status(id, cx)) == Some(Status::Attention);
        // Closed: the row stays, and a click starts a fresh conversation.
        let subtitle: SharedString = if ix.is_none() && self.probed {
            "Closed · click to start a new conversation".into()
        } else {
            subtitle
        };
        sidebar_row(div().id("control"), active)
            .group("control-row")
            .h(u(ROW_TWO_LINE))
            .child(lead(if working {
                thinking("control-think", glyph, 15., theme::text())
            } else {
                icon(glyph, 15., theme::text())
            }))
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
                            .text_color(rgb(theme::text()))
                            .child("Control"),
                    )
                    .child(
                        div()
                            .w_full()
                            .one_line()
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
                            .child(subtitle),
                    ),
            )
            .when_some(state.filter(|(_, loud)| *loud), |d, (label, _)| {
                d.child(if working {
                    div()
                        .text_size(u(11.))
                        .child(shimmer("control-shimmer", label))
                        .into_any_element()
                } else if wants {
                    alert_pill().into_any_element()
                } else {
                    pill(label).into_any_element()
                })
            })
            // Close, as on every other row, while hovered.
            .when(ix.is_some(), |d| {
                d.child(
                    icon_button("close-control", Icon::Close)
                        .size(u(22.))
                        .opacity(0.)
                        .group_hover("control-row", |s| s.opacity(1.))
                        .on_click(cx.listener(|ws, _, window, cx| {
                            cx.stop_propagation();
                            if let Some(ix) = ws.control_tab() {
                                ws.close_tab(ix, window, cx);
                            }
                        })),
                )
            })
            .on_click(
                cx.listener(move |ws, _, window, cx| match ws.control_tab() {
                    Some(i) => ws.activate(i, window, cx),
                    None => {
                        ws.open_control(Vec::new(), window, cx);
                        if let Some(i) = ws.control_tab() {
                            ws.activate(i, window, cx);
                        }
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|ws, ev: &gpui::MouseDownEvent, window, cx| {
                    ws.open_control_menu(ev.position, window, cx)
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
        let wants = self.status(first, cx) == Some(Status::Attention);
        // While it works or waits, the second line says on what.
        let who: SharedString = match (working || wants)
            .then(|| self.agent_activity(first, cx))
            .flatten()
        {
            Some(doing) => doing.into(),
            None => who,
        };
        let (state, loud) = self.thread_state(first, cx);
        let group = SharedString::from(format!("tab-{ix}"));
        sidebar_row(
            div().id(ElementId::NamedInteger("tab".into(), ix as u64)),
            active,
        )
        .group(group.clone())
        .h(u(ROW_TWO_LINE))
        .child(lead(if working {
            thinking(
                ElementId::NamedInteger("thread-think".into(), ix as u64),
                glyph,
                15.,
                theme::text(),
            )
        } else {
            icon(
                glyph,
                15.,
                if active || loud {
                    theme::text()
                } else {
                    theme::text_2()
                },
            )
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .w_full()
                        .one_line()
                        .text_size(u(13.))
                        .text_color(rgb(if active || loud {
                            theme::text()
                        } else {
                            theme::text_2()
                        }))
                        .child(self.pane_title(tab.focused, cx)),
                )
                .child(
                    div()
                        .w_full()
                        .one_line()
                        .text_size(u(11.))
                        .text_color(rgb(theme::dim()))
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
                        } else if wants {
                            alert_pill().into_any_element()
                        } else if loud {
                            pill(state).into_any_element()
                        } else {
                            div()
                                .text_size(u(11.))
                                .text_color(rgb(theme::dim()))
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
            .bg(rgb(theme::pane()))
            .border_1()
            .border_color(rgb(theme::border()));
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
                            .text_color(rgb(theme::text()))
                            .hover(|s| s.bg(rgb(theme::surface())))
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
                    .when(!installed, |d| d.text_color(rgb(theme::dim())))
                    .child(icon(
                        Icon::agent(agent.cmd),
                        15.,
                        if installed {
                            theme::text()
                        } else {
                            theme::dim()
                        },
                    ))
                    .child(div().flex_1().child(agent.name))
                    .child(
                        div()
                            .text_size(u(11.))
                            .text_color(rgb(theme::dim()))
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
        let tone = if active { tint } else { theme::text_2() };
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
        // An agent typed into a shell is shown as that agent, like a thread.
        let agent = self
            .places
            .get(&first)
            .and_then(|p| p.agent.as_ref())
            .map(|(cmd, _)| Icon::agent(cmd));
        // Activity always shows in the icon's place: the agent's mark turns,
        // a busy command or a loading page spins.
        let mark = match (loading, busy, agent, self.tab_favicon(first, cx)) {
            (true, ..) => spinner(
                ElementId::NamedInteger("tab-load".into(), ix as u64),
                14.,
                theme::text_2(),
            ),
            (_, true, Some(a), _) => thinking(
                ElementId::NamedInteger("tab-think".into(), ix as u64),
                a,
                15.,
                theme::text(),
            ),
            (_, true, None, _) if !is_page => spinner(
                ElementId::NamedInteger("tab-busy".into(), ix as u64),
                14.,
                theme::text_2(),
            ),
            (_, _, Some(a), _) => icon(a, 15., tone),
            (_, _, _, Some(image)) => crate::browser::site_image(image, 15.),
            _ => icon(glyph, 15., tone),
        };

        sidebar_row(
            div().id(ElementId::NamedInteger("tab".into(), ix as u64)),
            active,
        )
        .group(group.clone())
        .h(u(if subtitle.is_some() {
            ROW_TWO_LINE
        } else {
            ROW_ONE_LINE
        }))
        .text_size(u(13.))
        .text_color(rgb(if active {
            theme::text()
        } else {
            theme::text_2()
        }))
        .child(lead(mark))
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
                        .one_line()
                        .child(self.pane_title(tab.focused, cx)),
                )
                .children(subtitle.map(|sub| {
                    div()
                        .w_full()
                        .one_line()
                        .text_size(u(11.))
                        .text_color(rgb(theme::dim()))
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
                    .text_color(rgb(theme::dim()))
                    .child(icon(Icon::Split, 11., theme::dim()))
                    .child(format!("{count}")),
            )
        })
        // Something wants you: the bell. Idle needs no mark.
        .when(status == Some(Status::Attention), |d| d.child(alert_pill()))
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
                .text_color(rgb(theme::dim()))
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
            .text_color(rgb(theme::text_2()))
            .child(
                div()
                    .pb(u(8.))
                    .text_size(u(15.))
                    .text_color(rgb(theme::text()))
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
                    .bg(rgb(theme::surface()))
                    .text_size(u(13.))
                    .text_color(rgb(theme::text()))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::surface_hover())))
                    .child(icon(glyph, 15., theme::text_2()))
                    .child(label)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(u(12.))
                            .text_color(rgb(theme::dim()))
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
        .text_color(rgb(theme::text_2()))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::surface())).text_color(rgb(theme::text())))
        .child(icon(glyph, 15., theme::text_2()))
}

/// The content area never gets narrower than this to make room for chrome:
/// past it the inspector folds away (and comes back when there is room).
const MIN_CONTENT_PX: f32 = 520.;

#[derive(Clone, Copy, PartialEq)]
enum Finish {
    Merge,
    Discard,
    Pr,
}

/// A word for the shell, single-quoted.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A divider held down: the two panes either side (by the pane each
/// subtree starts with), where the drag began, their weights then, and the
/// pixels they share.
struct Resize {
    row: bool,
    a: PaneId,
    b: PaneId,
    start: f32,
    wa: f32,
    wb: f32,
    span: f32,
}

/// Sidebar rows share their edges: the leading mark sits in one slot at the
/// row inset, every title starts on the same column after it, and trailing
/// buttons end on one edge, headers' included.
const ROW_SLOT: f32 = 16.;
const ROW_GAP: f32 = 10.;
const ROW_END: f32 = 4.;
/// A row with a title and a line under it.
const ROW_TWO_LINE: f32 = 44.;
const ROW_ONE_LINE: f32 = 32.;

/// The leading mark of a sidebar row, centred in the shared slot, so icons,
/// favicons and spinners of different sizes line up.
fn lead(mark: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .w(u(ROW_SLOT))
        .child(mark)
        .into_any_element()
}

/// A sidebar row's frame, the same on every row. The open row is lifted onto
/// its own tone with a hairline edge (and a soft shadow on a light sidebar),
/// so it never reads as merely hovered. The edge is there at rest too, only
/// transparent, so picking a row moves nothing by a pixel.
fn sidebar_row(row: gpui::Stateful<gpui::Div>, active: bool) -> gpui::Stateful<gpui::Div> {
    row.flex()
        .items_center()
        .gap(u(ROW_GAP))
        // The edge takes a pixel; text stays on the section labels' line.
        .pl(u(theme::ROW_PAD - 1.))
        .pr(u(ROW_END - 1.))
        .rounded(u(theme::ROW_RADIUS))
        .border_1()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(rgb(theme::row_active()))
                .border_color(rgba(theme::hairline()))
                .when(!theme::is_dark(), |d| {
                    d.shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0., 0., 0., 0.06),
                        offset: gpui::point(px(0.), px(1.)),
                        blur_radius: px(2.),
                        spread_radius: px(0.),
                    }])
                })
        })
        .when(!active, |d| {
            d.border_color(gpui::transparent_black())
                .hover(|s| s.bg(rgb(theme::row_hover())))
        })
}

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
    cwd: PathBuf,
    project: String,
    /// The path inside the project, `""` at its root.
    sub: String,
    /// What a terminal is running in front ("zsh", "nvim", "cargo").
    front: Option<String>,
    /// An agent CLI in front, and the arguments it was started with.
    agent: Option<(&'static str, Vec<String>)>,
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

/// The agent CLI process `pid` (named `name`) is, with its arguments: by
/// its name, or for one run by a script interpreter (Gemini CLI under node)
/// by the script it runs. Arguments are read only when it could be one.
fn agent_of(pid: u32, name: &str) -> Option<(&'static Agent, Vec<String>)> {
    if let Some(a) = AGENTS.iter().find(|a| a.cmd == name) {
        return Some((a, inspect::argv(pid)));
    }
    if !matches!(name, "node" | "bun" | "deno" | "python3" | "python") {
        return None;
    }
    let argv = inspect::argv(pid);
    let script = argv.get(1)?.rsplit('/').next()?.to_string();
    AGENTS.iter().find(|a| a.cmd == script).map(|a| (a, argv))
}

fn is_shell(name: &str) -> bool {
    let name = name.trim_start_matches('-');
    matches!(name, "zsh" | "bash" | "sh" | "fish" | "dash" | "login")
}

fn place_of(cwd: &std::path::Path) -> Place {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.as_deref() == Some(cwd) {
        return Place {
            cwd: cwd.to_path_buf(),
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
            cwd: cwd.to_path_buf(),
            project: name(root),
            sub: cwd
                .strip_prefix(root)
                .map(|r| r.display().to_string())
                .unwrap_or_default(),
            ..Default::default()
        },
        None => Place {
            cwd: cwd.to_path_buf(),
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
        let screen = self.locked || !crate::account::allowed();
        if screen != self.screen_cover {
            self.screen_cover = screen;
            self.cover_pages(screen, cx);
        }
        if self.locked {
            window.set_rem_size(px(theme::REM_PX * self.settings.zoom));
            return self.lock_screen(cx);
        }
        if screen {
            window.set_rem_size(px(theme::REM_PX * self.settings.zoom));
            return self.sign_in_screen(cx);
        }
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
                        .one_line()
                        .text_size(u(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
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
                        d.bg(rgb(theme::surface())).text_color(rgb(theme::text()))
                    })
                    .on_click(cx.listener(|ws, _, w, cx| ws.toggle_inspector(w, cx))),
            )
            .children(self.update_pill(cx))
            .child(self.avatar_button(cx));

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
                            .border_color(rgba(theme::hairline()))
                            .bg(rgb(theme::pane()))
                    })
                    .child(content),
            );

        div()
            .relative()
            .size_full()
            .flex()
            .bg(rgb(theme::sidebar()))
            .font_family(theme::UI)
            .on_mouse_move(cx.listener(Self::drag_divider))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|ws, _, _, cx| {
                    if ws.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .text_color(rgb(theme::text()))
            .on_action(cx.listener(|ws, _: &CommandPalette, w, cx| ws.toggle_palette(false, w, cx)))
            .on_action(cx.listener(|ws, _: &LockDeck, _, cx| ws.lock(cx)))
            .on_action(cx.listener(|ws, _: &InsertSnippet, w, cx| ws.toggle_palette(true, w, cx)))
            .on_action(cx.listener(|ws, _: &SaveSnippet, w, cx| ws.save_snippet(w, cx)))
            .on_action(cx.listener(|ws, _: &EditSnippets, w, cx| ws.edit_snippets(w, cx)))
            .on_action(cx.listener(|ws, _: &NewControl, w, cx| ws.new_control(w, cx)))
            .on_action(cx.listener(|ws, _: &RestartControl, w, cx| ws.restart_control(w, cx)))
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
            // The palette floats over everything, a little above centre,
            // on a scrim that closes it when clicked.
            .children(self.palette.as_ref().map(|(p, ..)| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .justify_center()
                    .pt(gpui::relative(0.12))
                    .bg(rgba(if theme::is_dark() {
                        0x00000059
                    } else {
                        0x00000026
                    }))
                    // Nothing under the scrim gets the wheel or the hover,
                    // so scrolling moves the palette's list, not the pane.
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|ws, _, window, cx| ws.close_palette(window, cx)),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(p.clone()),
                    )
            }))
            .children(self.notice.as_ref().map(|n| self.render_notice(n, cx)))
            .children(self.toast.as_ref().map(|(text, _)| {
                div()
                    .absolute()
                    .bottom(u(20.))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .max_w(u(520.))
                            .px(u(14.))
                            .py(u(8.))
                            .rounded(u(10.))
                            .bg(rgb(theme::menu()))
                            .border_1()
                            .border_color(rgba(theme::alpha(theme::text(), 0x1f)))
                            .shadow_lg()
                            .text_size(u(12.))
                            .text_color(rgb(theme::text()))
                            .child(text.clone()),
                    )
            }))
            .into_any_element()
    }
}

/// Wear `theme`: deck's own colours, and the appearance native views take
/// (traffic lights, menus, scrollbars, web pages), so nothing is left in
/// the other mode. System leaves the appearance to macOS.
fn wear(theme: crate::theme::Theme, window: &Window) {
    use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
    let system_dark = matches!(
        window.appearance(),
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    );
    crate::theme::set(theme, system_dark);
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let named = match theme {
        crate::theme::Theme::System => None,
        // SAFETY: AppKit constants.
        _ if crate::theme::is_dark() => Some(unsafe { NSAppearanceNameDarkAqua }),
        _ => Some(unsafe { NSAppearanceNameAqua }),
    };
    let appearance = named.and_then(NSAppearance::appearanceNamed);
    app.setAppearance(appearance.as_deref());
}

/// The Dock icon's badge: how many panes want you, or none.
fn set_badge(n: usize) {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let label = (n > 0).then(|| objc2_foundation::NSString::from_str(&n.to_string()));
    app.dockTile().setBadgeLabel(label.as_deref());
}

/// "Needs you", with the bell: the one loud state, the same in every row.
fn alert_pill() -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(u(4.))
        .pl(u(6.))
        .pr(u(7.))
        .h(u(20.))
        .rounded(u(10.))
        .bg(rgb(theme::text()))
        .text_size(u(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::sidebar()))
        .child(icon(Icon::Bell, 11., theme::sidebar()))
        .child("Needs you")
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
        .bg(rgb(theme::surface_hover()))
        .text_size(u(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::text()))
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

/// Your snippets and the project's as palette rows.
fn snippet_items(root: Option<&std::path::Path>) -> Vec<Item> {
    crate::snippets::load_all(root)
        .into_iter()
        .filter(|s| !s.body.trim().is_empty())
        .map(|s| Item {
            kind: Kind::Snippet,
            detail: crate::snippets::detail(&s).into(),
            title: if s.name.trim().is_empty() {
                crate::snippets::preview(&s.body).into()
            } else {
                s.name.into()
            },
            icon: Icon::Snippet,
            hint: None,
            pick: crate::palette::Pick::Snippet(s.body),
        })
        .collect()
}

/// A question in the corner of the window, with its answers as buttons.
struct Notice {
    title: SharedString,
    detail: Option<SharedString>,
    /// The first is the main answer. Each closes the notice, then runs.
    buttons: Vec<(SharedString, NoticeAction)>,
}

type NoticeAction = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

impl Workspace {
    fn render_notice(&self, n: &Notice, cx: &mut Context<Self>) -> AnyElement {
        let mut buttons = div().flex().gap(u(8.)).pt(u(4.));
        for (ix, (label, action)) in n.buttons.iter().enumerate() {
            let action = action.clone();
            let primary = ix == 0;
            buttons = buttons.child(
                div()
                    .id(ElementId::NamedInteger("notice".into(), ix as u64))
                    .flex()
                    .items_center()
                    .h(u(26.))
                    .px(u(12.))
                    .rounded(u(7.))
                    .text_size(u(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(primary, |d| {
                        d.bg(rgb(theme::text()))
                            .text_color(rgb(theme::pane()))
                            .hover(|s| s.opacity(0.9))
                    })
                    .when(!primary, |d| {
                        d.border_1()
                            .border_color(rgba(theme::alpha(theme::text(), 0x26)))
                            .text_color(rgb(theme::text()))
                            .hover(|s| s.bg(rgb(theme::surface())))
                    })
                    .child(label.clone())
                    .on_click(cx.listener(move |ws, _, window, cx| {
                        ws.notice = None;
                        action(ws, window, cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .absolute()
            .bottom(u(18.))
            .right(u(18.))
            .w(u(340.))
            .flex()
            .flex_col()
            .gap(u(6.))
            .p(u(14.))
            .rounded(u(12.))
            .bg(rgb(theme::menu()))
            .border_1()
            .border_color(rgba(theme::alpha(theme::text(), 0x1f)))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .text_size(u(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::text()))
                    .child(n.title.clone()),
            )
            .children(n.detail.clone().map(|d| {
                div()
                    .text_size(u(12.))
                    .line_height(u(17.))
                    .text_color(rgb(theme::text_2()))
                    .child(d)
            }))
            .child(buttons)
            .into_any_element()
    }
}

// -- account, updates and the lock ------------------------------------------------------

/// A button on the sign-in and lock screens: filled for the main action.
fn screen_button(
    id: &'static str,
    label: impl Into<SharedString>,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(u(36.))
        .min_w(u(240.))
        .px(u(18.))
        .rounded(u(9.))
        .text_size(u(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(rgb(theme::text()))
                .text_color(rgb(theme::pane()))
                .hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.border_1()
                .border_color(rgba(theme::alpha(theme::text(), 0x26)))
                .text_color(rgb(theme::text()))
                .hover(|s| s.bg(rgb(theme::surface())))
        })
        .child(label.into())
}

impl Workspace {
    fn refresh_settings_panes(&self, cx: &mut Context<Self>) {
        for pane in self.panes.values() {
            if let PaneKind::Settings(s) = &pane.kind {
                s.update(cx, |_, cx| cx.notify());
            }
        }
        cx.notify();
    }

    // -- account --------------------------------------------------------------------

    fn account_action(&mut self, a: crate::settings_pane::AccountAction, cx: &mut Context<Self>) {
        use crate::settings_pane::AccountAction as A;
        match a {
            A::SignIn => self.sign_in(cx),
            A::SignOut => {
                cx.spawn(async move |this, cx| {
                    crate::account::sign_out().await;
                    let _ = this.update(cx, |ws, cx| {
                        ws.flash("Signed out.", cx);
                        ws.refresh_settings_panes(cx);
                    });
                })
                .detach();
            }
            A::Checkout => {
                if let Some(u) = crate::account::checkout_url() {
                    cx.open_url(&u);
                }
            }
            A::Billing => {
                if let Some(u) = crate::account::billing_url() {
                    cx.open_url(&u);
                }
            }
        }
    }

    /// Open the sign-in page in the browser; it comes back as a `deck://`
    /// link.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        match crate::account::sign_in_url() {
            Some(url) => {
                self.signing_in = true;
                self.account_note = None;
                cx.open_url(&url);
            }
            None => self.flash("This build of Deck has no account server.", cx),
        }
        cx.notify();
    }

    /// `deck://auth/callback?…` after signing in, or `deck://refresh` after
    /// paying: load the account again.
    fn deck_url(&mut self, url: &str, cx: &mut Context<Self>) {
        if url.starts_with("deck://refresh") {
            self.refresh_account(cx);
            return;
        }
        match crate::account::callback(url) {
            Some(Ok(code)) => {
                cx.spawn(async move |this, cx| {
                    let r = crate::account::exchange(&code).await;
                    let _ = this.update(cx, |ws, cx| {
                        ws.signing_in = false;
                        match r {
                            Ok(a) => {
                                ws.account_note = None;
                                ws.flash(format!("Signed in as {}.", a.display_name()), cx);
                            }
                            Err(e) => ws.account_note = Some(e.into()),
                        }
                        ws.refresh_settings_panes(cx);
                    });
                })
                .detach();
            }
            Some(Err(e)) => {
                self.signing_in = false;
                self.account_note = Some(e.into());
                cx.notify();
            }
            None => tracing::info!("nothing handles {url}"),
        }
    }

    fn refresh_account(&mut self, cx: &mut Context<Self>) {
        if !crate::account::required() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let r = crate::account::refresh().await;
            let _ = this.update(cx, |ws, cx| {
                ws.account_note = r.err().map(Into::into);
                ws.refresh_settings_panes(cx);
            });
        })
        .detach();
    }

    /// With an account server: check the account now and every six hours,
    /// so a lapsed plan or a session ended elsewhere is noticed.
    fn start_account(&mut self, cx: &mut Context<Self>) {
        if !crate::account::required() {
            return;
        }
        cx.spawn(async move |this, cx| loop {
            if this.update(cx, |ws, cx| ws.refresh_account(cx)).is_err() {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_secs(6 * 3600))
                .await;
        })
        .detach();
    }

    fn sign_in_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let account = crate::account::current();
        let note = self.account_note.clone();
        let mut col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(12.))
            .w(u(360.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(u(64.))
                    .mb(u(8.))
                    .rounded(u(16.))
                    .bg(rgb(theme::text()))
                    .text_size(u(30.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::pane()))
                    .child("D"),
            );
        let title = |t: &str| {
            div()
                .text_size(u(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::text()))
                .child(t.to_string())
        };
        let words = |t: String| {
            div()
                .text_size(u(13.))
                .line_height(u(19.))
                .text_center()
                .text_color(rgb(theme::text_2()))
                .child(t)
        };
        match account {
            Some(a) if !a.active || a.trial_days_left() == Some(0) => {
                let (head, why) = if a.trial_ends.is_some() {
                    (
                        "Your free trial has ended",
                        "Choose a plan to keep using Deck. Your chats, tabs and settings are all still here.".to_string(),
                    )
                } else {
                    (
                        "Choose a plan",
                        format!("{} has no active plan. Pick one to keep using Deck.", a.email),
                    )
                };
                col = col.child(title(head)).child(words(why))
                    .child(div().h(u(8.)))
                    .child(
                        screen_button("gate-checkout", "Choose a Plan", true).on_click(
                            cx.listener(|ws, _, _, cx| {
                                ws.account_action(crate::settings_pane::AccountAction::Checkout, cx)
                            }),
                        ),
                    )
                    .child(
                        screen_button("gate-refresh", "I've Paid, Check Again", false)
                            .on_click(cx.listener(|ws, _, _, cx| ws.refresh_account(cx))),
                    )
                    .child(
                        screen_button("gate-out", "Sign Out", false).on_click(cx.listener(
                            |ws, _, _, cx| {
                                ws.account_action(crate::settings_pane::AccountAction::SignOut, cx)
                            },
                        )),
                    );
            }
            Some(_) => {
                // Signed in and paid, but not checked for a week: offline.
                col = col
                    .child(title("Can't reach Deck"))
                    .child(words(
                        "Deck needs to check your plan once a week, and couldn't. Check your connection and try again."
                            .into(),
                    ))
                    .child(div().h(u(8.)))
                    .child(
                        screen_button("gate-retry", "Try Again", true)
                            .on_click(cx.listener(|ws, _, _, cx| ws.refresh_account(cx))),
                    );
            }
            None => {
                col = col
                    .child(title("Sign in to Deck"))
                    .child(words(
                        "Terminals, a browser and your coding agents in one window. Sign in to start."
                            .into(),
                    ))
                    .child(div().h(u(8.)))
                    .child(
                        screen_button(
                            "gate-sign-in",
                            if self.signing_in {
                                "Open the Sign-in Page Again"
                            } else {
                                "Continue with Google"
                            },
                            true,
                        )
                        .on_click(cx.listener(|ws, _, _, cx| ws.sign_in(cx))),
                    )
                    .child(
                        div()
                            .text_size(u(12.))
                            .text_color(rgb(theme::dim()))
                            .child(if self.signing_in {
                                "Waiting for your browser…"
                            } else {
                                "Opens your browser; you come back here when you're done."
                            }),
                    );
            }
        }
        col = col.children(note.map(|n| {
            div()
                .pt(u(6.))
                .text_size(u(12.))
                .text_color(rgb(theme::DANGER))
                .child(n)
        }));
        self.screen(col)
    }

    /// A full window with `content` in the middle, under a strip that still
    /// moves the window.
    fn screen(&self, content: gpui::Div) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::pane()))
            .font_family(theme::UI)
            .child(div().flex_none().h(px(theme::TITLEBAR_PX)).on_mouse_down(
                MouseButton::Left,
                |ev, window, _| {
                    if ev.click_count == 2 {
                        window.titlebar_double_click();
                    }
                },
            ))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pb(u(60.))
                    .child(content),
            )
            .into_any_element()
    }

    // -- lock -----------------------------------------------------------------------

    fn lock(&mut self, cx: &mut Context<Self>) {
        self.locked = true;
        self.menu = None;
        cx.notify();
    }

    fn unlock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if crate::lock::authenticate("unlock Deck").await {
                let _ = this.update(cx, |ws, cx| {
                    ws.locked = false;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn lock_screen(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(u(64.))
                    .mb(u(8.))
                    .rounded_full()
                    .bg(rgb(theme::surface()))
                    .child(icon(Icon::Lock, 28., theme::text())),
            )
            .child(
                div()
                    .text_size(u(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::text()))
                    .child("Deck is locked"),
            )
            .child(
                div()
                    .text_size(u(13.))
                    .text_color(rgb(theme::text_2()))
                    .child("Your terminals and agents keep running."),
            )
            .child(div().h(u(8.)))
            .child(
                screen_button("unlock", "Unlock", true)
                    .on_click(cx.listener(|ws, _, _, cx| ws.unlock(cx))),
            )
            .child(
                div()
                    .text_size(u(12.))
                    .text_color(rgb(theme::dim()))
                    .child("With Touch ID or your Mac's password"),
            );
        self.screen(col)
    }

    // -- the avatar menu --------------------------------------------------------

    fn avatar_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let account = crate::account::current();
        let disc = div()
            .flex()
            .items_center()
            .justify_center()
            .size(u(24.))
            .rounded_full();
        let disc = match &account {
            Some(a) => disc
                .bg(rgb(theme::accent()))
                .text_size(u(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(0xffffff))
                .child(a.initial()),
            None => disc
                .bg(rgb(theme::surface()))
                .child(icon(Icon::User, 13., theme::text_2())),
        };
        div()
            .id("account")
            .ml(u(6.))
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .child(disc)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|ws, ev: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    ws.open_account_menu(ev.position, window, cx);
                }),
            )
            .into_any_element()
    }

    fn open_account_menu(
        &mut self,
        mut at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::menu::{Entry, Item};
        let ws = cx.entity().downgrade();
        let account = crate::account::current();
        let mut entries: Vec<Entry> = Vec::new();
        let who = match &account {
            Some(a) => a.display_name().to_string(),
            None if crate::account::required() => "Sign In…".to_string(),
            None => "Account".to_string(),
        };
        let w = ws.clone();
        let signed_out = account.is_none() && crate::account::required();
        entries.push(
            Item::new(who, move |window, cx| {
                let _ = w.update(cx, |ws, cx| {
                    if signed_out {
                        ws.sign_in(cx);
                    } else {
                        ws.open_settings_at(crate::settings_pane::Page::General, window, cx);
                    }
                });
            })
            .icon(Icon::User)
            .into(),
        );
        if let Some(n) = account.as_ref().and_then(|a| a.trial_days_left()) {
            let left = match n {
                0 => "Free trial ended · Choose a Plan".to_string(),
                1 => "Free trial: last day · Choose a Plan".to_string(),
                n => format!("Free trial: {n} days left · Choose a Plan"),
            };
            let w = ws.clone();
            entries.push(
                Item::new(left, move |_, cx| {
                    let _ = w.update(cx, |ws, cx| {
                        ws.account_action(crate::settings_pane::AccountAction::Checkout, cx)
                    });
                })
                .into(),
            );
        }
        entries.push(Entry::Separator);
        entries.push(
            Item::new("Settings", |window, cx| {
                window.dispatch_action(Box::new(OpenSettings), cx)
            })
            .icon(Icon::Settings)
            .shortcut(hint("settings", &self.settings.keys))
            .into(),
        );
        let w = ws.clone();
        entries.push(
            Item::new("Theme", move |window, cx| {
                let _ = w.update(cx, |ws, cx| {
                    ws.open_settings_at(crate::settings_pane::Page::Appearance, window, cx)
                });
            })
            .icon(Icon::Theme)
            .into(),
        );
        entries.push(Entry::Separator);
        entries.push(
            Item::new("Lock", move |_, cx| {
                let _ = ws.update(cx, |ws, cx| ws.lock(cx));
            })
            .icon(Icon::Lock)
            .shortcut(hint("lock", &self.settings.keys))
            .danger()
            .into(),
        );
        // Under the avatar, its right edge on the window's.
        let width = window.viewport_size().width;
        at.x = (width - px(crate::menu::MIN_W * self.settings.zoom + 12.)).max(px(0.));
        at.y = px(theme::TITLEBAR_PX * self.settings.zoom);
        let restore = window.focused(cx);
        self.menu = Some(crate::menu::open(
            entries,
            at,
            restore,
            window,
            cx,
            |ws: &mut Self, _, _| ws.menu = None,
        ));
        cx.notify();
    }

    fn open_settings_at(
        &mut self,
        page: crate::settings_pane::Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
        if let Some(s) = self.panes.values().find_map(|p| match &p.kind {
            PaneKind::Settings(s) => Some(s.clone()),
            _ => None,
        }) {
            s.update(cx, |s, cx| s.show(page, cx));
        }
    }

    // -- updates --------------------------------------------------------------

    fn update_action(&mut self, a: crate::settings_pane::UpdateAction, cx: &mut Context<Self>) {
        use crate::settings_pane::UpdateAction as A;
        use crate::updates::State;
        match a {
            A::Check => self.check_updates(true, cx),
            A::Download => {
                let State::Available(r) = crate::updates::state() else {
                    return;
                };
                crate::updates::set(State::Downloading(r.clone()));
                self.refresh_settings_panes(cx);
                cx.spawn(async move |this, cx| {
                    let got = crate::updates::download(&r).await;
                    crate::updates::set(match got {
                        Ok(path) => State::Ready(r, path),
                        Err(e) => State::Failed(e),
                    });
                    let _ = this.update(cx, |ws, cx| ws.refresh_settings_panes(cx));
                })
                .detach();
            }
            A::Restart => {
                let State::Ready(_, path) = crate::updates::state() else {
                    return;
                };
                match crate::updates::install_on_quit(&path) {
                    Ok(()) => cx.quit(),
                    Err(e) => self.flash(format!("Could not update: {e}"), cx),
                }
            }
            A::Notes => match crate::updates::state() {
                State::Available(r) | State::Downloading(r) | State::Ready(r, _) => {
                    cx.open_url(&r.notes_url)
                }
                _ => {}
            },
        }
    }

    fn check_updates(&mut self, by_hand: bool, cx: &mut Context<Self>) {
        use crate::updates::State;
        if matches!(
            crate::updates::state(),
            State::Checking | State::Downloading(_) | State::Ready(..)
        ) {
            return;
        }
        crate::updates::set(State::Checking);
        self.refresh_settings_panes(cx);
        cx.spawn(async move |this, cx| {
            let r = crate::updates::check().await;
            crate::updates::set(match r {
                Ok(Some(rel)) => State::Available(rel),
                Ok(None) => State::UpToDate,
                // A failed check nobody asked for stays quiet.
                Err(e) if by_hand => State::Failed(e),
                Err(_) => State::Idle,
            });
            let _ = this.update(cx, |ws, cx| ws.refresh_settings_panes(cx));
        })
        .detach();
    }

    /// When Settings allows it: a check a minute after launch, then daily.
    fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut wait = Duration::from_secs(60);
            loop {
                cx.background_executor().timer(wait).await;
                wait = Duration::from_secs(24 * 3600);
                let alive = this.update(cx, |ws, cx| {
                    if ws.settings.check_updates {
                        ws.check_updates(false, cx);
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// In the title bar while there is news: an update to get, coming, or
    /// ready for a restart.
    fn update_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::settings_pane::UpdateAction as A;
        use crate::updates::State;
        let (label, action, busy): (&str, Option<A>, bool) = match crate::updates::state() {
            State::Available(_) => ("Update available", Some(A::Download), false),
            State::Downloading(_) => ("Downloading update", None, true),
            State::Ready(..) => ("Restart to update", Some(A::Restart), false),
            _ => return None,
        };
        Some(
            div()
                .id("update-pill")
                .flex()
                .items_center()
                .gap(u(6.))
                .h(u(24.))
                .ml(u(6.))
                .px(u(10.))
                .rounded(u(7.))
                .border_1()
                .border_color(rgba(theme::alpha(theme::text(), 0x26)))
                .text_size(u(12.))
                .text_color(rgb(theme::text()))
                .when(busy, |d| {
                    d.child(crate::icon::spinner("update-spin", 12., theme::text_2()))
                })
                .when(!busy, |d| {
                    d.child(icon(Icon::ArrowUp, 12., theme::text_2()))
                })
                .child(label)
                .when_some(action, |d, a| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::surface())))
                        .on_click(cx.listener(move |ws, _, _, cx| ws.update_action(a, cx)))
                })
                .into_any_element(),
        )
    }
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
            Node::Split(
                Axis::Row,
                vec![Node::Leaf(1), Node::Leaf(2), Node::Leaf(3)],
                vec![1., 0.5, 0.5]
            )
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
            Node::Split(Axis::Row, vec![Node::Leaf(1), Node::Leaf(2)], vec![1., 1.])
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
