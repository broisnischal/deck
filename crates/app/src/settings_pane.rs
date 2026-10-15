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
                .text_color(rgb(if live { theme::TEXT } else { theme::DIM }))
                .when(live, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(rgb(theme::SURFACE_HOVER)))
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
            .bg(rgb(theme::SURFACE))
            .child(minus)
            .child(
                div()
                    .min_w(u(56.))
                    .flex()
                    .justify_center()
                    .text_size(u(13.))
                    .text_color(rgb(theme::TEXT))
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
            .bg(rgb(theme::SURFACE));
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
                        d.bg(rgb(theme::SELECTION))
                            .text_color(rgb(theme::TEXT))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!on, |d| {
                        d.text_color(rgb(theme::TEXT_2))
                            .hover(|s| s.text_color(rgb(theme::TEXT)))
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
            .bg(rgb(if on { theme::PANE } else { theme::TEXT_2 }));
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
                theme::TEXT
            } else {
                theme::SURFACE_HOVER
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
                d.bg(rgb(theme::SURFACE))
                    .border_color(rgba(0xffffffb3))
                    .text_color(rgb(theme::TEXT))
            })
            .when(!on, |d| {
                d.border_color(rgba(theme::INPUT_EDGE))
                    .text_color(rgb(theme::TEXT_2))
                    .hover(|s| s.bg(rgb(theme::ROW_HOVER)).text_color(rgb(theme::TEXT)))
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
                .border_color(rgba(theme::HAIRLINE))
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
                        d.bg(rgb(theme::SURFACE)).border_color(rgba(0xffffffb3))
                    })
                    .when(!on, |d| {
                        d.border_color(rgba(theme::HAIRLINE))
                            .hover(|s| s.bg(rgb(theme::ROW_HOVER)))
                    })
                    .child(strip)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(u(2.))
                            .text_size(u(12.))
                            .text_color(rgb(if on { theme::TEXT } else { theme::TEXT_2 }))
                            .child(div().flex_1().child(scheme.name()))
                            .when(on, |d| d.child(icon(Icon::Check, 13., theme::TEXT))),
                    )
                    .on_click(self.on(move |s| s.scheme = scheme, cx)),
            );
        }
        wrap.into_any_element()
    }

    fn agents(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.default_agent.clone();
        let mut wrap = div().flex().flex_wrap().gap(u(6.)).child(
            Self::chip("agent-auto".into(), current.is_none())
                .child("Automatic")
                .on_click(self.on(|s| s.default_agent = None, cx)),
        );
        let mut listed: Vec<(SharedString, SharedString)> = self.agents.clone();
        // An agent chosen earlier that is not ready now still shows as chosen.
        if let Some(id) = current.as_deref() {
            if !listed.iter().any(|(a, _)| a.as_ref() == id) {
                listed.push((id.to_string().into(), format!("{id} (not ready)").into()));
            }
        }
        for (ix, (id, name)) in listed.into_iter().enumerate() {
            let on = current.as_deref() == Some(id.as_ref());
            let pick = id.to_string();
            wrap = wrap.child(
                Self::chip(ElementId::NamedInteger("agent".into(), ix as u64), on)
                    .child(icon(
                        Icon::agent(&id),
                        14.,
                        if on { theme::TEXT } else { theme::TEXT_2 },
                    ))
                    .child(name)
                    .on_click(self.on(move |s| s.default_agent = Some(pick.clone()), cx)),
            );
        }
        if self.agents.is_empty() {
            wrap = wrap.child(
                div()
                    .text_size(u(12.))
                    .text_color(rgb(theme::DIM))
                    .child("Detecting agents…"),
            );
        }
        wrap.into_any_element()
    }
}

// -- layout -----------------------------------------------------------------------

/// A setting: name and one line of explanation on the left, its control on
/// the right.
fn row(label: &'static str, about: impl Into<SharedString>, control: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(u(24.))
        .min_h(u(60.))
        .px(u(16.))
        .py(u(12.))
        .child(describe(label, about).flex_1().min_w_0())
        .child(control)
}

/// A setting whose control needs the full width (a list to pick from).
fn stacked(label: &'static str, about: impl Into<SharedString>, control: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(12.))
        .px(u(16.))
        .py(u(14.))
        .child(describe(label, about))
        .child(control)
}

fn describe(label: &'static str, about: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(2.))
        .child(
            div()
                .text_size(u(13.))
                .text_color(rgb(theme::TEXT))
                .child(label),
        )
        .child(
            div()
                .text_size(u(12.))
                .line_height(u(17.))
                .text_color(rgb(theme::DIM))
                .child(about.into()),
        )
}

/// A titled card of rows with a hairline between each.
fn section(title: &'static str, rows: Vec<Div>) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(u(theme::CARD_RADIUS))
        .bg(rgb(theme::RAISED))
        .border_1()
        .border_color(rgba(theme::HAIRLINE));
    for (ix, r) in rows.into_iter().enumerate() {
        card = card.child(r.when(ix > 0, |d| {
            d.border_t_1().border_color(rgba(theme::HAIRLINE))
        }));
    }
    div()
        .flex()
        .flex_col()
        .gap(u(10.))
        .mt(u(28.))
        .child(
            div()
                .pl(u(4.))
                .text_size(u(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::TEXT))
                .child(title),
        )
        .child(card)
}

impl Render for SettingsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.settings.clone();
        let (min_zoom, max_zoom) = (ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);

        let zoom = self.stepper(
            "zoom",
            format!("{:.0}%", s.zoom * 100.),
            (s.zoom > min_zoom).then_some(|s: &mut Settings| s.zoom = s.stepped(-1)),
            (s.zoom < max_zoom).then_some(|s: &mut Settings| s.zoom = s.stepped(1)),
            cx,
        );
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
        let engines: Vec<(Search, &'static str)> =
            Search::ALL.iter().map(|e| (*e, e.name())).collect();
        let access: Vec<(Access, &'static str)> =
            Access::ALL.iter().map(|a| (*a, a.name())).collect();

        let appearance = section(
            "Appearance",
            vec![
                row(
                    "Zoom",
                    "One size for the whole window: sidebar, chats, terminals and pages. ⌘+ / ⌘− / ⌘0.",
                    zoom,
                ),
                stacked(
                    "Terminal colours",
                    "The palette terminals and agent TUIs draw with. Deck's chrome stays monochrome.",
                    self.schemes(cx),
                ),
            ],
        );
        let terminal = section(
            "Terminal",
            vec![
                stacked(
                    "Font",
                    "Monospace faces installed on this Mac.",
                    self.fonts(cx),
                ),
                row("Font size", "At 100% zoom.", font_px),
                row(
                    "Line height",
                    "Space between lines, as a multiple of the font size.",
                    line,
                ),
                row(
                    "Cursor",
                    "Until a program asks for its own shape.",
                    self.segmented("cursor", &cursors, s.cursor, |s, c| s.cursor = c, cx),
                ),
                row(
                    "Blink cursor",
                    "Only while the terminal has focus.",
                    self.toggle("blink", s.cursor_blink, |s, on| s.cursor_blink = on, cx),
                ),
                row(
                    "Option as Meta",
                    "⌥ sends Esc+key (⌥B, ⌥F word jumps) instead of typing ∫ or ƒ.",
                    self.toggle("meta", s.option_as_meta, |s, on| s.option_as_meta = on, cx),
                ),
            ],
        );
        let browser = section(
            "Browser",
            vec![row(
                "Search engine",
                // The engine's real results URL, so the choice is concrete.
                format!(
                    "Anything typed in the address bar that is not an address goes to {}",
                    s.search_url("…")
                        .trim_start_matches("https://")
                        .trim_start_matches("www.")
                ),
                self.segmented("search", &engines, s.search, |s, e| s.search = e, cx),
            )],
        );
        let agents = section(
            "Agents",
            vec![
                stacked(
                    "New chats start with",
                    "Automatic picks Claude when it is signed in, else the first agent that is ready.",
                    self.agents(cx),
                ),
                row(
                    "Starting access",
                    "What a new thread may do without asking. Each thread's chip still changes it.",
                    self.segmented("access", &access, s.access, |s, a| s.access = a, cx),
                ),
            ],
        );

        let path = settings::path();
        let footer = div()
            .flex()
            .items_center()
            .gap(u(12.))
            .mt(u(24.))
            .px(u(4.))
            .text_size(u(12.))
            .text_color(rgb(theme::DIM))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("Saved to {}", tilde(&path.display().to_string()))),
            )
            .child(
                div()
                    .id("reveal")
                    .flex_none()
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child("Show in Finder")
                    .on_click(move |_, _, _| {
                        let _ = std::process::Command::new("/usr/bin/open")
                            .arg("-R")
                            .arg(&path)
                            .spawn();
                    }),
            )
            .child(
                div()
                    .id("defaults")
                    .flex_none()
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child("Restore defaults")
                    .on_click(self.on(|s| *s = Settings::default(), cx)),
            );

        let column = div()
            .flex()
            .flex_col()
            .w(u(640.))
            .max_w(relative(1.))
            .px(u(24.))
            .pt(u(36.))
            .pb(u(56.))
            .child(
                div()
                    .px(u(4.))
                    .text_size(u(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT))
                    .child("Settings"),
            )
            .child(
                div()
                    .px(u(4.))
                    .pt(u(4.))
                    .text_size(u(13.))
                    .text_color(rgb(theme::TEXT_2))
                    .child("Changes apply right away and are kept across launches."),
            )
            .child(appearance)
            .child(terminal)
            .child(browser)
            .child(agents)
            .child(footer);

        div()
            .id("settings")
            .track_focus(&self.focus)
            .key_context("Settings")
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(theme::PANE))
            .child(div().flex().justify_center().child(column))
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
