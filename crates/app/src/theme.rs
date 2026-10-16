//! Colours and metrics. The window wears one theme at a time, picked in
//! Settings and applied everywhere at once: sidebar, panes, chats, menus,
//! the terminal's own palette, and the appearance of native views (pages,
//! traffic lights, scrollbars).
//!
//! Every theme is monochrome: hierarchy comes from lightness alone, and
//! colour is left to content (terminal output, web pages). Edges are drawn in
//! translucent text colour, so they sit right on any surface.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// T3 Code's dark: a #000 sidebar, near-black panes.
    #[default]
    Black,
    /// macOS's own dark greys.
    Graphite,
    /// Dark with a blue cast.
    Midnight,
    /// macOS light.
    Light,
    /// Black or Light, following the system's appearance.
    System,
}

impl Theme {
    pub const ALL: [Theme; 5] = [
        Theme::System,
        Theme::Black,
        Theme::Graphite,
        Theme::Midnight,
        Theme::Light,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Black => "Black",
            Theme::Graphite => "Graphite",
            Theme::Midnight => "Midnight",
            Theme::Light => "Light",
            Theme::System => "System",
        }
    }

    /// The colours it stands for, given whether the system is dark.
    pub fn colors(self, system_dark: bool) -> &'static Colors {
        &ALL[self.index(system_dark) as usize]
    }

    fn index(self, system_dark: bool) -> u8 {
        match self {
            Theme::Black => 0,
            Theme::Graphite => 1,
            Theme::Midnight => 2,
            Theme::Light => 3,
            Theme::System if system_dark => 0,
            Theme::System => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub sidebar: u32,
    pub pane: u32,
    /// Cards and the composer: a step up from the pane.
    pub raised: u32,
    /// Filled controls and the selected row.
    pub surface: u32,
    pub surface_hover: u32,
    pub row_hover: u32,
    /// The selected sidebar row: lifted clear of the hover tone, so a
    /// hovered row never reads as the open one.
    pub row_active: u32,
    pub border: u32,
    pub text: u32,
    pub text_2: u32,
    /// Secondary labels, kept at 4.5:1 or better on the pane.
    pub dim: u32,
    /// Focus, carets, the cursor: the one emphasis colour.
    pub accent: u32,
    pub selection: u32,
    /// Structural edges (cards, panes), RGBA.
    pub hairline: u32,
    /// The edge of something you type into, RGBA.
    pub input_edge: u32,
    /// The card that has the keyboard, in a split, RGBA.
    pub focus_edge: u32,
    /// An edge that has to be seen (a failed card, a chosen option), RGBA.
    pub strong_edge: u32,
    /// Over the panes of a split that do not have the keyboard, RGBA.
    pub veil: u32,
    /// Floating menus.
    pub menu: u32,
    /// A faint fill behind inline code and quiet chips, RGBA.
    pub tint: u32,
    /// The terminal's sixteen colours under this theme.
    pub ansi: [u32; 16],
}

const DARK_ANSI: [u32; 16] = [
    0x2b2b33, 0xff7b7b, 0x6ddf9a, 0xffc069, 0x8b9dff, 0xc4a6ff, 0x8fd3f4, 0xa0a2ad, 0x565660,
    0xff9c9c, 0x92e9b4, 0xffd08e, 0xa8b5ff, 0xd4c1ff, 0xaee0f8, 0xecedf1,
];

/// Dark enough to read on white: the same hues, deepened.
const LIGHT_ANSI: [u32; 16] = [
    0x1d1d1f, 0xc4282e, 0x1a7f37, 0x9a6700, 0x0550ae, 0x8250df, 0x1b7c83, 0x6e7781, 0x57606a,
    0xd1242f, 0x1f883d, 0xb08800, 0x0969da, 0x8a63d2, 0x3192aa, 0x8c959f,
];

static ALL: [Colors; 4] = [
    // Black
    Colors {
        dark: true,
        sidebar: 0x000000,
        pane: 0x0a0a0a,
        raised: 0x111111,
        surface: 0x1c1c1c,
        surface_hover: 0x262626,
        row_hover: 0x141414,
        row_active: 0x1c1c1c,
        border: 0x1a1a1a,
        text: 0xf5f5f5,
        text_2: 0xa3a3a3,
        dim: 0x737373,
        accent: 0xffffff,
        selection: 0x3a3a3a,
        hairline: 0xffffff0f,
        input_edge: 0xffffff1a,
        focus_edge: 0xffffff29,
        strong_edge: 0xffffff30,
        veil: 0x00000047,
        menu: 0x161616,
        tint: 0xffffff0d,
        ansi: DARK_ANSI,
    },
    // Graphite
    Colors {
        dark: true,
        sidebar: 0x1c1c1e,
        pane: 0x232325,
        raised: 0x2a2a2c,
        surface: 0x353538,
        surface_hover: 0x3f3f43,
        row_hover: 0x2c2c2f,
        row_active: 0x353538,
        border: 0x38383b,
        text: 0xf2f2f7,
        text_2: 0xb0b0b5,
        dim: 0x8e8e93,
        accent: 0xffffff,
        selection: 0x4c4c51,
        hairline: 0xffffff12,
        input_edge: 0xffffff1f,
        focus_edge: 0xffffff33,
        strong_edge: 0xffffff38,
        veil: 0x0000004d,
        menu: 0x2c2c2f,
        tint: 0xffffff0f,
        ansi: DARK_ANSI,
    },
    // Midnight
    Colors {
        dark: true,
        sidebar: 0x0b0e14,
        pane: 0x10141c,
        raised: 0x161b25,
        surface: 0x1e2431,
        surface_hover: 0x272e3e,
        row_hover: 0x181d28,
        row_active: 0x1e2431,
        border: 0x202634,
        text: 0xe8ebf2,
        text_2: 0xa6aec0,
        dim: 0x76809a,
        accent: 0xffffff,
        selection: 0x313b55,
        hairline: 0xffffff10,
        input_edge: 0xffffff1c,
        focus_edge: 0xffffff2e,
        strong_edge: 0xffffff33,
        veil: 0x0000004d,
        menu: 0x181d28,
        tint: 0xffffff0d,
        ansi: DARK_ANSI,
    },
    // Light
    Colors {
        dark: false,
        sidebar: 0xf0f0f0,
        pane: 0xffffff,
        raised: 0xf7f7f7,
        surface: 0xe6e6e6,
        surface_hover: 0xdcdcdc,
        row_hover: 0xe8e8e8,
        row_active: 0xffffff,
        border: 0xe2e2e2,
        text: 0x1d1d1f,
        text_2: 0x55555a,
        dim: 0x76767b,
        accent: 0x000000,
        selection: 0xcfcfd4,
        hairline: 0x0000001a,
        input_edge: 0x00000026,
        focus_edge: 0x00000040,
        strong_edge: 0x00000033,
        veil: 0xffffff66,
        menu: 0xffffff,
        tint: 0x0000000a,
        ansi: LIGHT_ANSI,
    },
];

/// Index into `ALL` of the theme the window wears now.
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Wear `theme` from the next frame on.
pub fn set(theme: Theme, system_dark: bool) {
    CURRENT.store(theme.index(system_dark), Ordering::Relaxed);
}

/// The colours in use now.
pub fn current() -> &'static Colors {
    &ALL[CURRENT.load(Ordering::Relaxed) as usize]
}

pub fn sidebar() -> u32 {
    current().sidebar
}
pub fn pane() -> u32 {
    current().pane
}
pub fn raised() -> u32 {
    current().raised
}
pub fn surface() -> u32 {
    current().surface
}
pub fn surface_hover() -> u32 {
    current().surface_hover
}
pub fn row_hover() -> u32 {
    current().row_hover
}
pub fn row_active() -> u32 {
    current().row_active
}
pub fn border() -> u32 {
    current().border
}
pub fn text() -> u32 {
    current().text
}
pub fn text_2() -> u32 {
    current().text_2
}
pub fn dim() -> u32 {
    current().dim
}
pub fn accent() -> u32 {
    current().accent
}
/// Red for what takes something away (Lock, Delete), readable on every
/// theme's menu.
pub const DANGER: u32 = 0xe5534b;

pub fn selection() -> u32 {
    current().selection
}
pub fn hairline() -> u32 {
    current().hairline
}
pub fn input_edge() -> u32 {
    current().input_edge
}
pub fn focus_edge() -> u32 {
    current().focus_edge
}
pub fn strong_edge() -> u32 {
    current().strong_edge
}
pub fn veil() -> u32 {
    current().veil
}
pub fn menu() -> u32 {
    current().menu
}
pub fn tint() -> u32 {
    current().tint
}
pub fn is_dark() -> bool {
    current().dark
}

/// A theme colour (RGB) at an alpha, for `rgba`.
pub fn alpha(rgb: u32, a: u8) -> u32 {
    (rgb << 8) | a as u32
}

pub const UI: &str = ".SystemUIFont";
/// Every Mac has Menlo, so the grid never falls back to a proportional face.
pub const MONO: &str = "Menlo";
pub const FONT_PX: f32 = 13.;
pub const TOOLBAR_PX: f32 = 40.;
pub const SIDEBAR_PX: f32 = 256.;
pub const INSPECTOR_PX: f32 = 320.;
/// Height of the transparent titlebar the traffic lights sit in.
pub const TITLEBAR_PX: f32 = 40.;

/// The one inset between the window edge and everything inside it.
pub const GUTTER: f32 = 8.;
/// Leading padding inside a sidebar row; section labels share this edge.
pub const ROW_PAD: f32 = 8.;
/// Concentric with the window's corner: card = window radius − GUTTER.
pub const CARD_RADIUS: f32 = 10.;
pub const ROW_RADIUS: f32 = 8.;
/// Terminal text inset inside its card; the title bar aligns to it.
pub const PANE_PAD_X: f32 = 12.;
pub const PANE_PAD_Y: f32 = 8.;

/// A UI length that follows the app zoom: `u(13.)` is 13 points at 100%.
/// Built on rems, so one `set_rem_size` per frame scales the whole UI.
pub fn u(v: f32) -> gpui::Rems {
    gpui::rems(v / REM_PX)
}

/// One line of text that ends in "…" where it stops fitting.
///
/// gpui 0.2.2's `truncate()` sizes nowrap text once and keeps that size for
/// the rest of the layout, so the "…" goes where the first, wider pass put
/// it and the box then clips the line mid-letter. A one-line clamp is
/// measured again at every width, so the ellipsis lands at the real edge.
pub trait OneLine: gpui::Styled {
    fn one_line(self) -> Self {
        self.whitespace_normal().line_clamp(1).text_ellipsis()
    }
}

impl<T: gpui::Styled> OneLine for T {}

/// The rem size at 100% zoom.
pub const REM_PX: f32 = 16.;

/// The terminal palette that matches the theme: its text on its pane.
pub fn palette() -> term::Palette {
    let c = current();
    term::Palette {
        fg: c.text,
        bg: c.pane,
        selection: c.selection,
        search: if c.dark { 0x575757 } else { 0xc8c8cc },
        search_focus: c.accent,
        ansi: c.ansi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance contrast, WCAG's formula.
    fn contrast(a: u32, b: u32) -> f64 {
        let lum = |c: u32| {
            let ch = |v: u32| {
                let v = v as f64 / 255.;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(c >> 16 & 0xff) + 0.7152 * ch(c >> 8 & 0xff) + 0.0722 * ch(c & 0xff)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_theme_keeps_its_text_readable() {
        for c in &ALL {
            for bg in [c.pane, c.sidebar, c.raised] {
                assert!(contrast(c.text, bg) >= 12., "{c:?}");
                assert!(
                    contrast(c.text_2, bg) >= 4.5,
                    "{:06x} on {bg:06x}",
                    c.text_2
                );
                assert!(contrast(c.dim, bg) >= 3.9, "{:06x} on {bg:06x}", c.dim);
            }
        }
    }

    #[test]
    fn system_follows_the_appearance() {
        assert!(Theme::System.colors(true).dark);
        assert!(!Theme::System.colors(false).dark);
        assert_eq!(alpha(0xffffff, 0x30), 0xffffff30);
    }
}
