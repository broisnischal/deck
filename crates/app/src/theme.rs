//! Colours and metrics: macOS dark, matched to the system's own greys so the
//! app sits next to native windows without looking pasted in.

// Black and monochrome: hierarchy comes from lightness alone. Colour is
// left to content (terminal output, web pages), never to chrome. The steps
// follow T3 Code's dark palette: a #000 sidebar, a near-black content
// surface, cards 3% lighter, and edges drawn in translucent white.
pub const SIDEBAR: u32 = 0x000000;
pub const PANE: u32 = 0x0a0a0a;
/// Cards and the composer: the content surface plus 3% white.
pub const RAISED: u32 = 0x111111;
/// Filled controls and the selected row.
pub const SURFACE: u32 = 0x1c1c1c;
pub const SURFACE_HOVER: u32 = 0x262626;
pub const ROW_HOVER: u32 = 0x141414;
pub const BORDER: u32 = 0x1a1a1a;
pub const TEXT: u32 = 0xf5f5f5;
pub const TEXT_2: u32 = 0xa3a3a3;
/// Secondary labels. #737373 keeps 4.6:1 on black; anything darker stops
/// being readable text.
pub const DIM: u32 = 0x737373;
/// Focus, carets, the cursor: white is the only emphasis colour.
pub const ACCENT: u32 = 0xffffff;
pub const SELECTION: u32 = 0x3a3a3a;
/// White at 6%: structural edges (cards, panes).
pub const HAIRLINE: u32 = 0xffffff0f;
/// White at 10%: the edge of something you type into.
pub const INPUT_EDGE: u32 = 0xffffff1a;
/// White at 16%: the card that has the keyboard, in a split.
pub const FOCUS_EDGE: u32 = 0xffffff29;
/// Black at 28% over the panes of a split that do not have the keyboard.
pub const UNFOCUSED_VEIL: u32 = 0x00000047;

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

/// The rem size at 100% zoom.
pub const REM_PX: f32 = 16.;

pub fn palette() -> term::Palette {
    term::Palette {
        fg: TEXT,
        bg: PANE,
        selection: SELECTION,
        search: 0x575757,
        search_focus: ACCENT,
        ansi: [
            0x2b2b33, 0xff7b7b, 0x6ddf9a, 0xffc069, 0x8b9dff, 0xc4a6ff, 0x8fd3f4, 0xa0a2ad,
            0x565660, 0xff9c9c, 0x92e9b4, 0xffd08e, 0xa8b5ff, 0xd4c1ff, 0xaee0f8, 0xecedf1,
        ],
    }
}
