//! The Settings tab (⌘,): every preference deck keeps, applied the moment it
//! changes. The pane edits its own copy and hands each change to the
//! workspace, which saves it and pushes it to every pane.

use crate::field::{Field, FieldEvent};
use crate::icon::{icon, Icon};
use crate::settings::{
    self, Access, Cursor, Scheme, Search, Settings, FONT_PX_RANGE, LINE_HEIGHT_RANGE, ZOOMS,
};
use crate::snippets::{self, Snippet};
use crate::theme::{self, u, OneLine};
use gpui::prelude::*;
use gpui::{
    div, ease_in_out, relative, rgb, rgba, Animation, AnimationExt, AnyElement, ClickEvent,
    Context, Div, ElementId, Entity, EventEmitter, FocusHandle, FontWeight, SharedString, Stateful,
    Subscription, Window,
};
use std::time::Duration;

pub enum SettingsEvent {
    Changed(Settings),
    /// Make deck the Mac's terminal (true) or hand it back to Terminal.
    DefaultTerminal(bool),
    Account(AccountAction),
    Update(UpdateAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountAction {
    SignIn,
    SignOut,
    /// Choose and pay for a plan.
    Checkout,
    /// Manage the plan you have.
    Billing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    Check,
    Download,
    /// Quit and swap in the downloaded release.
    Restart,
    Notes,
}

pub struct SettingsPane {
    pub settings: Settings,
    pub focus: FocusHandle,
    /// Monospace families worth offering that are installed here.
    fonts: Vec<SharedString>,
    /// Agents that can start a chat: registry id and name.
    agents: Vec<(SharedString, SharedString)>,
    /// The toggle that just flipped and how many flips so far, so only it
    /// slides (a fresh id restarts the animation) and the rest sit still.
    flip: Option<(&'static str, u64)>,
    /// The page the sidebar has open.
    page: Page,
    scroll: gpui::ScrollHandle,
    /// The saved snippets, read again whenever their page opens.
    snippets: Vec<Snippet>,
    /// The snippet open for editing.
    editing: Option<Editing>,
    /// Whether deck opens shell scripts and terminal links, as macOS said
    /// when the page last opened.
    default_terminal: bool,
    /// The sidebar's search: while it holds text, the page shows every
    /// setting that matches, from all pages.
    search: Entity<Field>,
    /// Pages folded shut in the sidebar.
    collapsed: std::collections::HashSet<Page>,
    /// The shortcut waiting for its new keys, and the hook that takes the
    /// next keystroke before any shortcut can act on it.
    recording: Option<(&'static str, Subscription)>,
    /// Why the last recorded keys were not taken.
    key_note: Option<(&'static str, SharedString)>,
    _search: Subscription,
}

/// A snippet being edited: every keystroke is saved.
struct Editing {
    /// Its place in `snippets`.
    ix: usize,
    name: Entity<Field>,
    keyword: Entity<Field>,
    body: Entity<Field>,
    /// Delete was clicked once; the second click removes it.
    confirm: bool,
    _subs: [Subscription; 3],
}

impl EventEmitter<SettingsEvent> for SettingsPane {}

/// Monospace faces people pick on purpose, in the order offered. Only the
/// installed ones are shown.
const FONTS: &[&str] = &[
    "Menlo",
    "SF Mono",
    "Monaco",
    "JetBrains Mono",
    "Fira Code",
    "Berkeley Mono",
    "Iosevka",
    "Cascadia Code",
    "Geist Mono",
    "Commit Mono",
    "IBM Plex Mono",
    "Source Code Pro",
    "Roboto Mono",
    "Hack",
    "Ubuntu Mono",
    "Andale Mono",
    "PT Mono",
    "Courier New",
];

/// The curated faces that are installed, then any Nerd Font Mono patch (a
/// terminal font by definition), then the current choice if it is neither,
/// so a family set by hand in the file still shows as selected.
fn monospace_families(installed: &[String], current: &str) -> Vec<SharedString> {
    let mut out: Vec<SharedString> = FONTS
        .iter()
        .filter(|f| installed.iter().any(|n| n == *f))
        .map(|f| SharedString::from(*f))
        .collect();
    let mut nerd: Vec<&String> = installed
        .iter()
        .filter(|n| n.ends_with("Nerd Font Mono"))
        .collect();
    nerd.sort();
    nerd.dedup();
    out.extend(nerd.into_iter().map(|n| SharedString::from(n.clone())));
    // Menlo ships with every Mac and is the default; never leave it out.
    if !out.iter().any(|f| f.as_ref() == theme::MONO) {
        out.insert(0, theme::MONO.into());
    }
    if !out.iter().any(|f| f.as_ref() == current) {
        out.push(SharedString::from(current.to_string()));
    }
    out
}

/// `1.4` → `1.5`, clamped and kept to one decimal so repeated steps never
/// drift into `1.4999999`.
fn step_tenth(v: f32, dir: f32, (lo, hi): (f32, f32)) -> f32 {
    (((v * 10.).round() + dir) / 10.).clamp(lo, hi)
}

impl SettingsPane {
    pub fn new(
        settings: Settings,
        agents: Vec<(SharedString, SharedString)>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let installed = window.text_system().all_font_names();
        // No frame of its own: the sidebar draws one box around the mark and
        // the text.
        let search = cx.new(|cx| {
            let mut f = Field::new("Search settings…", cx);
            f.bare = true;
            f
        });
        let _search = cx.subscribe(&search, |p: &mut Self, _, ev: &FieldEvent, cx| {
            if matches!(ev, FieldEvent::Changed) {
                p.scroll.set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                cx.notify();
            }
        });
        Self {
            fonts: monospace_families(&installed, &settings.font_family),
            settings,
            focus: cx.focus_handle(),
            agents,
            flip: None,
            page: Page::Appearance,
            scroll: gpui::ScrollHandle::new(),
            snippets: Vec::new(),
            editing: None,
            default_terminal: crate::handler::is_default(),
            search,
            collapsed: Default::default(),
            recording: None,
            key_note: None,
            _search,
        }
    }

    fn query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    /// Open `page` at its section `ix`.
    fn show_section(&mut self, page: Page, ix: usize, cx: &mut Context<Self>) {
        if self.page != page {
            self.show(page, cx);
        }
        self.search.update(cx, |f, cx| f.set_text("", cx));
        // The page title is the first child, so section `ix` is `ix + 1`.
        self.scroll.scroll_to_top_of_item(ix + 1);
        cx.notify();
    }

    fn account_section(&self, cx: &mut Context<Self>) -> Section {
        let act =
            |id: &'static str, label: &'static str, a: AccountAction, cx: &mut Context<Self>| {
                button(id, label)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::Account(a))))
                    .into_any_element()
            };
        if !crate::account::required() {
            return group(
                "Account",
                vec![row("No account needed", div().into_any_element()).about(
                    "This build of Deck was made without an account server, so it runs without signing in.",
                )],
                None,
            );
        }
        match crate::account::current() {
            Some(a) => {
                let plan = match (a.trial_days_left(), &a.plan, a.active) {
                    (Some(0), ..) => "Your free trial has ended".to_string(),
                    (Some(1), ..) => "Free trial, last day".to_string(),
                    (Some(n), ..) => format!("Free trial, {n} days left"),
                    (None, Some(p), true) => format!("{p}, active"),
                    (None, Some(p), false) => format!("{p}, not active"),
                    (None, None, _) => "No plan yet".to_string(),
                };
                let manage = if a.active && a.trial_ends.is_none() {
                    act("billing", "Manage Plan", AccountAction::Billing, cx)
                } else {
                    act("checkout", "Choose a Plan", AccountAction::Checkout, cx)
                };
                group(
                    "Account",
                    vec![
                        row(a.display_name().to_string(), avatar(&a, 32.)).about(a.email.clone()),
                        row("Plan", manage).about(plan),
                        row("Sign out", act("sign-out", "Sign Out", AccountAction::SignOut, cx))
                            .about("Signs this Mac out of your Deck account."),
                    ],
                    None,
                )
            }
            None => group(
                "Account",
                vec![row(
                    "Sign in to Deck",
                    act("sign-in", "Sign In", AccountAction::SignIn, cx),
                )
                .about("Your account holds your plan. Signing in opens your browser, where you continue with Google.")],
                None,
            ),
        }
    }

    fn updates_section(&self, cx: &mut Context<Self>) -> Section {
        use crate::updates::{State, VERSION};
        let act =
            |id: &'static str, label: &'static str, a: UpdateAction, cx: &mut Context<Self>| {
                button(id, label)
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::Update(a))))
                    .into_any_element()
            };
        let (about, control): (String, AnyElement) = match crate::updates::state() {
            State::Idle => (
                format!("Deck {VERSION}"),
                act("update-check", "Check for Updates", UpdateAction::Check, cx),
            ),
            State::Checking => ("Checking…".into(), div().into_any_element()),
            State::UpToDate => (
                format!("Deck {VERSION} is the newest."),
                act("update-check", "Check Again", UpdateAction::Check, cx),
            ),
            State::Available(r) => (
                format!("Deck {} is out; you have {VERSION}.", r.version),
                if r.download.is_some() {
                    act("update-get", "Download", UpdateAction::Download, cx)
                } else {
                    act("update-notes", "View Release", UpdateAction::Notes, cx)
                },
            ),
            State::Downloading(r) => (
                format!("Downloading Deck {}…", r.version),
                div().into_any_element(),
            ),
            State::Ready(r, _) => (
                format!("Deck {} is ready.", r.version),
                act(
                    "update-restart",
                    "Restart to Update",
                    UpdateAction::Restart,
                    cx,
                ),
            ),
            State::Failed(e) => (e, act("update-check", "Try Again", UpdateAction::Check, cx)),
        };
        group(
            "Updates",
            vec![
                row(format!("Version {VERSION}"), control).about(about),
                row(
                    "Check automatically",
                    self.toggle(
                        "check-updates",
                        self.settings.check_updates,
                        |s, on| s.check_updates = on,
                        cx,
                    ),
                )
                .about("Looks for a newer Deck once a day and shows it in the title bar."),
            ],
            None,
        )
    }

    /// Every page's sections, in order.
    fn outline(&self, window: &Window, cx: &mut Context<Self>) -> Vec<(Page, Vec<Section>)> {
        Page::ALL
            .iter()
            .map(|p| (*p, self.sections(*p, window, cx)))
            .collect()
    }

    fn sections(&self, page: Page, window: &Window, cx: &mut Context<Self>) -> Vec<Section> {
        match page {
            Page::General => self.page_general(cx),
            Page::Appearance => self.page_appearance(window, cx),
            Page::Terminal => self.page_terminal(cx),
            Page::Browser => self.page_browser(cx),
            Page::Agents => self.page_agents(cx),
            Page::Keyboard => self.page_keyboard(cx),
            Page::Snippets => self.page_snippets(cx),
        }
    }

    // -- keyboard -------------------------------------------------------------

    /// Wait for the next keystroke and make it `id`'s shortcut. It is taken
    /// before the keymap sees it, so pressing ⌘T here does not open a tab.
    fn record(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.recording.as_ref().is_some_and(|(r, _)| *r == id) {
            self.recording = None;
            cx.notify();
            return;
        }
        let this = cx.entity().downgrade();
        let hook = cx.intercept_keystrokes(move |ev, _, cx| {
            let k = ev.keystroke.clone();
            if this.update(cx, |p, cx| p.recorded(k, cx)).is_ok() {
                cx.stop_propagation();
            }
        });
        self.key_note = None;
        self.recording = Some((id, hook));
        cx.notify();
    }

    fn recorded(&mut self, k: gpui::Keystroke, cx: &mut Context<Self>) {
        let Some((id, _)) = self.recording.take() else {
            return;
        };
        let m = k.modifiers;
        let plain = !(m.platform || m.control || m.alt || m.shift);
        if plain && k.key == "escape" {
            cx.notify();
            return;
        }
        let keys = if plain && (k.key == "backspace" || k.key == "delete") {
            String::new()
        } else if crate::keys::allowed(&k) {
            k.unparse()
        } else {
            self.key_note = Some((id, "A shortcut needs ⌘, ⌃ or ⌥ (or an F-key).".into()));
            cx.notify();
            return;
        };
        let default = crate::workspace::SHORTCUTS
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.defaults.first().copied());
        self.change(cx, |s| {
            if default == Some(keys.as_str()) {
                s.keys.remove(id);
            } else {
                s.keys.insert(id.to_string(), keys.clone());
            }
        });
    }

    fn page_keyboard(&self, cx: &mut Context<Self>) -> Vec<Section> {
        use crate::workspace::SHORTCUTS;
        let mine = &self.settings.keys;
        let norm = |k: &str| {
            gpui::Keystroke::parse(k)
                .map(|k| k.unparse())
                .unwrap_or_default()
        };
        // Who answers to what, to say when two want the same keys.
        let mut taken: Vec<(String, &'static str)> = FIXED_KEYS
            .iter()
            .map(|(k, name)| (norm(k), *name))
            .collect();
        for s in SHORTCUTS {
            for k in s.keys(mine) {
                taken.push((norm(k), s.name));
            }
        }
        let mut sections: Vec<Section> = Vec::new();
        for (ix, s) in SHORTCUTS.iter().enumerate() {
            let keys = s.keys(mine);
            let recording = self.recording.as_ref().is_some_and(|(r, _)| *r == s.id);
            let shown: SharedString = if recording {
                "Press a shortcut…".into()
            } else if keys.is_empty() {
                "None".into()
            } else {
                keys.iter()
                    .map(|k| crate::keys::pretty(k))
                    .collect::<Vec<_>>()
                    .join("  or  ")
                    .into()
            };
            let id = s.id;
            let cap = div()
                .id(ElementId::NamedInteger("keycap".into(), ix as u64))
                .flex()
                .items_center()
                .h(u(28.))
                .min_w(u(72.))
                .px(u(10.))
                .rounded(u(7.))
                .border_1()
                .border_color(rgba(if recording {
                    theme::input_edge()
                } else {
                    theme::hairline()
                }))
                .bg(rgb(if recording {
                    theme::surface_hover()
                } else {
                    theme::surface()
                }))
                .text_size(u(13.))
                .text_color(rgb(if keys.is_empty() && !recording {
                    theme::dim()
                } else {
                    theme::text()
                }))
                .cursor_pointer()
                .hover(|d| d.bg(rgb(theme::surface_hover())))
                .child(shown)
                .on_click(cx.listener(move |p, _, _, cx| p.record(id, cx)));
            let reset = mine.contains_key(s.id).then(|| {
                div()
                    .id(ElementId::NamedInteger("keyreset".into(), ix as u64))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(u(28.))
                    .rounded(u(7.))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(theme::row_hover())))
                    .child(icon(Icon::Refresh, 13., theme::dim()))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.change(cx, |s| {
                            s.keys.remove(id);
                        })
                    }))
            });
            let control = div()
                .flex()
                .items_center()
                .gap(u(6.))
                .children(reset)
                .child(cap)
                .into_any_element();
            let clash = keys.iter().find_map(|k| {
                let k = norm(k);
                taken
                    .iter()
                    .find(|(t, name)| *t == k && *name != s.name)
                    .map(|(_, name)| format!("{} is also {name}.", crate::keys::pretty(&k)))
            });
            let note = self
                .key_note
                .as_ref()
                .filter(|(n, _)| *n == s.id)
                .map(|(_, t)| t.to_string())
                .or(clash);
            let mut r = row(s.name, control);
            if let Some(n) = note {
                r = r.about(n);
            }
            match sections.iter_mut().find(|sec| sec.title == s.group) {
                Some(sec) => sec.rows.push(r),
                None => sections.push(group(s.group, vec![r], None)),
            }
        }
        let restore = button("keys-restore", "Restore Deck's Shortcuts")
            .on_click(self.on(|s| s.keys.clear(), cx))
            .into_any_element();
        let top = group(
            "Shortcuts",
            vec![row("Change a shortcut", if mine.is_empty() {
                div().into_any_element()
            } else {
                restore
            })
            .about("Click its keys, then press the new ones. ⌫ leaves it without a shortcut, esc keeps it as it was. The menus and ⌘K show yours.")],
            None,
        );
        sections.insert(0, top);
        sections
    }

    /// Read again what macOS says about deck (after it changed it).
    pub fn refresh_system(&mut self, cx: &mut Context<Self>) {
        self.default_terminal = crate::handler::is_default();
        cx.notify();
    }

    /// Open `page`, at its top.
    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.editing = None;
        if page == Page::Snippets {
            self.snippets = snippets::load();
        }
        if page == Page::General {
            self.default_terminal = crate::handler::is_default();
        }
        self.scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        cx.notify();
    }

    // -- snippets -------------------------------------------------------------

    fn edit_snippet(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.snippets.get(ix).cloned() else {
            return;
        };
        let name = cx.new(|cx| {
            let mut f = Field::new("Name", cx);
            f.set_text(&s.name, cx);
            f.forget_undo();
            f
        });
        let keyword = cx.new(|cx| {
            let mut f = Field::new("review", cx);
            f.set_text(&s.keyword, cx);
            f.forget_undo();
            f
        });
        let body = cx.new(|cx| {
            let mut f = Field::multiline("The message to put in", cx);
            f.editor = true;
            f.set_text(&s.body, cx);
            f.forget_undo();
            f
        });
        let subs = [
            cx.subscribe(&name, |p, f, ev: &FieldEvent, cx| {
                if matches!(ev, FieldEvent::Changed) {
                    let t = f.read(cx).text().to_string();
                    p.update_snippet(|s| s.name = t, cx);
                }
            }),
            cx.subscribe(&keyword, |p, f, ev: &FieldEvent, cx| {
                if matches!(ev, FieldEvent::Changed) {
                    // One word: what follows `;` up to the space.
                    let t: String = f
                        .read(cx)
                        .text()
                        .trim_start_matches(';')
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    p.update_snippet(|s| s.keyword = t, cx);
                }
            }),
            cx.subscribe(&body, |p, f, ev: &FieldEvent, cx| {
                if matches!(ev, FieldEvent::Changed) {
                    let t = f.read(cx).text().to_string();
                    p.update_snippet(|s| s.body = t, cx);
                }
            }),
        ];
        // A new one starts at its message; an old one is opened to change.
        let first = if s.body.is_empty() { &body } else { &name };
        window.focus(&first.read(cx).focus.clone());
        self.editing = Some(Editing {
            ix,
            name,
            keyword,
            body,
            confirm: false,
            _subs: subs,
        });
        cx.notify();
    }

    fn update_snippet(&mut self, f: impl FnOnce(&mut Snippet), cx: &mut Context<Self>) {
        let Some(ix) = self.editing.as_ref().map(|e| e.ix) else {
            return;
        };
        if let Some(s) = self.snippets.get_mut(ix) {
            f(s);
            snippets::save(&self.snippets);
            cx.notify();
        }
    }

    fn new_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.snippets = snippets::load();
        let mut name = "New snippet".to_string();
        let mut n = 2;
        while self.snippets.iter().any(|s| s.name == name) {
            name = format!("New snippet {n}");
            n += 1;
        }
        self.snippets.push(Snippet {
            name,
            body: String::new(),
            keyword: String::new(),
            project: false,
        });
        snippets::save(&self.snippets);
        self.edit_snippet(self.snippets.len() - 1, window, cx);
    }

    /// Back to the list. One left with no message is dropped.
    fn done_editing(&mut self, cx: &mut Context<Self>) {
        if let Some(e) = self.editing.take() {
            if self
                .snippets
                .get(e.ix)
                .is_some_and(|s| s.body.trim().is_empty())
            {
                self.snippets.remove(e.ix);
                snippets::save(&self.snippets);
            }
        }
        cx.notify();
    }

    fn delete_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.editing.as_mut() else {
            return;
        };
        if !e.confirm {
            e.confirm = true;
            cx.notify();
            return;
        }
        let ix = e.ix;
        self.editing = None;
        if ix < self.snippets.len() {
            self.snippets.remove(ix);
            snippets::save(&self.snippets);
        }
        cx.notify();
    }

    /// The workspace changed something itself (⌘+, ⌘−), or agents finished
    /// being detected.
    pub fn sync(
        &mut self,
        settings: &Settings,
        agents: Option<Vec<(SharedString, SharedString)>>,
        cx: &mut Context<Self>,
    ) {
        if &self.settings != settings {
            self.settings = settings.clone();
            cx.notify();
        }
        if let Some(a) = agents {
            if a != self.agents {
                self.agents = a;
                cx.notify();
            }
        }
    }

    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        let mut next = self.settings.clone();
        f(&mut next);
        if next != self.settings {
            self.settings = next.clone();
            cx.emit(SettingsEvent::Changed(next));
            cx.notify();
        }
    }

    /// A click handler that edits the settings.
    fn on(
        &self,
        f: impl Fn(&mut Settings) + 'static,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
        let this = cx.entity().downgrade();
        move |_: &ClickEvent, _: &mut Window, cx: &mut gpui::App| {
            let _ = this.update(cx, |p, cx| p.change(cx, &f));
        }
    }

    // -- controls -------------------------------------------------------------

    /// `−  value  +`. A side that has hit its limit is drawn dim and does
    /// nothing.
    fn stepper(
        &self,
        id: &'static str,
        value: String,
        dec: Option<impl Fn(&mut Settings) + 'static>,
        inc: Option<impl Fn(&mut Settings) + 'static>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = |side: u64, glyph: &'static str, live: bool| {
            div()
                .id(ElementId::NamedInteger(id.into(), side))
                .flex()
                .items_center()
                .justify_center()
                .size(u(28.))
                .rounded(u(6.))
                .text_size(u(15.))
                .text_color(rgb(if live { theme::text() } else { theme::dim() }))
                .when(live, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::surface_hover())))
                })
                .child(glyph)
        };
        let mut minus = button(0, "−", dec.is_some());
        if let Some(f) = dec {
            minus = minus.on_click(self.on(f, cx));
        }
        let mut plus = button(1, "+", inc.is_some());
        if let Some(f) = inc {
            plus = plus.on_click(self.on(f, cx));
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .p(u(2.))
            .rounded(u(8.))
            .bg(rgb(theme::surface()))
            .child(minus)
            .child(
                div()
                    .min_w(u(56.))
                    .flex()
                    .justify_center()
                    .text_size(u(13.))
                    .text_color(rgb(theme::text()))
                    .child(value),
            )
            .child(plus)
            .into_any_element()
    }

    /// A row of mutually exclusive choices; the chosen one is lit.
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        current: T,
        set: fn(&mut Settings, T),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(2.))
            .p(u(2.))
            .rounded(u(8.))
            .bg(rgb(theme::surface()));
        for (ix, (value, name)) in options.iter().enumerate() {
            let on = *value == current;
            let value = *value;
            row = row.child(
                div()
                    .id(ElementId::NamedInteger(id.into(), ix as u64))
                    .flex()
                    .items_center()
                    .h(u(26.))
                    .px(u(10.))
                    .rounded(u(6.))
                    .text_size(u(12.))
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(rgb(theme::selection()))
                            .text_color(rgb(theme::text()))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!on, |d| {
                        d.text_color(rgb(theme::text_2()))
                            .hover(|s| s.text_color(rgb(theme::text())))
                    })
                    .child(*name)
                    .on_click(self.on(move |s| set(s, value), cx)),
            );
        }
        row.into_any_element()
    }

    /// An on/off switch: white when on, as the composer's Full access chip.
    fn toggle(
        &self,
        id: &'static str,
        on: bool,
        set: fn(&mut Settings, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (from, to) = if on { (2., 16.) } else { (16., 2.) };
        let knob = div()
            .absolute()
            .top(u(2.))
            .size(u(16.))
            .rounded_full()
            .bg(rgb(if on { theme::pane() } else { theme::text_2() }));
        let knob = match self.flip {
            Some((flipped, n)) if flipped == id => knob
                .with_animation(
                    ElementId::NamedInteger(id.into(), n),
                    Animation::new(Duration::from_millis(140)).with_easing(ease_in_out),
                    move |d, t| d.left(u(from + (to - from) * t)),
                )
                .into_any_element(),
            _ => knob.left(u(to)).into_any_element(),
        };
        div()
            .id(id)
            .relative()
            .flex_none()
            .w(u(34.))
            .h(u(20.))
            .rounded_full()
            .cursor_pointer()
            .bg(rgb(if on {
                theme::text()
            } else {
                theme::surface_hover()
            }))
            .child(knob)
            .on_click(cx.listener(move |p, _, _, cx| {
                let n = p.flip.map_or(0, |(_, n)| n + 1);
                p.flip = Some((id, n));
                p.change(cx, |s| set(s, !on));
            }))
            .into_any_element()
    }

    /// A pickable chip; the chosen one is outlined in white.
    fn chip(id: ElementId, on: bool) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(u(8.))
            .h(u(32.))
            .px(u(12.))
            .rounded(u(8.))
            .border_1()
            .text_size(u(13.))
            .cursor_pointer()
            .when(on, |d| {
                d.bg(rgb(theme::surface()))
                    .border_color(rgba(theme::alpha(theme::text(), 0xb3)))
                    .text_color(rgb(theme::text()))
            })
            .when(!on, |d| {
                d.border_color(rgba(theme::input_edge()))
                    .text_color(rgb(theme::text_2()))
                    .hover(|s| s.bg(rgb(theme::row_hover())).text_color(rgb(theme::text())))
            })
    }

    fn fonts(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut wrap = div().flex().flex_wrap().gap(u(6.));
        for (ix, family) in self.fonts.iter().enumerate() {
            let on = family.as_ref() == self.settings.font_family;
            let name = family.clone();
            wrap = wrap.child(
                Self::chip(ElementId::NamedInteger("font".into(), ix as u64), on)
                    // Each name in its own face, so the choice is visible.
                    .font_family(family.clone())
                    .child(family.clone())
                    .on_click(self.on(move |s| s.font_family = name.to_string(), cx)),
            );
        }
        wrap.into_any_element()
    }

    /// Each scheme as a card: a strip of its background with its colours on
    /// it, the name below.
    fn schemes(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut wrap = div().flex().flex_wrap().gap(u(8.));
        for (ix, scheme) in Scheme::ALL.iter().copied().enumerate() {
            let on = scheme == self.settings.scheme;
            let p = scheme.palette();
            let mut strip = div()
                .flex()
                .items_center()
                .gap(u(5.))
                .h(u(30.))
                .px(u(10.))
                .rounded(u(6.))
                .bg(rgb(p.bg))
                .border_1()
                .border_color(rgba(theme::hairline()))
                .child(
                    div()
                        .mr(u(4.))
                        .font_family(theme::MONO)
                        .text_size(u(12.))
                        .text_color(rgb(p.fg))
                        .child("~ $"),
                );
            for c in &p.ansi[1..7] {
                strip = strip.child(div().size(u(9.)).rounded_full().bg(rgb(*c)));
            }
            wrap = wrap.child(
                div()
                    .id(ElementId::NamedInteger("scheme".into(), ix as u64))
                    .flex()
                    .flex_col()
                    .gap(u(8.))
                    .w(u(268.))
                    .p(u(8.))
                    .rounded(u(10.))
                    .border_1()
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(rgb(theme::surface()))
                            .border_color(rgba(theme::alpha(theme::text(), 0xb3)))
                    })
                    .when(!on, |d| {
                        d.border_color(rgba(theme::hairline()))
                            .hover(|s| s.bg(rgb(theme::row_hover())))
                    })
                    .child(strip)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(u(2.))
                            .text_size(u(12.))
                            .text_color(rgb(if on { theme::text() } else { theme::text_2() }))
                            .child(div().flex_1().child(scheme.name()))
                            .when(on, |d| d.child(icon(Icon::Check, 13., theme::text()))),
                    )
                    .on_click(self.on(move |s| s.scheme = scheme, cx)),
            );
        }
        wrap.into_any_element()
    }
}

// -- layout -----------------------------------------------------------------------

/// The pages, as the sidebar lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    General,
    Appearance,
    Terminal,
    Browser,
    Agents,
    Keyboard,
    Snippets,
}

impl Page {
    const ALL: [Page; 7] = [
        Page::General,
        Page::Appearance,
        Page::Terminal,
        Page::Browser,
        Page::Agents,
        Page::Keyboard,
        Page::Snippets,
    ];

    fn name(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::Terminal => "Terminal",
            Page::Browser => "Browser",
            Page::Agents => "Agents",
            Page::Keyboard => "Keyboard",
            Page::Snippets => "Snippets",
        }
    }
}

/// The page column's width.
const PAGE_W: f32 = 680.;

/// One setting: its name and what it does on the leading side, its control
/// on the trailing one (or below, across the width, when `wide`).
struct Row {
    label: SharedString,
    about: Option<SharedString>,
    control: AnyElement,
    wide: bool,
}

impl Row {
    /// What the setting does, under its name.
    fn about(mut self, text: impl Into<SharedString>) -> Self {
        self.about = Some(text.into());
        self
    }

    fn matches(&self, q: &str) -> bool {
        has(&self.label, q) || self.about.as_ref().is_some_and(|a| has(a, q))
    }
}

fn row(label: impl Into<SharedString>, control: AnyElement) -> Row {
    Row {
        label: label.into(),
        about: None,
        control,
        wide: false,
    }
}

/// A row whose control needs the width: the words above, the control below.
fn wide(label: impl Into<SharedString>, control: AnyElement) -> Row {
    Row {
        wide: true,
        ..row(label, control)
    }
}

/// A titled run of settings; the sidebar lists the titles under their page
/// and search looks through the rows.
struct Section {
    title: &'static str,
    rows: Vec<Row>,
    note: Option<SharedString>,
}

fn group(title: &'static str, rows: Vec<Row>, note: Option<SharedString>) -> Section {
    Section { title, rows, note }
}

/// A section that is one list to pick from.
fn choices(title: &'static str, items: Vec<Stateful<Div>>, note: Option<SharedString>) -> Section {
    let list = div()
        .flex()
        .flex_col()
        .gap(u(2.))
        .children(items)
        .into_any_element();
    group(title, vec![wide("", list)], note)
}

/// Case-insensitive `contains`.
fn has(text: &str, q: &str) -> bool {
    q.is_empty() || text.to_lowercase().contains(&q.to_lowercase())
}

/// A section as the page draws it: its title over a hairline, then its rows
/// with a faint line between them. Searching keeps the rows that match (all
/// of them when the title does); `None` when nothing is left.
fn render_section(sec: Section, q: &str) -> Option<Div> {
    let whole = has(sec.title, q);
    let rows: Vec<Row> = sec
        .rows
        .into_iter()
        .filter(|r| whole || r.matches(q))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut d = div().flex().flex_col().w_full();
    if !sec.title.is_empty() {
        d = d
            .child(
                div()
                    .pb(u(10.))
                    .text_size(u(13.))
                    .text_color(rgb(theme::dim()))
                    .child(sec.title),
            )
            .child(div().h(u(1.)).bg(rgba(theme::hairline())));
    }
    for (ix, r) in rows.into_iter().enumerate() {
        if ix > 0 {
            d = d.child(div().h(u(1.)).bg(rgba(theme::alpha(theme::text(), 0x0d))));
        }
        d = d.child(render_row(r));
    }
    if let Some(n) = sec.note.filter(|_| q.is_empty() || whole) {
        d = d.child(
            div()
                .pt(u(2.))
                .pb(u(4.))
                .text_size(u(12.))
                .line_height(u(17.))
                .text_color(rgb(theme::dim()))
                .child(n),
        );
    }
    Some(d)
}

fn render_row(r: Row) -> Div {
    let words = (!r.label.is_empty() || r.about.is_some()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(u(4.))
            .flex_1()
            .min_w_0()
            .when(!r.label.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(u(14.))
                        .text_color(rgb(theme::text()))
                        .child(r.label),
                )
            })
            .children(r.about.map(|a| {
                div()
                    .text_size(u(12.5))
                    .line_height(u(18.))
                    .text_color(rgb(theme::dim()))
                    .child(a)
            }))
    });
    if r.wide {
        div()
            .flex()
            .flex_col()
            .gap(u(12.))
            .py(u(16.))
            .children(words)
            .child(r.control)
    } else {
        div()
            .flex()
            .items_center()
            .gap(u(28.))
            .min_h(u(56.))
            .py(u(14.))
            .children(words)
            .child(div().flex_none().child(r.control))
    }
}

/// One choice in a list group: an optional mark, the name and a line under
/// it, a check on the chosen one.
fn choice(
    id: ElementId,
    mark: Option<Icon>,
    label: SharedString,
    detail: Option<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(u(10.))
        .min_h(u(40.))
        .px(u(10.))
        .rounded(u(8.))
        .py(u(6.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::row_hover())))
        .children(mark.map(|m| icon(m, 15., if on { theme::text() } else { theme::text_2() })))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(u(13.))
                        .text_color(rgb(theme::text()))
                        .child(label),
                )
                .children(detail.map(|d| {
                    div()
                        .text_size(u(11.))
                        .text_color(rgb(theme::dim()))
                        .child(d)
                })),
        )
        .child(
            div()
                .w(u(14.))
                .flex_none()
                .when(on, |d| d.child(icon(Icon::Check, 14., theme::text()))),
        )
}

/// A small window in a theme's colours: sidebar, a pane with a few lines,
/// the accent where the caret would be.
fn preview(c: &theme::Colors) -> Div {
    let line = |w: f32, color: u32| div().h(u(3.)).w(u(w)).rounded(u(2.)).bg(rgb(color));
    div()
        .flex()
        .w(u(104.))
        .h(u(66.))
        .rounded(u(8.))
        .overflow_hidden()
        .bg(rgb(c.sidebar))
        .border_1()
        .border_color(rgba(if c.dark { 0xffffff1f } else { 0x0000001f }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(u(4.))
                .w(u(28.))
                .pt(u(14.))
                .px(u(5.))
                .child(div().h(u(6.)).rounded(u(2.)).bg(rgb(c.surface)))
                .child(line(14., c.dim))
                .child(line(16., c.dim)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(u(5.))
                .flex_1()
                .mt(u(8.))
                .mr(u(5.))
                .mb(u(5.))
                .p(u(6.))
                .rounded(u(5.))
                .bg(rgb(c.pane))
                .child(line(38., c.text))
                .child(line(30., c.text_2))
                .child(line(44., c.dim))
                .child(div().w(u(2.)).h(u(8.)).bg(rgb(c.accent))),
        )
}

impl SettingsPane {
    /// The window themes as tiles, System first, as macOS shows its own.
    fn themes(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let system_dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let mut tiles = div().flex().flex_wrap().gap(u(14.));
        for (ix, t) in theme::Theme::ALL.iter().copied().enumerate() {
            let on = t == self.settings.theme;
            let face = if t == theme::Theme::System {
                // Half light, half dark: it is both.
                div()
                    .relative()
                    .child(preview(theme::Theme::Light.colors(system_dark)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(u(52.))
                            .w(u(52.))
                            .h(u(66.))
                            .overflow_hidden()
                            .rounded_r(u(8.))
                            .child(
                                div()
                                    .ml(u(-52.))
                                    .child(preview(theme::Theme::Black.colors(true))),
                            ),
                    )
            } else {
                preview(t.colors(system_dark))
            };
            tiles = tiles.child(
                div()
                    .id(ElementId::NamedInteger("theme".into(), ix as u64))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(6.))
                    .cursor_pointer()
                    .child(
                        div()
                            .p(u(2.))
                            .rounded(u(10.))
                            .border_2()
                            .border_color(rgba(if on {
                                theme::alpha(theme::accent(), 0xff)
                            } else {
                                0x00000000
                            }))
                            .child(face),
                    )
                    .child(
                        div()
                            .text_size(u(12.))
                            .text_color(rgb(if on { theme::text() } else { theme::text_2() }))
                            .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(t.name()),
                    )
                    .on_click(self.on(move |s| s.theme = t, cx)),
            );
        }
        tiles.into_any_element()
    }

    /// The pages as a tree: each one's sections under it, folding shut by
    /// its chevron, with the search box on top.
    fn sidebar(
        &self,
        titles: &[(Page, Vec<&'static str>)],
        searching_focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut col = div()
            .id("settings-tree")
            .flex()
            .flex_col()
            .flex_none()
            .gap(u(2.))
            .w(u(232.))
            .h_full()
            .overflow_y_scroll()
            .pt(u(16.))
            .px(u(10.))
            .pb(u(16.))
            .bg(rgb(theme::sidebar()))
            .border_r_1()
            .border_color(rgba(theme::hairline()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(u(32.))
                    .mb(u(12.))
                    .pl(u(10.))
                    .rounded(u(8.))
                    .border_1()
                    .border_color(rgba(if searching_focused {
                        theme::input_edge()
                    } else {
                        theme::hairline()
                    }))
                    .bg(rgb(if searching_focused {
                        theme::surface_hover()
                    } else {
                        theme::surface()
                    }))
                    .text_size(u(13.))
                    .child(icon(Icon::Search, 13., theme::dim()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .ml(u(-4.))
                            .child(self.search.clone()),
                    ),
            );
        let searching = !self.query(cx).is_empty();
        for (pi, (page, sections)) in titles.iter().enumerate() {
            let page = *page;
            let on = page == self.page && !searching;
            let open = !self.collapsed.contains(&page);
            let foldable = sections.len() > 1;
            col = col.child(
                div()
                    .id(ElementId::NamedInteger("page".into(), pi as u64))
                    .flex()
                    .items_center()
                    .gap(u(6.))
                    .h(u(34.))
                    .px(u(6.))
                    .rounded(u(8.))
                    .text_size(u(14.))
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(rgb(theme::surface())).text_color(rgb(theme::text()))
                    })
                    .when(!on, |d| {
                        d.text_color(rgb(theme::text_2()))
                            .hover(|s| s.bg(rgb(theme::row_hover())))
                    })
                    .child(
                        div()
                            .id(ElementId::NamedInteger("fold".into(), pi as u64))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(u(18.))
                            .when(foldable, |d| {
                                d.child(icon(
                                    if open {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    },
                                    12.,
                                    theme::dim(),
                                ))
                                .on_click(cx.listener(
                                    move |p, _, _, cx| {
                                        cx.stop_propagation();
                                        if !p.collapsed.remove(&page) {
                                            p.collapsed.insert(page);
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                    )
                    .child(page.name())
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.collapsed.remove(&page);
                        p.search.update(cx, |f, cx| f.set_text("", cx));
                        p.show(page, cx);
                    })),
            );
            if foldable && open {
                let mut kids = div()
                    .flex()
                    .flex_col()
                    .gap(u(2.))
                    .ml(u(15.))
                    .pl(u(10.))
                    .border_l_1()
                    .border_color(rgba(theme::hairline()));
                for (si, title) in sections.iter().enumerate() {
                    kids = kids.child(
                        div()
                            .id(ElementId::NamedInteger(
                                "section".into(),
                                (pi * 100 + si) as u64,
                            ))
                            .flex()
                            .items_center()
                            .h(u(32.))
                            .px(u(10.))
                            .rounded(u(8.))
                            .text_size(u(13.5))
                            .text_color(rgb(theme::text_2()))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(theme::row_hover())).text_color(rgb(theme::text())))
                            .child(*title)
                            .on_click(cx.listener(move |p, _, _, cx| p.show_section(page, si, cx))),
                    );
                }
                col = col.child(kids);
            }
        }
        col.into_any_element()
    }

    fn page_snippets(&self, cx: &mut Context<Self>) -> Vec<Section> {
        if let Some(e) = &self.editing {
            let delete = button(
                "snippet-delete",
                if e.confirm {
                    "Click Again to Delete"
                } else {
                    "Delete Snippet"
                },
            )
            .on_click(cx.listener(|p, _, _, cx| p.delete_snippet(cx)));
            let done = button("snippet-done", "Done")
                .on_click(cx.listener(|p, _, _, cx| p.done_editing(cx)));
            let actions = div()
                .flex()
                .gap(u(8.))
                .child(delete)
                .child(done)
                .into_any_element();
            return vec![group(
                "Snippet",
                vec![
                    row("Name", div().w(u(260.)).child(e.name.clone()).into_any_element()),
                    row("Keyword", div().w(u(180.)).child(e.keyword.clone()).into_any_element())
                        .about("Type ;keyword and a space in a chat to put the message in."),
                    wide("Message", e.body.clone().into_any_element()).about(
                        "⏎ starts a new line. {clipboard}, {selection}, {branch} and {folder} become the clipboard, what is selected, and the branch and folder you are in when the snippet goes in.",
                    ),
                    row("", actions).about("Saved as you type."),
                ],
                None,
            )];
        }
        let add = button("snippet-new", "New Snippet")
            .on_click(cx.listener(|p, _, w, cx| p.new_snippet(w, cx)));
        let note: SharedString = "⌘; lists these anywhere, and ;keyword with a space expands one in a chat. The one you pick goes into the chat composer or terminal you are in without being sent, so you can add to it first. Save as Snippet in ⌘K or the Edit menu keeps what you have typed, or what is selected in a terminal. A project can share its own in .deck/snippets.json, committed with the code.".into();
        if self.snippets.is_empty() {
            return vec![group(
                "Snippets",
                vec![row("No snippets yet", add.into_any_element())],
                Some(note),
            )];
        }
        let rows = self
            .snippets
            .iter()
            .enumerate()
            .map(|(ix, s)| {
                div()
                    .id(ElementId::NamedInteger("snippet".into(), ix as u64))
                    .flex()
                    .items_center()
                    .gap(u(12.))
                    .min_h(u(52.))
                    .px(u(10.))
                    .py(u(8.))
                    .rounded(u(8.))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(theme::row_hover())))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(u(2.))
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .one_line()
                                    .text_size(u(13.))
                                    .text_color(rgb(theme::text()))
                                    .child(s.name.clone()),
                            )
                            .child(
                                div()
                                    .one_line()
                                    .text_size(u(11.))
                                    .text_color(rgb(theme::dim()))
                                    .child(snippets::detail(s)),
                            ),
                    )
                    .child(icon(Icon::ChevronRight, 13., theme::dim()))
                    .on_click(cx.listener(move |p, _, w, cx| p.edit_snippet(ix, w, cx)))
            })
            .collect();
        let mut list = choices("Snippets", rows, Some(note));
        list.rows.push(row("", add.into_any_element()));
        vec![list]
    }

    fn page_general(&self, cx: &mut Context<Self>) -> Vec<Section> {
        let path = settings::path();
        let shown = tilde(&path.display().to_string());
        let reveal = button("reveal", "Show in Finder").on_click(move |_, _, _| {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(&path)
                .spawn();
        });
        let reset = button("defaults", "Restore Defaults").on_click(self.on(
            |s| {
                // What was asked once stays asked.
                let asked = s.asked_default_terminal;
                *s = Settings::default();
                s.asked_default_terminal = asked;
            },
            cx,
        ));
        let on = self.default_terminal;
        let terminal = button(
            "default-terminal",
            if on {
                "Give Back to Terminal"
            } else {
                "Make Default"
            },
        )
        .on_click(cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::DefaultTerminal(!on))));
        vec![
            self.account_section(cx),
            self.updates_section(cx),
            group(
                "Default terminal",
                vec![row(
                    if on {
                        "Deck is your default terminal"
                    } else {
                        "Make Deck the default terminal"
                    },
                    terminal.into_any_element(),
                )
                .about("Shell scripts, .command files and ssh:, telnet: and x-man-page: links open in Deck, each in a new tab.")],
                None,
            ),
            group(
                "Settings file",
                vec![
                    row("Settings file", reveal.into_any_element())
                        .about(format!("Every change is saved to {shown} the moment you make it.")),
                    row("Restore defaults", reset.into_any_element()).about(
                        "Puts every setting back the way Deck ships. Your tabs and threads are not touched.",
                    ),
                ],
                None,
            ),
        ]
    }

    fn page_appearance(&self, window: &Window, cx: &mut Context<Self>) -> Vec<Section> {
        let s = &self.settings;
        let (min_zoom, max_zoom) = (ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let zoom = self.stepper(
            "zoom",
            format!("{:.0}%", s.zoom * 100.),
            (s.zoom > min_zoom).then_some(|s: &mut Settings| s.zoom = s.stepped(-1)),
            (s.zoom < max_zoom).then_some(|s: &mut Settings| s.zoom = s.stepped(1)),
            cx,
        );
        vec![
            group(
                "Theme",
                vec![wide("Window theme", self.themes(window, cx)).about(
                    "The whole window: sidebar, panes, chats and menus, and the appearance web pages and scrollbars take. System follows macOS.",
                )],
                None,
            ),
            group(
                "Scale",
                vec![row("Zoom", zoom)
                    .about("One size for everything. ⌘+ and ⌘− step it, ⌘0 puts it back.")],
                None,
            ),
            group(
                "Terminal colours",
                vec![wide("Palette", self.schemes(cx)).about(
                    "What terminals and agent TUIs draw with. Match window uses the theme's own.",
                )],
                None,
            ),
        ]
    }

    fn page_terminal(&self, cx: &mut Context<Self>) -> Vec<Section> {
        let s = &self.settings;
        let font_px = self.stepper(
            "font-px",
            format!("{} pt", s.font_px.round()),
            (s.font_px > FONT_PX_RANGE.0).then_some(|s: &mut Settings| {
                s.font_px = (s.font_px.round() - 1.).max(FONT_PX_RANGE.0)
            }),
            (s.font_px < FONT_PX_RANGE.1).then_some(|s: &mut Settings| {
                s.font_px = (s.font_px.round() + 1.).min(FONT_PX_RANGE.1)
            }),
            cx,
        );
        let line = self.stepper(
            "line-height",
            format!("{:.1}", s.line_height),
            (s.line_height > LINE_HEIGHT_RANGE.0 + 0.01).then_some(|s: &mut Settings| {
                s.line_height = step_tenth(s.line_height, -1., LINE_HEIGHT_RANGE)
            }),
            (s.line_height < LINE_HEIGHT_RANGE.1 - 0.01).then_some(|s: &mut Settings| {
                s.line_height = step_tenth(s.line_height, 1., LINE_HEIGHT_RANGE)
            }),
            cx,
        );
        let cursors: Vec<(Cursor, &'static str)> =
            Cursor::ALL.iter().map(|c| (*c, c.name())).collect();
        vec![
            group(
                "Typography",
                vec![
                    wide("Terminal font", self.fonts(cx))
                        .about("Monospace faces installed on this Mac."),
                    row("Font size", font_px).about("The text size terminals use, at 100% zoom."),
                    row("Line height", line)
                        .about("Space between lines, as a multiple of the font size."),
                ],
                None,
            ),
            group(
                "Cursor",
                vec![
                    row(
                        "Shape",
                        self.segmented("cursor", &cursors, s.cursor, |s, c| s.cursor = c, cx),
                    )
                    .about("Until a program asks for a shape of its own."),
                    row(
                        "Blink",
                        self.toggle("blink", s.cursor_blink, |s, on| s.cursor_blink = on, cx),
                    )
                    .about("Only while the terminal has focus."),
                ],
                None,
            ),
            group(
                "Colours",
                vec![row(
                    "Soften programs' backgrounds",
                    self.toggle(
                        "soften",
                        s.soften_backgrounds,
                        |s, on| s.soften_backgrounds = on,
                        cx,
                    ),
                )
                .about("Diffs and highlights that programs like Claude Code paint in solid colour show as a tint instead. Greys and your selection stay as they are.")],
                None,
            ),
            group(
                "Option key",
                vec![row(
                    "Use ⌥ as Meta",
                    self.toggle("meta", s.option_as_meta, |s, on| s.option_as_meta = on, cx),
                )
                .about("⌥B and ⌥F jump by word in the shell, instead of typing ∫ and ƒ.")],
                None,
            ),
        ]
    }

    fn page_browser(&self, cx: &mut Context<Self>) -> Vec<Section> {
        let rows = Search::ALL
            .iter()
            .copied()
            .enumerate()
            .map(|(ix, e)| {
                let host = Settings {
                    search: e,
                    ..Settings::default()
                }
                .search_url("…")
                .trim_start_matches("https://")
                .trim_start_matches("www.")
                .split('/')
                .next()
                .unwrap_or_default()
                .to_string();
                choice(
                    ElementId::NamedInteger("engine".into(), ix as u64),
                    None,
                    e.name().into(),
                    Some(host.into()),
                    e == self.settings.search,
                )
                .on_click(self.on(move |s| s.search = e, cx))
            })
            .collect();
        vec![choices(
            "Search engine",
            rows,
            Some("Anything typed in the address bar that is not an address is searched for here, and suggestions come from it too.".into()),
        )]
    }

    fn page_agents(&self, cx: &mut Context<Self>) -> Vec<Section> {
        let current = self.settings.default_agent.clone();
        let mut listed: Vec<(SharedString, SharedString)> = self.agents.clone();
        // An agent chosen earlier that is not ready now still shows as chosen.
        if let Some(id) = current.as_deref() {
            if !listed.iter().any(|(a, _)| a.as_ref() == id) {
                listed.push((id.to_string().into(), format!("{id} (not ready)").into()));
            }
        }
        let mut start = vec![choice(
            "agent-auto".into(),
            Some(Icon::GenericAgent),
            "Automatic".into(),
            Some("Claude when it is signed in, else the first agent that is ready".into()),
            current.is_none(),
        )
        .on_click(self.on(|s| s.default_agent = None, cx))];
        for (ix, (id, name)) in listed.into_iter().enumerate() {
            let on = current.as_deref() == Some(id.as_ref());
            let pick = id.to_string();
            start.push(
                choice(
                    ElementId::NamedInteger("agent".into(), ix as u64),
                    Some(Icon::agent(&id)),
                    name,
                    None,
                    on,
                )
                .on_click(self.on(move |s| s.default_agent = Some(pick.clone()), cx)),
            );
        }
        let access = Access::ALL
            .iter()
            .copied()
            .enumerate()
            .map(|(ix, a)| {
                let detail = match a {
                    Access::Ask => "Asks before every edit and command",
                    Access::Edits => "Edits files on its own, asks before commands",
                    Access::Full => "Edits and runs commands without asking",
                };
                choice(
                    ElementId::NamedInteger("access".into(), ix as u64),
                    None,
                    a.name().into(),
                    Some(detail.into()),
                    a == self.settings.access,
                )
                .on_click(self.on(move |s| s.access = a, cx))
            })
            .collect();
        vec![
            choices(
                "New chats",
                start,
                (self.agents.is_empty()).then(|| "Detecting agents…".into()),
            ),
            choices(
                "Access",
                access,
                Some(
                    "What a new thread may do without asking. Each thread's chip still changes it."
                        .into(),
                ),
            ),
            group(
                "Prompt rewriting",
                vec![
                    row(
                        "When you press ⏎",
                        self.segmented(
                            "refine",
                            &crate::refine::Mode::ALL
                                .iter()
                                .map(|m| (*m, m.name()))
                                .collect::<Vec<_>>(),
                            self.settings.refine,
                            |s, m| s.refine = m,
                            cx,
                        ),
                    ),
                    row(
                        "Model",
                        self.segmented(
                            "refine-model",
                            &crate::refine::MODELS,
                            crate::refine::MODELS
                                .iter()
                                .map(|(id, _)| *id)
                                .find(|id| *id == self.settings.refine_model)
                                .unwrap_or("haiku"),
                            |s, m| s.refine_model = m.to_string(),
                            cx,
                        ),
                    ),
                ],
                Some(
                    "A quick Claude run turns what you typed or dictated into a clear, complete prompt, using the thread's task and the agent's last reply to work out what \"it\" and \"that\" mean. ⌃O rewrites into the box without sending, ⌃Z brings back your words. Short replies, / commands and ! commands go as typed."
                        .into(),
                ),
            ),
            group(
                "Recovery",
                vec![row(
                    "Retry when a turn fails",
                    self.toggle(
                        "auto-retry",
                        self.settings.auto_retry,
                        |s, on| s.auto_retry = on,
                        cx,
                    ),
                )],
                Some(
                    "On an API or network error deck says \"continue\" after 5 s, 10 s, 20 s… up to five minutes, eight tries; a usage limit waits until it resets. Login and billing problems are shown to you instead. Claude Code stuck in its own retry wait in a terminal is nudged the same way."
                        .into(),
                ),
            ),
            group(
                "Notifications",
                vec![
                    row(
                        "When an agent needs you",
                        self.toggle(
                            "notify-attention",
                            self.settings.notify_attention,
                            |s, on| s.notify_attention = on,
                            cx,
                        ),
                    ),
                    row(
                        "When an agent finishes",
                        self.toggle(
                            "notify-done",
                            self.settings.notify_done,
                            |s, on| s.notify_done = on,
                            cx,
                        ),
                    ),
                ],
                Some(
                    "Only while you are looking at something else. A finish is told once work has run for ten seconds; quick replies are not."
                        .into(),
                ),
            ),
        ]
    }
}

/// A plain push button, the kind System Settings puts at a row's end.
fn button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(u(26.))
        .px(u(12.))
        .rounded(u(6.))
        .bg(rgb(theme::surface()))
        .border_1()
        .border_color(rgba(theme::hairline()))
        .text_size(u(12.))
        .text_color(rgb(theme::text()))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::surface_hover())))
        .child(label)
}

impl Render for SettingsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.query(cx);
        let outline = self.outline(window, cx);
        let titles: Vec<(Page, Vec<&'static str>)> = outline
            .iter()
            .map(|(p, secs)| {
                (
                    *p,
                    secs.iter()
                        .map(|s| s.title)
                        .filter(|t| !t.is_empty())
                        .collect(),
                )
            })
            .collect();
        let focused = self.search.read(cx).focus.is_focused(window);
        let sidebar = self.sidebar(&titles, focused, cx);

        // Each child of the scrolled column is one item, so the sidebar can
        // bring a section to the top.
        let item = || div().w(u(PAGE_W)).max_w(relative(1.)).px(u(32.));
        let title = |t: SharedString| {
            item().pt(u(32.)).pb(u(8.)).child(
                div()
                    .text_size(u(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::text()))
                    .child(t),
            )
        };
        let mut column = div()
            .id("settings-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(28.))
            .pb(u(64.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if q.is_empty() {
            let page = self.page;
            column = column.child(title(page.name().into()));
            for (p, secs) in outline {
                if p == page {
                    for sec in secs {
                        if let Some(d) = render_section(sec, "") {
                            column = column.child(item().child(d));
                        }
                    }
                }
            }
        } else {
            column = column.child(title(format!("Results for “{q}”").into()));
            let mut any = false;
            for (p, secs) in outline {
                let found: Vec<Div> = secs
                    .into_iter()
                    .filter_map(|sec| render_section(sec, &q))
                    .collect();
                if found.is_empty() {
                    continue;
                }
                any = true;
                column = column.child(
                    item().child(
                        div()
                            .text_size(u(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::text_2()))
                            .child(p.name()),
                    ),
                );
                for d in found {
                    column = column.child(item().child(d));
                }
            }
            if !any {
                column = column.child(
                    item()
                        .text_size(u(13.))
                        .text_color(rgb(theme::dim()))
                        .child("No settings match."),
                );
            }
        }

        div()
            .id("settings")
            .track_focus(&self.focus)
            .key_context("Settings")
            .size_full()
            .flex()
            .bg(rgb(theme::pane()))
            .child(sidebar)
            .child(column)
    }
}

/// A round avatar: the account's initial on a tinted disc.
fn avatar(a: &crate::account::Account, size: f32) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(u(size))
        .rounded_full()
        .bg(rgb(theme::accent()))
        .text_size(u(size * 0.45))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0xffffff))
        .child(a.initial())
        .into_any_element()
}

/// Keys Deck keeps for itself, so a shortcut set to one of them says so.
const FIXED_KEYS: &[(&str, &str)] = &[
    ("cmd-q", "Quit"),
    ("cmd-c", "Copy"),
    ("cmd-v", "Paste"),
    ("cmd-x", "Cut"),
    ("cmd-a", "Select All"),
    ("cmd-z", "Undo"),
    ("cmd-shift-z", "Redo"),
    ("cmd-f", "Find"),
    ("cmd-g", "Find Next"),
    ("cmd-shift-g", "Find Previous"),
    ("cmd-1", "Go to tab 1"),
    ("cmd-2", "Go to tab 2"),
    ("cmd-3", "Go to tab 3"),
    ("cmd-4", "Go to tab 4"),
    ("cmd-5", "Go to tab 5"),
    ("cmd-6", "Go to tab 6"),
    ("cmd-7", "Go to tab 7"),
    ("cmd-8", "Go to tab 8"),
    ("cmd-9", "Go to the last tab"),
];

/// `/Users/me/x` → `~/x`.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_offered_are_installed_ones_plus_menlo_and_the_current() {
        let installed: Vec<String> = [
            "Monaco",
            "Helvetica",
            "Fira Code",
            "FiraCode Nerd Font Mono",
            "FiraCode Nerd Font Mono",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let got = monospace_families(&installed, "Menlo");
        let got: Vec<&str> = got.iter().map(|s| s.as_ref()).collect();
        assert_eq!(
            got,
            ["Menlo", "Monaco", "Fira Code", "FiraCode Nerd Font Mono"]
        );
        let got = monospace_families(&installed, "My Hand Font");
        assert_eq!(got.last().map(|s| s.as_ref()), Some("My Hand Font"));
    }

    #[test]
    fn tenths_step_without_drift_and_stop_at_the_ends() {
        let mut v = 1.4;
        for _ in 0..3 {
            v = step_tenth(v, 1., LINE_HEIGHT_RANGE);
        }
        assert_eq!(v, 1.7);
        assert_eq!(step_tenth(2.0, 1., LINE_HEIGHT_RANGE), 2.0);
        assert_eq!(step_tenth(1.0, -1., LINE_HEIGHT_RANGE), 1.0);
        assert_eq!(step_tenth(1.385, -1., LINE_HEIGHT_RANGE), 1.3);
    }
}
