//! Preferences that outlive a launch: one small JSON file beside the control
//! socket. Unknown or missing fields fall back to defaults, so an older file
//! never stops the app from starting.

use crate::theme;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The zoom steps ⌘+ and ⌘− walk through (1.0 = 100%).
pub const ZOOMS: [f32; 10] = [0.75, 0.85, 0.9, 1.0, 1.1, 1.2, 1.35, 1.5, 1.75, 2.0];
pub const DEFAULT_ZOOM: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Terminal font size at 100% zoom.
    pub font_px: f32,
    /// One zoom for everything: UI, terminals, web pages.
    pub zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_px: theme::FONT_PX,
            zoom: DEFAULT_ZOOM,
        }
    }
}

impl Settings {
    pub fn terminal_font(&self) -> f32 {
        self.font_px * self.zoom
    }

    /// The next zoom step in `dir` (+1 / −1), or 100% for 0.
    pub fn stepped(&self, dir: i32) -> f32 {
        let at = ZOOMS
            .iter()
            .position(|z| (z - self.zoom).abs() < 0.01)
            // Off the ladder (an old file): start from the nearest step.
            .unwrap_or_else(|| {
                ZOOMS
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (a.1 - self.zoom).abs().total_cmp(&(b.1 - self.zoom).abs()))
                    .map_or(3, |(i, _)| i)
            }) as i32;
        match dir {
            0 => DEFAULT_ZOOM,
            d => ZOOMS[(at + d).clamp(0, ZOOMS.len() as i32 - 1) as usize],
        }
    }
}

/// The on-disk shape, tolerant of files written by older versions (which had
/// only `font_px`, then the terminal's own zoom).
#[derive(Deserialize, Default)]
struct Stored {
    font_px: Option<f32>,
    zoom: Option<f32>,
}

fn path() -> PathBuf {
    crate::control::socket_dir().join("settings.json")
}

fn from_stored(s: Stored) -> Settings {
    let base = theme::FONT_PX;
    match (s.font_px, s.zoom) {
        (Some(f), Some(z)) => Settings {
            font_px: f.clamp(9., 32.),
            zoom: z.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]),
        },
        // An older file zoomed by growing the terminal font: keep how big
        // things looked by turning that into the app zoom.
        (Some(f), None) => Settings {
            font_px: base,
            zoom: (f / base).clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]),
        },
        (None, z) => Settings {
            font_px: base,
            zoom: z
                .unwrap_or(DEFAULT_ZOOM)
                .clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]),
        },
    }
}

pub fn load() -> Settings {
    let stored: Stored = std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    from_stored(stored)
}

/// Written to a temp file and renamed, so a crash mid-write cannot leave a
/// half file that resets everything on the next launch.
pub fn save(s: &Settings) {
    let target = path();
    let tmp = target.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
        std::fs::rename(&tmp, &target)
    };
    if let Err(e) = write() {
        tracing::warn!("could not save settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Settings {
        from_stored(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn old_files_keep_their_size_as_app_zoom() {
        assert_eq!(parse("{}"), Settings::default());
        let s = parse(r#"{"font_px": 19.5}"#);
        assert_eq!(s.font_px, theme::FONT_PX);
        assert!((s.zoom - 1.5).abs() < 1e-6);
        assert_eq!(parse(r#"{"font_px": 13, "zoom": 1.2}"#).zoom, 1.2);
        assert_eq!(parse(r#"{"zoom": 9}"#).zoom, 2.0);
    }

    #[test]
    fn zoom_walks_the_ladder_and_stops_at_the_ends() {
        let mut s = Settings::default();
        s.zoom = s.stepped(1);
        assert_eq!(s.zoom, 1.1);
        s.zoom = s.stepped(-1);
        s.zoom = s.stepped(-1);
        assert_eq!(s.zoom, 0.9);
        s.zoom = 2.0;
        assert_eq!(s.stepped(1), 2.0);
        s.zoom = 1.31; // off the ladder: nearest is 1.35
        assert_eq!(s.stepped(1), 1.5);
        assert_eq!(s.stepped(0), 1.0);
    }
}
