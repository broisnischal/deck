//! A terminal pane: a real PTY (see the `term` crate), painted directly.
//!
//! Glyphs are placed by column, never by shaping a whole row. A stretch of
//! plain characters (ASCII, Latin, box drawing, all exactly one Menlo cell)
//! is shaped as one line; anything else (CJK, emoji, symbols from a fallback
//! font) is shaped alone at its own column. A fallback glyph whose advance is
//! not exactly one cell can therefore never push the rest of the row off the
//! grid. Shaped lines are cached by gpui, so an unchanged row is a hash hit.
//!
//! Text arrives through the platform input handler (IME, dead keys, the emoji
//! picker); `key_down` only claims keys that become escape sequences.

use crate::field::{Field, FieldEvent};
use crate::icon::Icon;
use crate::menu::{self, Entry, Item};
use crate::pasteboard::{self, Pasted};
use crate::theme;
use crate::workspace::{
    icon_button, ClearTerminal, ClosePane, Copy, Find, FindNext, FindPrevious, Paste, SelectAll,
    SplitDown, SplitRight,
};
use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    div, fill, font, outline, point, px, relative, rgb, rgba, size, App, BorderStyle, Bounds,
    ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, ExternalPaths, FocusHandle, FontStyle, FontWeight, GlobalElementId,
    InspectorElementId, KeyDownEvent, Keystroke, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine,
    SharedString, Style, Subscription, Task, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::time::Instant;

pub enum TerminalEvent {
    Focused,
    /// The title, the bell or the progress changed: things the sidebar shows.
    Retitled,
    /// The program wants you: a notification it posted (OSC 9 / 777), or
    /// `None` for a plain bell.
    Alert(Option<String>),
    Exited,
    /// ⌘-click on a URL in the output.
    OpenUrl(String),
}

pub struct TerminalPane {
    term: term::Terminal,
    pub focus: FocusHandle,
    pub title: SharedString,
    /// When output last arrived; agents are "working" while this is recent.
    pub last_output: Instant,
    /// Set by BEL or a notification, cleared when the pane is looked at:
    /// "this one wants you".
    pub bell: bool,
    /// What the program said when it asked for you, if it said anything.
    pub alert: Option<String>,
    /// OSC 9;4 progress, once the program has reported any: `Some` while it
    /// works.
    progress: Option<Option<u8>>,
    exited: bool,
    look: Look,
    cell_w: f32,
    /// Where the grid was last painted, for turning mouse positions into cells.
    origin: Point<Pixels>,
    /// A press that has not moved yet; a drag turns it into a selection.
    press: Option<(usize, usize, bool)>,
    /// The button held down while the program is receiving mouse reports.
    reporting: Option<term::MouseButton>,
    /// Last cell a motion report was sent for, so moves within a cell are free.
    last_motion: Option<(usize, usize)>,
    /// Trackpads send fractions of a line; the remainder carries over.
    scroll_acc: f32,
    /// IME composition in progress, drawn at the cursor until committed.
    marked: Option<String>,
    /// Shaped rows from the last frame; see `RowCache`.
    rows: Vec<RowCache>,
    /// The find bar (⌘F), while open.
    find: Option<FindBar>,
    /// The right-click menu, while open.
    menu: Option<menu::Open>,
    /// The next snapshot must rebuild every row (the palette changed).
    restyle: bool,
    /// Whether a blinking cursor is in its visible half.
    blink_on: bool,
    /// When the user last typed: the cursor stays solid while they do.
    typed: Instant,
    _blink: Option<Task<()>>,
    _wake: Task<()>,
}

/// How terminals look and type, from Settings. Every pane gets the same one.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub family: SharedString,
    /// The font size on screen: the setting times the app zoom.
    pub font_px: f32,
    /// Line height as a multiple of `font_px`.
    pub line: f32,
    pub palette: term::Palette,
    pub cursor_color: u32,
    /// The cursor until a program asks for another (DECSCUSR).
    pub cursor: term::CursorShape,
    pub blink: bool,
    /// Option sends Esc+key (Meta) instead of typing Option-characters.
    pub option_meta: bool,
}

/// Half a blink cycle, as macOS text fields use.
const BLINK: std::time::Duration = std::time::Duration::from_millis(530);

struct FindBar {
    field: Entity<Field>,
    /// The query last searched for: the field also notifies on caret moves
    /// and focus changes, which must not search again.
    query: String,
    found: bool,
    _subs: [Subscription; 2],
}

impl EventEmitter<TerminalEvent> for TerminalPane {}

pub fn line_height(font_px: f32, factor: f32) -> f32 {
    (font_px * factor).round()
}

impl TerminalPane {
    pub fn spawn(
        cwd: &Path,
        command: Option<&str>,
        env: HashMap<String, String>,
    ) -> anyhow::Result<term::Terminal> {
        let mut spawn = match command {
            Some(c) => term::Spawn::command(c, cwd),
            None => term::Spawn::shell(cwd),
        };
        spawn.env.extend(env);
        term::Terminal::spawn(spawn, 80, 24)
    }

    pub fn new(
        term: term::Terminal,
        title: String,
        look: Look,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Woken by the PTY thread, so output reaches the screen on the next
        // frame instead of the next poll tick.
        let mut wakes = term.wakeups();
        let wake = cx.spawn(async move |this, cx| {
            while wakes.next().await.is_some() {
                let alive = this.update(cx, |p, cx| {
                    p.term.take_wake();
                    p.last_output = Instant::now();
                    // A selection the program has since redrawn over (or
                    // stretched by scrolling its own region) is no longer
                    // what the user selected; drop it rather than show it.
                    p.term.prune_selection();
                    for ev in p.term.drain_events() {
                        match ev {
                            term::TermEvent::Title(t) if !t.is_empty() && t != p.title.as_ref() => {
                                p.title = t.into();
                                cx.emit(TerminalEvent::Retitled);
                            }
                            term::TermEvent::Bell => {
                                p.bell = true;
                                cx.emit(TerminalEvent::Alert(None));
                                cx.emit(TerminalEvent::Retitled);
                            }
                            term::TermEvent::Notify { title, body } => {
                                let said = match title {
                                    Some(t) => format!("{t}: {body}"),
                                    None => body,
                                };
                                p.bell = true;
                                p.alert = Some(said.clone());
                                cx.emit(TerminalEvent::Alert(Some(said)));
                                cx.emit(TerminalEvent::Retitled);
                            }
                            term::TermEvent::Progress(state) if p.progress != Some(state) => {
                                p.progress = Some(state);
                                cx.emit(TerminalEvent::Retitled);
                            }
                            term::TermEvent::Exited(_) => {
                                p.exited = true;
                                cx.emit(TerminalEvent::Exited);
                            }
                            _ => {}
                        }
                    }
                    cx.notify();
                    !p.exited
                });
                if !matches!(alive, Ok(true)) {
                    return;
                }
            }
        });

        term.set_cursor_default(look.cursor, look.blink);
        Self {
            term,
            focus: cx.focus_handle(),
            title: title.into(),
            last_output: Instant::now(),
            bell: false,
            alert: None,
            progress: None,
            exited: false,
            cell_w: cell_width(window, &look.family, look.font_px),
            look,
            origin: Point::default(),
            press: None,
            reporting: None,
            last_motion: None,
            scroll_acc: 0.,
            marked: None,
            rows: Vec::new(),
            find: None,
            menu: None,
            restyle: false,
            blink_on: true,
            typed: Instant::now(),
            _blink: None,
            _wake: wake,
        }
    }

    pub fn pid(&self) -> u32 {
        self.term.pid()
    }

    fn line_px(&self) -> f32 {
        line_height(self.look.font_px, self.look.line)
    }

    /// Settings or the zoom changed: re-measure the cell and redraw every row.
    pub fn set_look(&mut self, look: Look, window: &Window, cx: &mut Context<Self>) {
        if self.look == look {
            return;
        }
        if (self.look.cursor, self.look.blink) != (look.cursor, look.blink) {
            self.term.set_cursor_default(look.cursor, look.blink);
        }
        self.cell_w = cell_width(window, &look.family, look.font_px);
        self.restyle |= self.look.palette != look.palette;
        self.look = look;
        self.rows.clear();
        cx.notify();
    }

    /// Run the blink timer only while a focused cursor is meant to blink, so
    /// an idle window costs nothing.
    fn sync_blink(&mut self, blink: bool, cx: &mut Context<Self>) {
        if blink == self._blink.is_some() {
            return;
        }
        self.blink_on = true;
        self._blink = blink.then(|| {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(BLINK).await;
                let alive = this.update(cx, |p, cx| {
                    p.blink_on = !p.blink_on || p.typed.elapsed() < BLINK;
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            })
        });
    }

    /// ⌘K: drop scrollback and have the shell redraw its prompt at the top.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.term.clear_scrollback();
        self.term.write(&b"\x0c"[..]);
        cx.notify();
    }

    /// ⌘F: open the find bar, or select its query if it is already open.
    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(bar) = &self.find {
            let field = bar.field.clone();
            field.update(cx, |f, cx| f.select_all(cx));
            window.focus(&field.read(cx).focus);
            return;
        }
        let field = cx.new(|cx| Field::new("Find", cx));
        let subs = [
            // Search as you type.
            cx.observe(&field, |p, field, cx| {
                let query = field.read(cx).text().to_string();
                p.search(query, cx);
            }),
            cx.subscribe_in(
                &field,
                window,
                |p, _, ev: &FieldEvent, window, cx| match ev {
                    FieldEvent::Submit(_) => p.find_step(!window.modifiers().shift, cx),
                    FieldEvent::Cancel => p.close_find(window, cx),
                    _ => {}
                },
            ),
        ];
        window.focus(&field.read(cx).focus);
        self.find = Some(FindBar {
            field,
            query: String::new(),
            found: true,
            _subs: subs,
        });
        cx.notify();
    }

    fn search(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(bar) = self.find.as_mut() else {
            return;
        };
        if bar.query != query {
            bar.found = self.term.search(&query) || query.is_empty();
            bar.query = query;
            cx.notify();
        }
    }

    /// Enter and ⌘G go up, to older output (the search starts at the
    /// bottom); with Shift, back down.
    fn find_step(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(bar) = self.find.as_mut().filter(|b| !b.query.is_empty()) {
            bar.found = self.term.search_step(up);
            cx.notify();
        }
    }

    /// Esc: highlights go, the view stays where the last match left it.
    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            self.term.search_clear();
            window.focus(&self.focus);
            cx.notify();
        }
    }

    fn render_find(&self, bar: &FindBar, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top(theme::u(theme::PANE_PAD_Y))
            .right(theme::u(theme::PANE_PAD_X))
            .w(theme::u(320.))
            .max_w(relative(1.))
            .flex()
            .items_center()
            .gap(theme::u(2.))
            .p(theme::u(4.))
            .rounded(theme::u(theme::CARD_RADIUS))
            .bg(rgb(theme::sidebar()))
            .border_1()
            .border_color(rgba(theme::hairline()))
            .cursor_default()
            // Clicks here are not grid clicks: no selection, and the field
            // keeps the focus it just took.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.emit(TerminalEvent::Focused);
                    cx.stop_propagation();
                }),
            )
            .child(bar.field.clone())
            .when(!bar.found, |d| {
                d.child(
                    div()
                        .flex_none()
                        .px(theme::u(6.))
                        .text_size(theme::u(12.))
                        .text_color(rgb(theme::dim()))
                        .child("No matches"),
                )
            })
            .child(
                icon_button("find-up", Icon::ChevronUp)
                    .on_click(cx.listener(|p, _, _, cx| p.find_step(true, cx))),
            )
            .child(
                icon_button("find-down", Icon::ChevronDown)
                    .on_click(cx.listener(|p, _, _, cx| p.find_step(false, cx))),
            )
            .child(
                icon_button("find-close", Icon::Close)
                    .on_click(cx.listener(|p, _, window, cx| p.close_find(window, cx))),
            )
    }

    /// Type `text`, then press enter as a separate write. Agent TUIs treat a
    /// newline arriving inside the same burst as pasted text, not a submit.
    pub fn send(&self, text: &str, enter: bool, cx: &mut Context<Self>) {
        self.term.write(text.as_bytes().to_vec());
        if enter {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(40))
                    .await;
                let _ = this.update(cx, |p, _| p.term.write(&b"\r"[..]));
            })
            .detach();
        }
    }

    /// The visible screen as plain text, trailing blanks trimmed.
    pub fn read_text(&mut self) -> String {
        self.term.snapshot(&self.look.palette, true);
        let mut lines: Vec<String> = self
            .term
            .last()
            .rows
            .iter()
            .map(|row| row_text(row).trim_end().to_string())
            .collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    fn cell_at(&self, pos: Point<Pixels>) -> (usize, usize, bool) {
        let x = (f32::from(pos.x - self.origin.x) / self.cell_w).max(0.);
        let y = (f32::from(pos.y - self.origin.y) / self.line_px()).max(0.);
        let rows = self.term.rows().max(1);
        let cols = self.term.columns().max(1);
        let col = (x as usize).min(cols - 1);
        ((y as usize).min(rows - 1), col, x.fract() > 0.5)
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Mid-composition the IME owns every key, Enter and Backspace included.
        // Keys the find field passes up are not for the shell either.
        if self.marked.is_some() || !self.focus.is_focused(window) {
            return;
        }
        self.typed = Instant::now();
        self.blink_on = true;
        let bytes = key_bytes(&ev.keystroke, self.term.app_cursor()).or_else(|| {
            self.look
                .option_meta
                .then(|| meta_bytes(&ev.keystroke))
                .flatten()
        });
        if let Some(bytes) = bytes {
            self.term.select_clear();
            self.term.write(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn focus_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        crate::browser::reclaim_keyboard(window);
        self.seen(cx);
        cx.emit(TerminalEvent::Focused);
    }

    /// The pane has been looked at: whatever it wanted you for is answered
    /// or at least known.
    pub fn seen(&mut self, cx: &mut Context<Self>) {
        if self.bell || self.alert.is_some() {
            self.bell = false;
            self.alert = None;
            cx.emit(TerminalEvent::Retitled);
        }
    }

    /// Whether the program says it is working, when it says: OSC 9;4
    /// progress if it reports any, else Claude Code's title, which turns
    /// `◐`/`◑` while it works and rests on `✳`. `None` when it does neither.
    pub fn activity(&self) -> Option<bool> {
        if let Some(state) = self.progress {
            return Some(state.is_some());
        }
        match self.title.chars().next() {
            Some('◐' | '◑') => Some(true),
            Some('✳') => Some(false),
            _ => None,
        }
    }

    /// Send a mouse event to the program if it asked for mouse input. Shift
    /// is the universal escape hatch: with it held, the mouse selects text.
    fn report(
        &mut self,
        button: Option<term::MouseButton>,
        action: term::MouseAction,
        pos: Point<Pixels>,
        m: &gpui::Modifiers,
    ) -> bool {
        if m.shift || !self.term.mouse_modes().report {
            return false;
        }
        let (row, col, _) = self.cell_at(pos);
        match self.term.mouse_report(button, action, row, col, mods(m)) {
            Some(bytes) => {
                self.term.write(bytes);
                true
            }
            None => false,
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_here(window, cx);
        if ev.modifiers.platform {
            let (row, col, _) = self.cell_at(ev.position);
            let line = self
                .term
                .last()
                .rows
                .get(row)
                .map(row_cols)
                .unwrap_or_default();
            if let Some(url) = url_at(&line, col) {
                cx.emit(TerminalEvent::OpenUrl(url));
            }
            return;
        }
        let button = term::MouseButton::Left;
        if self.report(
            Some(button),
            term::MouseAction::Press,
            ev.position,
            &ev.modifiers,
        ) {
            self.reporting = Some(button);
            return;
        }
        let (row, col, right) = self.cell_at(ev.position);
        match ev.click_count {
            2 => self
                .term
                .select_start(row, col, right, term::SelectMode::Word),
            3 => self
                .term
                .select_start(row, col, right, term::SelectMode::Line),
            _ => {
                self.term.select_clear();
                self.press = Some((row, col, right));
            }
        }
        cx.notify();
    }

    fn on_other_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_here(window, cx);
        // Taken by the find bar's field.
        if window.default_prevented() {
            return;
        }
        let button = match ev.button {
            MouseButton::Right => term::MouseButton::Right,
            MouseButton::Middle => term::MouseButton::Middle,
            _ => return,
        };
        if self.report(
            Some(button),
            term::MouseAction::Press,
            ev.position,
            &ev.modifiers,
        ) {
            self.reporting = Some(button);
        } else if button == term::MouseButton::Right {
            self.open_menu(ev.position, window, cx);
        }
    }

    /// The right-click menu. A program that takes the mouse (vim, htop)
    /// gets the click instead; Shift brings the menu back, as it does
    /// selection.
    fn open_menu(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (row, col, _) = self.cell_at(at);
        let line = self
            .term
            .last()
            .rows
            .get(row)
            .map(row_cols)
            .unwrap_or_default();
        let this = cx.entity().downgrade();
        let f = self.focus.clone();
        let mut entries: Vec<Entry> = Vec::new();
        if let Some(url) = url_at(&line, col) {
            let open = url.clone();
            let open_in = this.clone();
            entries.push(
                Item::new("Open Link", move |_, cx| {
                    let url = open.clone();
                    let _ = open_in.update(cx, |_, cx| cx.emit(TerminalEvent::OpenUrl(url)));
                })
                .icon(Icon::ArrowUpRight)
                .into(),
            );
            entries.push(
                Item::new("Copy Link", move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                })
                .icon(Icon::Link)
                .into(),
            );
            entries.push(Entry::Separator);
        }
        let selected = self.term.selection_text().is_some();
        entries.extend([
            Item::new("Copy", menu::send(f.clone(), Copy))
                .icon(Icon::Copy)
                .shortcut("⌘C")
                .disabled(!selected)
                .into(),
            Item::new("Paste", menu::send(f.clone(), Paste))
                .icon(Icon::Clipboard)
                .shortcut("⌘V")
                .into(),
            Item::new("Select All", menu::send(f.clone(), SelectAll))
                .icon(Icon::SelectAll)
                .shortcut("⌘A")
                .into(),
            Entry::Separator,
            Item::new("Find…", menu::send(f.clone(), Find))
                .icon(Icon::Search)
                .shortcut("⌘F")
                .into(),
            Item::new("Clear", menu::send(f.clone(), ClearTerminal))
                .icon(Icon::Eraser)
                .shortcut("⌘K")
                .into(),
            Entry::Separator,
            Item::new("Split Right", menu::send(f.clone(), SplitRight))
                .icon(Icon::Split)
                .shortcut("⌘D")
                .into(),
            Item::new("Split Down", menu::send(f.clone(), SplitDown))
                .icon(Icon::SplitDown)
                .shortcut("⇧⌘D")
                .into(),
            Entry::Separator,
            Item::new("Close", menu::send(f.clone(), ClosePane))
                .icon(Icon::Close)
                .shortcut("⌘W")
                .into(),
        ]);
        self.menu = Some(menu::open(
            entries,
            at,
            Some(f),
            window,
            cx,
            |p: &mut Self, _, _| p.menu = None,
        ));
        cx.notify();
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.press = None;
        if let Some(button) = self.reporting.take() {
            self.report(
                Some(button),
                term::MouseAction::Release,
                ev.position,
                &ev.modifiers,
            );
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (row, col, right) = self.cell_at(ev.position);
        // A button let go where this pane never heard it (over the sidebar,
        // another pane, the menu bar) left a press open. The program would
        // take every later move for a drag, and Claude Code then selects as
        // you scroll until the whole screen is grey. Close it now.
        if let (Some(button), None) = (self.reporting, ev.pressed_button) {
            self.reporting = None;
            self.report(
                Some(button),
                term::MouseAction::Release,
                ev.position,
                &ev.modifiers,
            );
        }
        if self.reporting.is_some()
            || (ev.pressed_button.is_none() && self.term.mouse_modes().motion)
        {
            if self.last_motion != Some((row, col)) {
                self.last_motion = Some((row, col));
                self.report(
                    self.reporting,
                    term::MouseAction::Motion,
                    ev.position,
                    &ev.modifiers,
                );
            }
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.press = None;
            return;
        }
        if let Some((r, c, side)) = self.press.take() {
            self.term.select_start(r, c, side, term::SelectMode::Char);
        }
        self.term.select_to(row, col, right);
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let lh = self.line_px();
        let delta_px = match ev.delta {
            ScrollDelta::Lines(l) => l.y * lh,
            ScrollDelta::Pixels(d) => f32::from(d.y),
        };
        let lines = whole_lines(&mut self.scroll_acc, delta_px, lh);
        if lines == 0 {
            return;
        }
        let (row, col, _) = self.cell_at(ev.position);
        // Full-screen programs scroll themselves (wheel reports, or arrows in
        // the alternate screen); everything else scrolls our scrollback.
        let report = if ev.modifiers.shift {
            None
        } else {
            self.term
                .scroll_report(lines, row, col, mods(&ev.modifiers))
        };
        match report {
            // The program scrolls its own content now; a highlight left on
            // cells it is about to redraw would only mislead.
            Some(bytes) => {
                self.term.select_clear();
                self.term.write(bytes);
            }
            None => self.term.scroll(lines),
        }
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.term.selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Text pastes as text. Files copied in Finder paste as their paths, and
    /// an image (a screenshot, "Copy Image") is kept as a file and pastes as
    /// its path, which Claude Code and Codex pick up as an attachment.
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        match pasteboard::read_for_terminal() {
            Some(Pasted::Files(paths)) => self.paste_text(&pasteboard::shell_paths(&paths)),
            Some(Pasted::Image(bytes, ext)) => {
                cx.spawn(async move |this, cx| {
                    let saved = cx
                        .background_executor()
                        .spawn(async move { pasteboard::save_image(&bytes, ext) })
                        .await;
                    match saved {
                        Ok(path) => {
                            let _ = this
                                .update(cx, |t, _| t.paste_text(&pasteboard::shell_paths(&[path])));
                        }
                        Err(e) => tracing::warn!("could not keep the pasted image: {e}"),
                    }
                })
                .detach();
            }
            None => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.paste_text(&text);
                }
            }
        }
    }

    /// What is selected, if anything.
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_text()
    }

    /// Type `text` in as a paste: a shell or TUI takes it whole, and a
    /// newline in it does not run anything.
    pub fn paste_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if self.term.bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        self.term.write(bytes.into_bytes());
    }
}

/// Add a scroll delta to the carried remainder and take out whole lines.
/// Trackpads deliver a few pixels per event; rounding each one alone (as the
/// first version did) turned every gesture into zero lines.
fn whole_lines(acc: &mut f32, delta_px: f32, line_px: f32) -> i32 {
    *acc += delta_px;
    let lines = (*acc / line_px).trunc();
    *acc -= lines * line_px;
    lines as i32
}

fn mods(m: &gpui::Modifiers) -> term::Mods {
    term::Mods {
        shift: m.shift,
        alt: m.alt,
        ctrl: m.control,
    }
}

impl EntityInputHandler for TerminalPane {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let n = self.marked.as_ref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection {
            range: n..n,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if !text.is_empty() {
            self.typed = Instant::now();
            self.blink_on = true;
            self.term.select_clear();
            self.term.write(text.as_bytes().to_vec());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        cx.notify();
    }

    /// Where the IME should put its candidate window: at the cursor.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (y, x) = self.term.last().cursor?;
        let lh = self.line_px();
        Some(Bounds::new(
            point(
                self.origin.x + px(x as f32 * self.cell_w),
                self.origin.y + px(y as f32 * lh),
            ),
            size(px(self.cell_w), px(lh)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let restyle = std::mem::take(&mut self.restyle);
        self.term.snapshot(&self.look.palette, restyle);
        // An open menu holds the keyboard for this pane; the cursor should
        // not go hollow under it.
        let focused = self.focus.is_focused(window) || self.menu.is_some();
        let blink = focused && self.look.blink && self.term.last().cursor_blink;
        self.sync_blink(blink, cx);
        // Full-screen TUIs (OpenCode, Claude Code) paint their own background.
        // Extending the top-left cell's colour into the padding, as Ghostty
        // does, keeps that from showing as a frame around the app.
        let pad = self
            .term
            .last()
            .rows
            .first()
            .and_then(|r| r.runs.first())
            .and_then(|r| r.style.bg)
            .unwrap_or(self.look.palette.bg);
        let find = self.find.as_ref().map(|bar| self.render_find(bar, cx));
        div()
            .track_focus(&self.focus)
            .key_context("Terminal")
            .relative()
            .size_full()
            .px(theme::u(theme::PANE_PAD_X))
            .py(theme::u(theme::PANE_PAD_Y))
            .bg(rgb(pad))
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(|p, _: &SelectAll, _, cx| {
                p.term.select_all();
                cx.notify();
            }))
            .on_action(cx.listener(|p, _: &FindNext, _, cx| p.find_step(true, cx)))
            .on_action(cx.listener(|p, _: &FindPrevious, _, cx| p.find_step(false, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_other_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_other_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            // Released outside the pane: still a release, for the program
            // and for a selection being dragged.
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            // Files dropped from Finder type their paths, as in Terminal; an
            // agent CLI turns an image path into an attachment.
            .on_drop(cx.listener(|p, paths: &ExternalPaths, window, cx| {
                p.paste_text(&pasteboard::shell_paths(paths.paths()));
                window.focus(&p.focus);
                cx.notify();
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(theme::raised())))
            .child(Grid {
                pane: cx.entity(),
                focus: self.focus.clone(),
                focused,
            })
            .children(find)
            .children(self.menu.as_ref().map(menu::Open::render))
    }
}

/// The painted grid. Reads the pane's latest snapshot; owns nothing.
struct Grid {
    pane: Entity<TerminalPane>,
    focus: FocusHandle,
    focused: bool,
}

struct Prepared {
    glyphs: Vec<(Point<Pixels>, ShapedLine)>,
    fills: Vec<(Bounds<Pixels>, u32)>,
    cursor: Option<(Bounds<Pixels>, term::CursorShape)>,
    /// The character under a block cursor, redrawn in the background colour.
    under_cursor: Option<(Point<Pixels>, ShapedLine)>,
    /// IME composition, drawn over the cursor cell.
    marked: Option<(Point<Pixels>, ShapedLine)>,
    line_px: f32,
    cursor_color: u32,
    bg: u32,
}

impl IntoElement for Grid {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Characters guaranteed to be exactly one Menlo cell wide.
fn on_grid(ch: char) -> bool {
    matches!(ch as u32, 0x20..=0x7e | 0xa0..=0x17f | 0x2500..=0x259f)
}

/// One screen row, shaped: glyph runs at their column and background fills
/// as (column, width in columns, colour). Positions are in columns so the
/// row can be reused wherever the grid is painted.
#[derive(Default)]
struct RowCache {
    key: u64,
    valid: bool,
    glyphs: Vec<(usize, ShapedLine)>,
    fills: Vec<(usize, usize, u32)>,
}

/// What a row looks like, as a hash: text, styles and the font metrics.
fn row_key(row: &term::Row, font_px: f32, cell_w: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    font_px.to_bits().hash(&mut h);
    cell_w.to_bits().hash(&mut h);
    for run in &row.runs {
        run.text.hash(&mut h);
        let s = &run.style;
        (s.fg, s.bg, s.bold, s.italic, s.underline).hash(&mut h);
    }
    h.finish()
}

fn text_run(font: gpui::Font, color: u32, underline: bool, len: usize) -> TextRun {
    TextRun {
        len,
        font,
        color: rgb(color).into(),
        background_color: None,
        underline: underline.then(|| UnderlineStyle {
            thickness: px(1.),
            color: Some(rgb(color).into()),
            wavy: false,
        }),
        strikethrough: None,
    }
}

/// Shape one row. A stretch of plain characters is one shaped line; any
/// other glyph is shaped alone at its own column (see the module docs).
fn shape_row(
    row: &term::Row,
    regular: &gpui::Font,
    size_px: Pixels,
    ts: &std::sync::Arc<gpui::WindowTextSystem>,
) -> RowCache {
    let mut out = RowCache {
        valid: true,
        ..Default::default()
    };
    let mut col = 0usize;
    for run in &row.runs {
        let s = &run.style;
        let start = col;
        let mut f = regular.clone();
        if s.bold {
            f.weight = FontWeight::BOLD;
        }
        if s.italic {
            f.style = FontStyle::Italic;
        }
        let mut push = |text: &mut String, at: usize| {
            if !text.is_empty() && (s.underline || !text.trim().is_empty()) {
                let len = text.len();
                let line = ts.shape_line(
                    SharedString::from(std::mem::take(text)),
                    size_px,
                    &[text_run(f.clone(), s.fg, s.underline, len)],
                    None,
                );
                out.glyphs.push((at, line));
            }
            text.clear();
        };
        let mut seg = String::new();
        let mut seg_col = col;
        for ch in run.text.chars() {
            if ch == '\0' {
                // Right half of a wide character: a column, no glyph.
                push(&mut seg, seg_col);
            } else if on_grid(ch) {
                if seg.is_empty() {
                    seg_col = col;
                }
                seg.push(ch);
            } else {
                push(&mut seg, seg_col);
                let mut solo = ch.to_string();
                push(&mut solo, col);
            }
            col += 1;
        }
        push(&mut seg, seg_col);
        if let Some(bg) = s.bg {
            out.fills.push((start, col - start, bg));
        }
    }
    out
}

impl Element for Grid {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepared {
        let regular = font(self.pane.read(cx).look.family.clone());
        let ts = window.text_system().clone();
        // Rows are shaped once and reused while their content is unchanged:
        // typing or a spinner touches one or two rows a frame, not all fifty.
        let (cell_w, lh, font_px, glyphs, fills) = self.pane.update(cx, |p, cx| {
            p.origin = bounds.origin;
            let lh = p.line_px();
            let cols = (f32::from(bounds.size.width) / p.cell_w).floor().max(2.) as u16;
            let rows = (f32::from(bounds.size.height) / lh).floor().max(2.) as u16;
            if p.term.resize(cols, rows, p.cell_w as u16, lh as u16) {
                cx.notify();
            }
            let (cell_w, font_px) = (p.cell_w, p.look.font_px);
            let size_px = px(font_px);
            let snap = p.term.last();
            let cache = &mut p.rows;
            cache.resize_with(snap.rows.len(), RowCache::default);
            let mut glyphs = Vec::new();
            let mut fills = Vec::new();
            for (y, row) in snap.rows.iter().enumerate() {
                let key = row_key(row, font_px, cell_w);
                let entry = &mut cache[y];
                if entry.key != key || !entry.valid {
                    *entry = shape_row(row, &regular, size_px, &ts);
                    entry.key = key;
                }
                let top = bounds.origin.y + px(y as f32 * lh);
                let x_at = |col: usize| bounds.origin.x + px(col as f32 * cell_w);
                for (col, line) in &entry.glyphs {
                    glyphs.push((point(x_at(*col), top), line.clone()));
                }
                for (col, n, color) in &entry.fills {
                    fills.push((
                        Bounds::new(point(x_at(*col), top), size(px(*n as f32 * cell_w), px(lh))),
                        *color,
                    ));
                }
            }
            (cell_w, lh, font_px, glyphs, fills)
        });

        let size_px = px(font_px);
        let pane = self.pane.read(cx);
        let snap = pane.term.last();
        let shape = |text: String, f: gpui::Font, color: u32, underline: bool| {
            let len = text.len();
            ts.shape_line(
                SharedString::from(text),
                size_px,
                &[text_run(f, color, underline, len)],
                None,
            )
        };

        let cursor_at = snap.cursor.map(|(y, x)| {
            Bounds::new(
                point(
                    bounds.origin.x + px(x as f32 * cell_w),
                    bounds.origin.y + px(y as f32 * lh),
                ),
                size(px(cell_w), px(lh)),
            )
        });
        let (bg, fg, cursor_color) = (
            pane.look.palette.bg,
            pane.look.palette.fg,
            pane.look.cursor_color,
        );
        let cursor = cursor_at
            .filter(|_| pane.blink_on)
            .map(|b| (b, snap.cursor_shape));
        let under_cursor = match (snap.cursor, snap.cursor_shape, self.focused) {
            (Some((y, x)), term::CursorShape::Block, true) if pane.blink_on => snap
                .rows
                .get(y)
                .and_then(|r| row_cols(r).chars().nth(x))
                .filter(|c| *c != '\0' && !c.is_whitespace())
                .zip(cursor_at)
                .map(|(c, b)| (b.origin, shape(c.to_string(), regular.clone(), bg, false))),
            _ => None,
        };
        let marked = pane
            .marked
            .clone()
            .zip(cursor_at)
            .map(|(m, b)| (b.origin, shape(m, regular.clone(), fg, true)));
        Prepared {
            glyphs,
            fills,
            cursor,
            under_cursor,
            marked,
            line_px: lh,
            cursor_color,
            bg,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prep: &mut Prepared,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.pane.clone()),
            cx,
        );
        let lh = px(prep.line_px);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for (b, color) in &prep.fills {
                window.paint_quad(fill(*b, rgb(*color)));
            }
            for (origin, line) in &prep.glyphs {
                let _ = line.paint(*origin, lh, window, cx);
            }
            if let Some((c, shape)) = prep.cursor {
                let bar = |w: f32, h: f32, bottom: bool| {
                    let y = if bottom {
                        c.origin.y + c.size.height - px(h)
                    } else {
                        c.origin.y
                    };
                    Bounds::new(
                        point(c.origin.x, y),
                        size(
                            if w > 0. { px(w) } else { c.size.width },
                            if h > 0. { px(h) } else { c.size.height },
                        ),
                    )
                };
                match (shape, self.focused) {
                    (term::CursorShape::Block, true) => {
                        window.paint_quad(fill(c, rgb(prep.cursor_color)));
                        if let Some((origin, glyph)) = &prep.under_cursor {
                            let _ = glyph.paint(*origin, lh, window, cx);
                        }
                    }
                    (term::CursorShape::Beam, _) => {
                        window.paint_quad(fill(bar(2., 0., false), rgb(prep.cursor_color)))
                    }
                    (term::CursorShape::Underline, _) => {
                        window.paint_quad(fill(bar(0., 2., true), rgb(prep.cursor_color)))
                    }
                    (term::CursorShape::Block, false) => window.paint_quad(outline(
                        c,
                        rgba(theme::alpha(theme::text(), 0x80)),
                        BorderStyle::Solid,
                    )),
                }
            }
            if let Some((origin, glyph)) = &prep.marked {
                let w = glyph.width;
                window.paint_quad(fill(Bounds::new(*origin, size(w, lh)), rgb(prep.bg)));
                let _ = glyph.paint(*origin, lh, window, cx);
            }
        });
    }
}

/// A row with one char per column ('\0' for the right half of a wide char),
/// for column arithmetic.
fn row_cols(row: &term::Row) -> String {
    row.runs.iter().map(|r| r.text.as_str()).collect()
}

/// A row as text, for copying and reading.
fn row_text(row: &term::Row) -> String {
    row.runs
        .iter()
        .flat_map(|r| r.text.chars())
        .filter(|&c| c != '\0')
        .collect()
}

fn cell_width(window: &Window, family: &SharedString, font_px: f32) -> f32 {
    let ts = window.text_system();
    let id = ts.resolve_font(&font(family.clone()));
    ts.advance(id, px(font_px), 'm')
        .map(|s| f32::from(s.width))
        .unwrap_or(font_px * 0.6)
}

/// The http(s) URL covering character column `col`, if any. `line` has one
/// char per column.
fn url_at(line: &str, col: usize) -> Option<String> {
    let mut from = 0;
    while let Some(i) = line[from..].find("http").map(|i| i + from) {
        let rest = &line[i..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            from = i + 4;
            continue;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | '\0'))
            .map_or(line.len(), |e| i + e);
        let url = line[i..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']);
        let start_col = line[..i].chars().count();
        let end_col = start_col + url.chars().count();
        if (start_col..end_col).contains(&col) {
            return Some(url.to_string());
        }
        from = end.max(i + 1);
    }
    None
}

/// Translate a keystroke into an escape sequence. `None` means "not a
/// terminal key": ⌘ shortcuts go to the app, and printable text goes to the
/// input handler (so the IME, dead keys and ⌥-characters work).
fn key_bytes(ks: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform {
        return None;
    }
    let arrow = |c: u8| -> Vec<u8> {
        match (m.alt, c) {
            // ⌥←/⌥→ move by word, as Terminal.app and iTerm do.
            (true, b'D') => b"\x1bb".to_vec(),
            (true, b'C') => b"\x1bf".to_vec(),
            _ if app_cursor => vec![0x1b, b'O', c],
            _ => vec![0x1b, b'[', c],
        }
    };
    let fixed: Option<&[u8]> = match ks.key.as_str() {
        "enter" => Some(b"\r"),
        "backspace" if m.alt => Some(b"\x1b\x7f"),
        "backspace" => Some(b"\x7f"),
        "tab" if m.shift => Some(b"\x1b[Z"),
        "tab" => Some(b"\t"),
        "escape" => Some(b"\x1b"),
        "home" => Some(b"\x1b[H"),
        "end" => Some(b"\x1b[F"),
        "pageup" => Some(b"\x1b[5~"),
        "pagedown" => Some(b"\x1b[6~"),
        "delete" => Some(b"\x1b[3~"),
        "f1" => Some(b"\x1bOP"),
        "f2" => Some(b"\x1bOQ"),
        "f3" => Some(b"\x1bOR"),
        "f4" => Some(b"\x1bOS"),
        "f5" => Some(b"\x1b[15~"),
        "f6" => Some(b"\x1b[17~"),
        "f7" => Some(b"\x1b[18~"),
        "f8" => Some(b"\x1b[19~"),
        "f9" => Some(b"\x1b[20~"),
        "f10" => Some(b"\x1b[21~"),
        "f11" => Some(b"\x1b[23~"),
        "f12" => Some(b"\x1b[24~"),
        _ => None,
    };
    if let Some(b) = fixed {
        return Some(b.to_vec());
    }
    match ks.key.as_str() {
        "up" => return Some(arrow(b'A')),
        "down" => return Some(arrow(b'B')),
        "right" => return Some(arrow(b'C')),
        "left" => return Some(arrow(b'D')),
        _ => {}
    }
    if m.control {
        let b = match ks.key.as_bytes() {
            [c @ b'a'..=b'z'] => c - b'a' + 1,
            b"space" | b"@" | b"2" => 0,
            b"[" | b"3" => 0x1b,
            b"\\" | b"4" => 0x1c,
            b"]" | b"5" => 0x1d,
            b"^" | b"6" => 0x1e,
            b"_" | b"-" | b"7" => 0x1f,
            _ => return None,
        };
        return Some(if m.alt { vec![0x1b, b] } else { vec![b] });
    }
    None
}

/// Option-key as Meta: Esc then the key itself, so Option-b / -f / -d reach
/// readline, Emacs and TUIs as word motions instead of typing ∫ ƒ ∂.
fn meta_bytes(ks: &Keystroke) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if !m.alt || m.control || m.platform {
        return None;
    }
    let mut chars = ks.key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if !c.is_ascii_graphic() {
        return None;
    }
    let c = if m.shift { c.to_ascii_uppercase() } else { c };
    Some(vec![0x1b, c as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn ks(key: &str, ch: Option<&str>, m: Modifiers) -> Keystroke {
        Keystroke {
            modifiers: m,
            key: key.into(),
            key_char: ch.map(Into::into),
        }
    }

    #[test]
    fn keys_map_to_terminal_bytes() {
        let none = Modifiers::default();
        let ctrl = Modifiers {
            control: true,
            ..none
        };
        let alt = Modifiers { alt: true, ..none };
        let cmd = Modifiers {
            platform: true,
            ..none
        };

        assert_eq!(key_bytes(&ks("c", None, ctrl), false), Some(vec![3]));
        assert_eq!(key_bytes(&ks("space", None, ctrl), false), Some(vec![0]));
        assert_eq!(
            key_bytes(&ks("up", None, none), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(&ks("up", None, none), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(&ks("left", None, alt), false),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            key_bytes(&ks("enter", None, none), false),
            Some(b"\r".to_vec())
        );
        assert_eq!(key_bytes(&ks("v", None, cmd), false), None);
        // Text belongs to the input handler, so the IME and ⌥-characters work.
        assert_eq!(key_bytes(&ks("a", Some("a"), none), false), None);
        assert_eq!(key_bytes(&ks("s", Some("ß"), alt), false), None);
    }

    #[test]
    fn urls_are_found_under_the_cursor_column() {
        let line = "  ➜  Local:   http://localhost:5173/ (ready)";
        let at = line.chars().position(|c| c == 'h').unwrap();
        assert_eq!(
            url_at(line, at + 3).as_deref(),
            Some("http://localhost:5173/")
        );
        assert_eq!(url_at(line, 1), None);
        assert_eq!(
            url_at("see https://a.dev/x, then", 6).as_deref(),
            Some("https://a.dev/x")
        );
        assert_eq!(url_at("httpx nothing", 2), None);
        assert_eq!(
            url_at("(https://b.io/y)", 3).as_deref(),
            Some("https://b.io/y")
        );
        // A wide char before the URL occupies two columns ('\0' is its tail).
        assert_eq!(
            url_at("中\0 https://c.io", 5).as_deref(),
            Some("https://c.io")
        );
    }

    #[test]
    fn plain_text_stays_on_one_shaped_run_and_symbols_do_not() {
        assert!("ls -la ~/deck ── │ é".chars().all(on_grid));
        assert!(!on_grid('⏺') && !on_grid('中') && !on_grid('😀'));
    }

    #[test]
    fn trackpad_pixels_add_up_to_lines_in_both_directions() {
        let mut acc = 0.;
        let lines: i32 = (0..30).map(|_| whole_lines(&mut acc, 4., 18.)).sum();
        assert_eq!(lines, 6); // 120px over 18px lines
        assert!((acc - 12.).abs() < 1e-3);
        let back: i32 = (0..30).map(|_| whole_lines(&mut acc, -4., 18.)).sum();
        assert_eq!(back, -6);
        assert!(acc.abs() < 1e-3);
        // A mouse wheel notch arrives as whole lines.
        assert_eq!(whole_lines(&mut acc, 3. * 18., 18.), 3);
    }

    #[test]
    fn option_as_meta_sends_escape_and_the_key() {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            meta_bytes(&ks("b", Some("∫"), alt)),
            Some(b"\x1bb".to_vec())
        );
        let alt_shift = Modifiers { shift: true, ..alt };
        assert_eq!(
            meta_bytes(&ks("f", Some("Ï"), alt_shift)),
            Some(b"\x1bF".to_vec())
        );
        assert_eq!(meta_bytes(&ks("b", Some("b"), Modifiers::default())), None);
        let cmd_alt = Modifiers {
            platform: true,
            ..alt
        };
        assert_eq!(meta_bytes(&ks("b", None, cmd_alt)), None);
        assert_eq!(meta_bytes(&ks("space", None, alt)), None);
    }

    #[test]
    fn line_height_matches_the_default_metrics() {
        // 13px Menlo on an 18px line is the classic Terminal.app metric.
        assert_eq!(line_height(13., crate::settings::DEFAULT_LINE_HEIGHT), 18.);
        assert_eq!(
            line_height(theme::FONT_PX, crate::settings::DEFAULT_LINE_HEIGHT),
            18.
        );
        assert_eq!(line_height(13., 1.6), 21.);
    }
}
