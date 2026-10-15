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
    ArrowUp => "arrow-up.svg",
    Stop => "stop.svg",
    Check => "check.svg",
    Spinner => "spinner.svg",
    ListCheck => "list-check.svg",
    Pencil => "pencil.svg",
    Search => "search.svg",
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

/// Serves the embedded icons to gpui.
/// A spinner that turns once a second, for work in progress. `id` keeps the
/// rotation steady across frames.
pub fn spinner(id: impl Into<gpui::ElementId>, size: f32, color: u32) -> AnyElement {
    use gpui::{percentage, Animation, AnimationExt, Transformation};
    svg()
        .path(Icon::Spinner.path())
        .size(u(size))
        .flex_none()
        .text_color(rgb(color))
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(900)).repeat(),
            |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
        )
        .into_any_element()
}

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
