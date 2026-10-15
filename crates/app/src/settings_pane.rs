//! The Settings tab (⌘,): every preference deck keeps, applied the moment it
//! changes. The pane edits its own copy and hands each change to the
//! workspace, which saves it and pushes it to every pane.

use crate::icon::{icon, Icon};
use crate::settings::{
    self, Access, Cursor, Scheme, Search, Settings, FONT_PX_RANGE, LINE_HEIGHT_RANGE, ZOOMS,
};
use crate::theme::{self, u};
use gpui::prelude::*;
use gpui::{
    div, ease_in_out, relative, rgb, rgba, Animation, AnimationExt, AnyElement, ClickEvent,
    Context, Div, ElementId, EventEmitter, FocusHandle, FontWeight, SharedString, Stateful, Window,
};
use std::time::Duration;

pub enum SettingsEvent {
    Changed(Settings),
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
        Self {
            fonts: monospace_families(&installed, &settings.font_family),
            settings,
            focus: cx.focus_handle(),
            agents,
            flip: None,
            page: Page::Appearance,
            scroll: gpui::ScrollHandle::new(),
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Appearance,
    Terminal,
    Browser,
    Agents,
}

impl Page {
    const ALL: [Page; 5] = [
        Page::General,
        Page::Appearance,
        Page::Terminal,
        Page::Browser,
        Page::Agents,
    ];

    fn name(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::Terminal => "Terminal",
            Page::Browser => "Browser",
            Page::Agents => "Agents",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::General => Icon::Settings,
            Page::Appearance => Icon::PanelRight,
            Page::Terminal => Icon::Terminal,
            Page::Browser => Icon::Globe,
            Page::Agents => Icon::Claude,
        }
    }
}

/// The page column's width, and the inset every row shares.
const PAGE_W: f32 = 560.;
const ROW_X: f32 = 14.;

/// One row of a group: the name on the leading edge, its control on the
/// trailing one, as System Settings lays them out.
fn row(label: &'static str, control: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(u(16.))
        .min_h(u(44.))
        .px(u(ROW_X))
        .py(u(7.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(u(13.))
                .text_color(rgb(theme::text()))
                .child(label),
        )
        .child(control)
}

/// A row whose control needs the width: the name above, the choices below.
fn wide(label: &'static str, control: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(10.))
        .px(u(ROW_X))
        .pt(u(12.))
        .pb(u(14.))
        .child(
            div()
                .text_size(u(13.))
                .text_color(rgb(theme::text()))
                .child(label),
        )
        .child(control)
}

/// An inset group: rows on one raised surface, a hairline between them
/// that starts where the text does, and a note under the group saying what
/// it does rather than repeating it in every row.
fn group(heading: Option<&'static str>, rows: Vec<Div>, note: Option<SharedString>) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(u(10.))
        .bg(rgb(theme::raised()))
        .border_1()
        .border_color(rgba(theme::hairline()));
    for (ix, r) in rows.into_iter().enumerate() {
        if ix > 0 {
            card = card.child(div().h(u(1.)).ml(u(ROW_X)).bg(rgba(theme::hairline())));
        }
        card = card.child(r);
    }
    div()
        .flex()
        .flex_col()
        .gap(u(6.))
        .children(heading.map(|h| {
            div()
                .pl(u(ROW_X))
                .text_size(u(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::text_2()))
                .child(h)
        }))
        .child(card)
        .children(note.map(|n| {
            div()
                .px(u(ROW_X))
                .text_size(u(11.))
                .line_height(u(15.))
                .text_color(rgb(theme::dim()))
                .child(n)
        }))
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
        .px(u(ROW_X))
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

/// A group of choices: its first and last rows round with the card.
fn choices(
    heading: Option<&'static str>,
    rows: Vec<Stateful<Div>>,
    note: Option<SharedString>,
) -> Div {
    let n = rows.len();
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(ix, r)| {
            div().child(
                r.when(ix == 0, |d| d.rounded_t(u(9.)))
                    .when(ix + 1 == n, |d| d.rounded_b(u(9.))),
            )
        })
        .collect();
    group(heading, rows, note)
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

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut col = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(u(2.))
            .w(u(196.))
            .h_full()
            .pt(u(20.))
            .px(u(10.))
            .bg(rgb(theme::sidebar()))
            .border_r_1()
            .border_color(rgba(theme::hairline()))
            .child(
                div()
                    .pl(u(8.))
                    .pb(u(10.))
                    .text_size(u(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::dim()))
                    .child("Settings"),
            );
        for (ix, page) in Page::ALL.iter().copied().enumerate() {
            let on = page == self.page;
            col = col.child(
                div()
                    .id(ElementId::NamedInteger("page".into(), ix as u64))
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .h(u(32.))
                    .px(u(8.))
                    .rounded(u(theme::ROW_RADIUS))
                    .text_size(u(13.))
                    .cursor_pointer()
                    .when(on, |d| {
                        d.bg(rgb(theme::surface())).text_color(rgb(theme::text()))
                    })
                    .when(!on, |d| {
                        d.text_color(rgb(theme::text_2()))
                            .hover(|s| s.bg(rgb(theme::row_hover())))
                    })
                    // The mark in a rounded tile, as System Settings has it,
                    // in the theme's monochrome.
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(u(22.))
                            .rounded(u(6.))
                            .bg(rgb(if on { theme::text() } else { theme::surface() }))
                            .child(icon(
                                page.icon(),
                                13.,
                                if on { theme::pane() } else { theme::text_2() },
                            )),
                    )
                    .child(page.name())
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.page = page;
                        p.scroll.set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                        cx.notify();
                    })),
            );
        }
        col.into_any_element()
    }

    fn page_general(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let path = settings::path();
        let shown = tilde(&path.display().to_string());
        let reveal = button("reveal", "Show in Finder").on_click(move |_, _, _| {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(&path)
                .spawn();
        });
        let reset = button("defaults", "Restore Defaults")
            .on_click(self.on(|s| *s = Settings::default(), cx));
        vec![
            group(
                None,
                vec![row("Settings file", reveal.into_any_element())],
                Some(format!("Every change is saved to {shown} the moment you make it.").into()),
            ),
            group(
                None,
                vec![row("Restore defaults", reset.into_any_element())],
                Some("Puts every setting back the way deck ships. Your tabs and threads are not touched.".into()),
            ),
        ]
    }

    fn page_appearance(&self, window: &Window, cx: &mut Context<Self>) -> Vec<Div> {
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
                None,
                vec![wide("Theme", self.themes(window, cx))],
                Some("The whole window: sidebar, panes, chats and menus, and the appearance web pages and scrollbars take. System follows macOS.".into()),
            ),
            group(
                None,
                vec![row("Zoom", zoom)],
                Some("One size for everything. ⌘+ and ⌘− step it, ⌘0 puts it back.".into()),
            ),
            group(
                Some("Terminal colours"),
                vec![wide("Palette", self.schemes(cx))],
                Some("What terminals and agent TUIs draw with. Match window uses the theme's own.".into()),
            ),
        ]
    }

    fn page_terminal(&self, cx: &mut Context<Self>) -> Vec<Div> {
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
                Some("Text"),
                vec![
                    wide("Font", self.fonts(cx)),
                    row("Size", font_px),
                    row("Line height", line),
                ],
                Some("Monospace faces installed on this Mac. Size is at 100% zoom.".into()),
            ),
            group(
                Some("Cursor"),
                vec![
                    row(
                        "Shape",
                        self.segmented("cursor", &cursors, s.cursor, |s, c| s.cursor = c, cx),
                    ),
                    row(
                        "Blink",
                        self.toggle("blink", s.cursor_blink, |s, on| s.cursor_blink = on, cx),
                    ),
                ],
                Some("Until a program asks for a shape of its own. Blinking only while the terminal has focus.".into()),
            ),
            group(
                Some("Keyboard"),
                vec![row(
                    "Use ⌥ as Meta",
                    self.toggle("meta", s.option_as_meta, |s, on| s.option_as_meta = on, cx),
                )],
                Some("⌥B and ⌥F jump by word in the shell, instead of typing ∫ and ƒ.".into()),
            ),
        ]
    }

    fn page_browser(&self, cx: &mut Context<Self>) -> Vec<Div> {
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
            Some("Search engine"),
            rows,
            Some("Anything typed in the address bar that is not an address is searched for here, and suggestions come from it too.".into()),
        )]
    }

    fn page_agents(&self, cx: &mut Context<Self>) -> Vec<Div> {
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
                Some("New chats start with"),
                start,
                (self.agents.is_empty()).then(|| "Detecting agents…".into()),
            ),
            choices(
                Some("Starting access"),
                access,
                Some(
                    "What a new thread may do without asking. Each thread's chip still changes it."
                        .into(),
                ),
            ),
            group(
                Some("Prompt rewriting"),
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
                Some("Recovery"),
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
                Some("Notifications"),
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
        let groups = match self.page {
            Page::General => self.page_general(cx),
            Page::Appearance => self.page_appearance(window, cx),
            Page::Terminal => self.page_terminal(cx),
            Page::Browser => self.page_browser(cx),
            Page::Agents => self.page_agents(cx),
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap(u(24.))
            .w(u(PAGE_W))
            .max_w(relative(1.))
            .px(u(24.))
            .pt(u(28.))
            .pb(u(48.))
            .child(
                div()
                    .px(u(2.))
                    .text_size(u(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::text()))
                    .child(self.page.name()),
            );
        for g in groups {
            column = column.child(g);
        }

        div()
            .id("settings")
            .track_focus(&self.focus)
            .key_context("Settings")
            .size_full()
            .flex()
            .bg(rgb(theme::pane()))
            .child(self.sidebar(cx))
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().flex().justify_center().child(column)),
            )
    }
}

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
