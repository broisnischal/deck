//! Icons: real SVGs through gpui's `svg()`, tinted by `text_color`.
//!
//! gpui rasterises an SVG to an alpha mask, so one file serves every colour
//! and state. Files live in `assets/icons/` and are embedded at compile time.
//! The set (and its 1.8px round stroke) comes from yourcode; keep additions in
//! that style so nothing on a surface reads heavier than its neighbours.

use crate::theme::u;
use gpui::prelude::*;
use gpui::{rgb, svg, AnyElement, AssetSource, SharedString};
use std::borrow::Cow;

/// `Variant => "file.svg"`, once; the enum, paths and embeds derive from it.
macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon { $($variant),* }

        impl Icon {
            pub fn path(self) -> &'static str {
                match self { $(Icon::$variant => concat!("icons/", $file)),* }
            }
        }

        const FILES: &[(&str, &[u8])] = &[
            $(($file, include_bytes!(concat!("../assets/icons/", $file)) as &[u8])),*
        ];
    };
}

icons! {
    Sidebar => "sidebar.svg",
    PanelRight => "panel-right.svg",
    Split => "split.svg",
    Plus => "plus.svg",
    Close => "close.svg",
    ChevronDown => "chevron-down.svg",
    ChevronUp => "chevron-up.svg",
    ChevronRight => "chevron-right.svg",
    ArrowLeft => "arrow-left.svg",
    ArrowRight => "arrow-right.svg",
    ArrowUpRight => "arrow-up-right.svg",
    Refresh => "refresh.svg",
    Globe => "globe.svg",
    Terminal => "terminal.svg",
    Folder => "folder.svg",
    File => "file.svg",
    Files => "files.svg",
    Git => "git.svg",
    Info => "info.svg",
    Copy => "copy.svg",
    Clipboard => "clipboard.svg",
    Scissors => "scissors.svg",
    SelectAll => "select-all.svg",
    Eraser => "eraser.svg",
    SplitDown => "split-down.svg",
    Link => "link.svg",
    CloseOthers => "close-others.svg",
    ArrowUp => "arrow-up.svg",
    Check => "check.svg",
    Spinner => "spinner.svg",
    ListCheck => "list-check.svg",
    Pencil => "pencil.svg",
    Search => "search.svg",
    Download => "download.svg",
    Clock => "clock.svg",
    Bell => "bell.svg",
    Bolt => "bolt.svg",
    Shield => "shield.svg",
    Settings => "settings.svg",
    Snippet => "snippet.svg",
    User => "user.svg",
    Lock => "lock.svg",
    Theme => "theme.svg",
    Claude => "agent-claude.svg",
    Codex => "agent-codex.svg",
    Gemini => "agent-gemini.svg",
    Grok => "agent-grok.svg",
    OpenCode => "agent-opencode.svg",
    GenericAgent => "agent-generic.svg",
}

impl Icon {
    /// An agent's mark by its command name; unknown ones get the generic mark.
    pub fn agent(cmd: &str) -> Icon {
        match cmd {
            "claude" => Icon::Claude,
            "codex" => Icon::Codex,
            "gemini" => Icon::Gemini,
            "grok" => Icon::Grok,
            "opencode" => Icon::OpenCode,
            _ => Icon::GenericAgent,
        }
    }
}

pub fn icon(kind: Icon, size: f32, color: u32) -> AnyElement {
    svg()
        .path(kind.path())
        .size(u(size))
        .flex_none()
        .text_color(rgb(color))
        .into_any_element()
}

/// Work in progress: a comet, its head bright and its tail fading, turning
/// a little faster than once a second. `id` keeps the motion steady across
/// frames.
pub fn spinner(id: impl Into<gpui::ElementId>, size: f32, color: u32) -> AnyElement {
    use gpui::{percentage, Animation, AnimationExt, Transformation};
    svg()
        .path(Icon::Spinner.path())
        .size(u(size))
        .flex_none()
        .text_color(rgb(color))
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(850)).repeat(),
            |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
        )
        .into_any_element()
}

/// An agent at work: its own mark turns in eased quarter steps and breathes
/// in between, the way Claude Code's glyph does in a terminal. Still marks
/// read as idle; this one reads as thinking.
pub fn thinking(id: impl Into<gpui::ElementId>, glyph: Icon, size: f32, color: u32) -> AnyElement {
    use gpui::{percentage, size as sz, Animation, AnimationExt, Transformation};
    svg()
        .path(glyph.path())
        .size(u(size))
        .flex_none()
        .text_color(rgb(color))
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(2400)).repeat(),
            |s, t| {
                let (step, within) = ((t * 4.).floor(), (t * 4.).fract());
                let turn = (step + ease_in_out(within)) / 4.;
                // Smallest mid-turn, full size at rest.
                let scale = 1. - 0.16 * (within * std::f32::consts::PI).sin();
                s.with_transformation(
                    Transformation::rotate(percentage(turn)).with_scaling(sz(scale, scale)),
                )
            },
        )
        .into_any_element()
}

/// Text with a band of contrast sweeping across it: "Working…" while an
/// agent is busy. It runs from the dim label tone to the text colour, so on
/// a light theme the band darkens instead of fading into the background.
pub fn shimmer(id: impl Into<gpui::ElementId>, text: SharedString) -> AnyElement {
    use gpui::{Animation, AnimationExt, HighlightStyle, StyledText};
    gpui::div()
        .flex_none()
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(1900)).repeat(),
            move |d, t| {
                let n = text.chars().count() as f32;
                let band = 5.;
                let at = t * (n + band * 2.) - band;
                let (base, peak) = (crate::theme::dim(), crate::theme::text());
                let lights: Vec<_> = text
                    .char_indices()
                    .enumerate()
                    .map(|(i, (byte, ch))| {
                        let k = (1. - (i as f32 - at).abs() / band).max(0.);
                        (
                            byte..byte + ch.len_utf8(),
                            HighlightStyle {
                                color: Some(rgb(mix(base, peak, k)).into()),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                d.child(StyledText::new(text.clone()).with_highlights(lights))
            },
        )
        .into_any_element()
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// `a` → `b` by `k` in 0–1, per channel.
fn mix(a: u32, b: u32, k: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        ((x as f32 + (y as f32 - x as f32) * k).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// Serves the embedded icons to gpui.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path.rsplit('/').next().unwrap_or(path);
        Ok(FILES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .map(|(n, _)| SharedString::from(format!("icons/{n}")))
            .collect())
    }
}
