//! Selecting text in a chat with the mouse, across paragraphs and messages,
//! to copy with ⌘C. gpui's text has no selection of its own, so every piece
//! of text a chat draws is a unit with a key in reading order, and the chat
//! keeps the layout each unit was painted with to find the character under
//! the mouse.

use gpui::{Bounds, Hsla, Pixels, Point, SharedString, TextLayout, TextRun};
use std::collections::BTreeMap;
use std::ops::Range;

/// (message, unit within the message), in reading order.
pub type Key = (usize, usize);

/// A place in the text: a unit and a byte offset into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub key: Key,
    pub at: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag started.
    pub anchor: Spot,
    /// Where it is now.
    pub focus: Spot,
}

impl Selection {
    fn ordered(&self) -> (Spot, Spot) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The part of unit `key`, `len` bytes long, that is selected.
    pub fn covers(&self, key: Key, len: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        if key < a.key || key > b.key {
            return None;
        }
        let start = if key == a.key { a.at.min(len) } else { 0 };
        let end = if key == b.key { b.at.min(len) } else { len };
        (start < end).then_some(start..end)
    }
}

/// A unit's text, and how it joins the one before it when copied.
#[derive(Clone)]
pub struct Unit {
    pub text: SharedString,
    /// Put between it and the unit before: a blank line between paragraphs,
    /// a newline between list items, a tab between table cells.
    pub sep: &'static str,
    /// Copied before it when it is taken from its start: a list marker.
    pub prefix: SharedString,
}

/// A unit as painted in the last frame.
pub struct Painted {
    pub key: Key,
    pub layout: TextLayout,
    pub bounds: Bounds<Pixels>,
}

/// The spot under `p`, only when it is over text.
pub fn hit(painted: &[Painted], p: Point<Pixels>) -> Option<Spot> {
    let u = painted.iter().find(|u| u.bounds.contains(&p))?;
    Some(Spot {
        key: u.key,
        at: index(u, p),
    })
}

/// The spot nearest `p`, for a drag that has left the text: the start of
/// the next unit below, or the end of the last one above.
pub fn nearest(painted: &[Painted], p: Point<Pixels>) -> Option<Spot> {
    if let Some(s) = hit(painted, p) {
        return Some(s);
    }
    // On a unit's lines but past its edge: the line's start or end.
    let beside = painted
        .iter()
        .filter(|u| p.y >= u.bounds.top() && p.y <= u.bounds.bottom())
        .min_by_key(|u| {
            let dx = if p.x < u.bounds.left() {
                u.bounds.left() - p.x
            } else {
                p.x - u.bounds.right()
            };
            (f32::from(dx) * 64.) as i64
        });
    if let Some(u) = beside {
        let x = p.x.clamp(u.bounds.left(), u.bounds.right() - gpui::px(1.));
        return Some(Spot {
            key: u.key,
            at: index(u, gpui::point(x, p.y)),
        });
    }
    let mut sorted: Vec<&Painted> = painted.iter().collect();
    sorted.sort_by_key(|u| u.key);
    match sorted.iter().find(|u| u.bounds.top() > p.y) {
        Some(below) => Some(Spot {
            key: below.key,
            at: 0,
        }),
        None => sorted.last().map(|u| Spot {
            key: u.key,
            at: u.layout.len(),
        }),
    }
}

fn index(u: &Painted, p: Point<Pixels>) -> usize {
    u.layout
        .index_for_position(p)
        .unwrap_or_else(|i| i)
        .min(u.layout.len())
}

/// The word around `at` in `text`, for a double click.
pub fn word(text: &str, at: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = at.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(i, _)| at + i);
    start..end
}

/// `runs` with the bytes in `range` on the selection colour.
pub fn highlight(runs: Vec<TextRun>, range: Range<usize>, bg: Hsla) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs {
        let (s, e) = (at, at + run.len);
        at = e;
        let cuts = [s, range.start.clamp(s, e), range.end.clamp(s, e), e];
        for w in cuts.windows(2) {
            if w[0] < w[1] {
                let mut piece = run.clone();
                piece.len = w[1] - w[0];
                if w[0] >= range.start && w[1] <= range.end {
                    piece.background_color = Some(bg);
                }
                out.push(piece);
            }
        }
    }
    out
}

/// The selected text, units joined the way they are laid out.
pub fn text(units: &BTreeMap<Key, Unit>, sel: &Selection) -> String {
    let (a, b) = sel.ordered();
    let mut out = String::new();
    for (key, u) in units.range(a.key..=b.key) {
        let Some(r) = sel.covers(*key, u.text.len()) else {
            continue;
        };
        let Some(part) = u.text.get(r.clone()) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str(u.sep);
        }
        if r.start == 0 {
            out.push_str(&u.prefix);
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(text: &str, sep: &'static str, prefix: &str) -> Unit {
        Unit {
            text: text.to_string().into(),
            sep,
            prefix: prefix.to_string().into(),
        }
    }

    fn sel(a: (usize, usize, usize), b: (usize, usize, usize)) -> Selection {
        Selection {
            anchor: Spot {
                key: (a.0, a.1),
                at: a.2,
            },
            focus: Spot {
                key: (b.0, b.1),
                at: b.2,
            },
        }
    }

    #[test]
    fn a_selection_covers_whole_units_between_its_ends() {
        let s = sel((1, 2, 5), (0, 1, 3)); // dragged upwards
        assert_eq!(s.covers((0, 0), 10), None);
        assert_eq!(s.covers((0, 1), 10), Some(3..10));
        assert_eq!(s.covers((0, 5), 10), Some(0..10));
        assert_eq!(s.covers((1, 2), 10), Some(0..5));
        assert_eq!(s.covers((1, 3), 10), None);
        assert!(sel((0, 0, 4), (0, 0, 4)).is_empty());
    }

    #[test]
    fn copied_text_joins_paragraphs_items_and_cells() {
        let mut units = BTreeMap::new();
        units.insert((0, 0), unit("First paragraph.", "\n\n", ""));
        units.insert((0, 1), unit("one", "\n\n", "1. "));
        units.insert((0, 2), unit("two", "\n", "2. "));
        units.insert((1, 0), unit("Next reply.", "\n\n", ""));
        assert_eq!(
            text(&units, &sel((0, 0, 6), (1, 0, 4))),
            "paragraph.\n\n1. one\n2. two\n\nNext"
        );
        // Starting inside an item leaves its marker out.
        assert_eq!(text(&units, &sel((0, 1, 1), (0, 2, 3))), "ne\n2. two");
    }

    #[test]
    fn words_and_highlights() {
        assert_eq!(word("fix the snake_case name", 10), 8..18);
        assert_eq!(word("a b", 1), 0..1);
        assert_eq!(word("a  b", 2), 2..2);
        assert_eq!(word("end", 3), 0..3);
        let run = |len| TextRun {
            len,
            font: gpui::font("Menlo"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let out = highlight(vec![run(4), run(6)], 2..7, gpui::white());
        let lens: Vec<usize> = out.iter().map(|r| r.len).collect();
        assert_eq!(lens, [2, 2, 3, 3]);
        let lit: Vec<bool> = out.iter().map(|r| r.background_color.is_some()).collect();
        assert_eq!(lit, [false, true, true, false]);
    }
}
