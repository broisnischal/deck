//! Syntax colouring for code in chats: fenced blocks and file excerpts.
//!
//! The grammars are bat's (syntect, via two-face). A token is tagged with what
//! it is (a keyword, a string, a comment), not with a colour: the colour comes
//! from the theme when the text is drawn, so switching themes recolours code at
//! once without parsing it again. A block that is still streaming in is parsed
//! only from its last complete line on, resuming from the state saved there.

use crate::theme;
use gpui::{rgb, TextRun};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::str::FromStr;
use std::sync::OnceLock;
use syntect::highlighting::{
    Color, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, StyleModifier, Theme,
    ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

/// A language's grammar.
pub type Grammar = SyntaxReference;

/// What a stretch of code is, as far as colour goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Comment,
    String,
    Number,
    Constant,
    Keyword,
    Operator,
    Function,
    Type,
    Tag,
    Attribute,
    Punctuation,
    Inserted,
    Deleted,
    Heading,
}

const KINDS: [Kind; 15] = [
    Kind::Plain,
    Kind::Comment,
    Kind::String,
    Kind::Number,
    Kind::Constant,
    Kind::Keyword,
    Kind::Operator,
    Kind::Function,
    Kind::Type,
    Kind::Tag,
    Kind::Attribute,
    Kind::Punctuation,
    Kind::Inserted,
    Kind::Deleted,
    Kind::Heading,
];

/// TextMate scopes to kinds. syntect picks the most specific selector that
/// matches, so `punctuation.definition.string` is a string, not punctuation.
const SELECTORS: &[(&str, Kind)] = &[
    ("comment, punctuation.definition.comment", Kind::Comment),
    ("string, punctuation.definition.string, meta.string", Kind::String),
    ("constant.numeric", Kind::Number),
    (
        "constant.language, constant.character, constant.other, support.constant, variable.language",
        Kind::Constant,
    ),
    ("keyword, storage.type, storage.modifier", Kind::Keyword),
    ("keyword.operator", Kind::Operator),
    (
        "entity.name.function, support.function, variable.function, meta.function-call.identifier, \
         support.macro, entity.name.macro",
        Kind::Function,
    ),
    (
        "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, \
         entity.name.interface, entity.other.inherited-class, support.type, support.class, \
         storage.type.primitive, storage.type.numeric",
        Kind::Type,
    ),
    ("entity.name.tag", Kind::Tag),
    ("entity.other.attribute-name, meta.attribute, meta.annotation", Kind::Attribute),
    ("punctuation", Kind::Punctuation),
    ("markup.inserted, punctuation.definition.inserted", Kind::Inserted),
    ("markup.deleted, punctuation.definition.deleted", Kind::Deleted),
    ("markup.heading, entity.name.section, meta.diff.range, meta.diff.header", Kind::Heading),
];

/// Past this many lines a block is drawn plain: colour isn't worth a stall.
const MAX_LINES: usize = 2000;
/// Blocks remembered between frames before the memory is let go.
const MAX_BLOCKS: usize = 256;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// A theme whose "colour" for each scope is its kind's index, read back by
/// [`kind_of`].
fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| Theme {
        settings: ThemeSettings {
            foreground: Some(tag(Kind::Plain)),
            ..ThemeSettings::default()
        },
        scopes: SELECTORS
            .iter()
            .map(|(scope, kind)| ThemeItem {
                scope: ScopeSelectors::from_str(scope).expect("valid selector"),
                style: StyleModifier {
                    foreground: Some(tag(*kind)),
                    ..StyleModifier::default()
                },
            })
            .collect(),
        ..Theme::default()
    })
}

fn tag(kind: Kind) -> Color {
    Color {
        r: kind as u8,
        g: 0,
        b: 0,
        a: 0xff,
    }
}

fn kind_of(c: Color) -> Kind {
    KINDS.get(c.r as usize).copied().unwrap_or(Kind::Plain)
}

/// The grammar for a fence's language tag (`rust`, `ts`, `sh`), if any.
pub fn for_lang(lang: &str) -> Option<&'static SyntaxReference> {
    let lang = lang.trim().split([' ', ',', '{']).next()?.to_ascii_lowercase();
    let token = match lang.as_str() {
        "" | "text" | "txt" | "plain" | "plaintext" | "output" => return None,
        "shell" | "console" | "shellsession" | "sh-session" | "terminal" => "bash",
        "golang" => "go",
        "jsx" => "tsx",
        "c++" => "cpp",
        "objc" | "objective-c" => "m",
        "jsonc" | "json5" => "json",
        "patch" => "diff",
        "dockerfile" => "Dockerfile",
        other => other,
    };
    syntaxes().find_syntax_by_token(token)
}

/// The grammar for a file, from its extension or its whole name (`Makefile`).
pub fn for_path(path: &str) -> Option<&'static SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let set = syntaxes();
    name.rsplit_once('.')
        .and_then(|(_, ext)| set.find_syntax_by_extension(ext))
        .or_else(|| set.find_syntax_by_extension(name))
}

/// A block's spans so far, and where to pick up when more of it arrives.
struct Block {
    syntax: &'static str,
    text: String,
    /// Byte lengths and kinds, covering `text`.
    spans: Vec<(usize, Kind)>,
    /// Bytes of `text` in complete lines, and the spans that cover them.
    done: usize,
    done_spans: usize,
    lines: usize,
    /// The parser and highlighter as they stand at `done`.
    parse: ParseState,
    hl: HighlightState,
}

thread_local! {
    static BLOCKS: RefCell<HashMap<String, Block>> = RefCell::new(HashMap::new());
}

/// `code` as kinds: (byte length, kind) runs covering all of it. `id` names
/// the block across frames, so a streaming block is parsed only where it grew.
pub fn spans(id: &str, syntax: &'static SyntaxReference, code: &str) -> Vec<(usize, Kind)> {
    BLOCKS.with(|blocks| {
        let mut blocks = blocks.borrow_mut();
        let resume = blocks.get(id).is_some_and(|b| {
            b.syntax == syntax.name.as_str() && code.starts_with(&b.text[..b.done])
        });
        if !resume {
            if blocks.len() >= MAX_BLOCKS {
                blocks.clear();
            }
            let highlighter = Highlighter::new(theme());
            blocks.insert(
                id.to_string(),
                Block {
                    syntax: syntax.name.as_str(),
                    text: String::new(),
                    spans: Vec::new(),
                    done: 0,
                    done_spans: 0,
                    lines: 0,
                    parse: ParseState::new(syntax),
                    hl: HighlightState::new(&highlighter, ScopeStack::new()),
                },
            );
        }
        let block = blocks.get_mut(id).expect("inserted above");
        if block.text != code {
            block.extend(code);
        }
        block.spans.clone()
    })
}

impl Block {
    fn extend(&mut self, code: &str) {
        let highlighter = Highlighter::new(theme());
        let set = syntaxes();
        self.spans.truncate(self.done_spans);
        let mut at = self.done;
        while at < code.len() {
            let end = code[at..].find('\n').map_or(code.len(), |i| at + i + 1);
            let line = &code[at..end];
            let complete = line.ends_with('\n');
            if self.lines >= MAX_LINES {
                push(&mut self.spans, self.done_spans, code.len() - at, Kind::Plain);
                break;
            }
            // Runs of a complete line may merge with what came before; a
            // partial line's must not, or the next resume would count them twice.
            let floor = if complete { 0 } else { self.done_spans };
            // A partial last line is parsed on copies: the saved state stays
            // at the line's start for when the rest of it arrives.
            let (mut parse, mut hl);
            let (parse_ref, hl_ref) = if complete {
                (&mut self.parse, &mut self.hl)
            } else {
                parse = self.parse.clone();
                hl = self.hl.clone();
                (&mut parse, &mut hl)
            };
            match parse_ref.parse_line(line, set) {
                Ok(ops) => {
                    for (style, piece) in HighlightIterator::new(hl_ref, &ops, line, &highlighter) {
                        push(&mut self.spans, floor, piece.len(), kind_of(style.foreground));
                    }
                }
                Err(_) => {
                    // A grammar that trips on its input: the rest goes plain.
                    push(&mut self.spans, self.done_spans, code.len() - at, Kind::Plain);
                    self.lines = MAX_LINES;
                    break;
                }
            }
            if complete {
                self.lines += 1;
                self.done = end;
                self.done_spans = self.spans.len();
            }
            at = end;
        }
        self.text.clear();
        self.text.push_str(code);
    }
}

/// Append a run, merging it into the last one when the kind is the same and
/// that run is at or past `floor`.
fn push(spans: &mut Vec<(usize, Kind)>, floor: usize, len: usize, kind: Kind) {
    if len == 0 {
        return;
    }
    let mergeable = spans.len() > floor;
    match spans.last_mut() {
        Some((n, k)) if *k == kind && mergeable => *n += len,
        _ => spans.push((len, kind)),
    }
}

/// The colour a kind is drawn in under the theme worn now: the terminal's
/// sixteen, so code in a chat matches code in a pane.
pub fn color(kind: Kind) -> u32 {
    let c = theme::current();
    let ansi = &c.ansi;
    match kind {
        Kind::Plain | Kind::Operator => c.text,
        Kind::Punctuation => c.text_2,
        Kind::Comment => c.dim,
        Kind::String | Kind::Inserted => ansi[2],
        Kind::Number | Kind::Constant | Kind::Attribute => ansi[3],
        Kind::Function | Kind::Heading => ansi[4],
        Kind::Keyword => ansi[5],
        Kind::Type => ansi[6],
        Kind::Tag | Kind::Deleted => ansi[1],
    }
}

/// Text runs for `spans` in `family`, coloured for the theme worn now.
pub fn runs(spans: &[(usize, Kind)], family: &'static str) -> Vec<TextRun> {
    let font = gpui::font(family);
    spans
        .iter()
        .map(|&(len, kind)| TextRun {
            len,
            font: font.clone(),
            color: rgb(color(kind)).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect()
}

/// `spans` cut at each line of `code`: for every line, byte ranges within it
/// (its newline left out) and their kinds.
pub fn by_line(code: &str, spans: &[(usize, Kind)]) -> Vec<Vec<(Range<usize>, Kind)>> {
    let mut lines = vec![Vec::new()];
    let (mut line_start, mut at) = (0, 0);
    for &(len, kind) in spans {
        let end = at + len;
        let mut from = at;
        while let Some(nl) = code[from..end].find('\n') {
            let nl = from + nl;
            if nl > from {
                lines.last_mut().expect("never empty").push((from - line_start..nl - line_start, kind));
            }
            lines.push(Vec::new());
            line_start = nl + 1;
            from = nl + 1;
        }
        if end > from {
            lines.last_mut().expect("never empty").push((from - line_start..end - line_start, kind));
        }
        at = end;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = "// note\nfn main() {\n    let s = \"hi\"; // tail\n    println!(\"{s} {}\", 42);\n}\n";

    fn kinds(code: &str, spans: &[(usize, Kind)]) -> Vec<(String, Kind)> {
        let mut at = 0;
        let mut out = Vec::new();
        for &(len, kind) in spans {
            let t = code[at..at + len].trim();
            if !t.is_empty() && kind != Kind::Plain && kind != Kind::Punctuation {
                out.push((t.to_string(), kind));
            }
            at += len;
        }
        assert_eq!(at, code.len(), "the spans cover the code exactly");
        out
    }

    #[test]
    fn fences_and_files_find_their_grammar() {
        for lang in ["rust", "rs", "ts", "tsx", "jsx", "python", "sh", "zsh", "shell", "go", "swift", "toml", "json", "yaml", "diff", "c++", "dockerfile"] {
            assert!(for_lang(lang).is_some(), "{lang}");
        }
        for lang in ["", "text", "output", "no-such-language"] {
            assert!(for_lang(lang).is_none(), "{lang}");
        }
        assert_eq!(for_lang("rust {linenos}").map(|s| s.name.as_str()), Some("Rust"));
        assert_eq!(for_path("/a/b/main.rs").map(|s| s.name.as_str()), Some("Rust"));
        assert_eq!(for_path("Makefile").map(|s| s.name.as_str()), Some("Makefile"));
        assert!(for_path("notes.unknownext").is_none());
    }

    #[test]
    fn tokens_get_their_kinds() {
        let k = kinds(RUST, &spans("t-kinds", for_lang("rust").unwrap(), RUST));
        for want in [
            ("// note", Kind::Comment),
            ("fn", Kind::Keyword),
            ("main", Kind::Function),
            ("let", Kind::Keyword),
            ("// tail", Kind::Comment),
            ("println!", Kind::Function),
            ("42", Kind::Number),
        ] {
            assert!(k.contains(&(want.0.to_string(), want.1)), "{want:?} in {k:?}");
        }
        assert!(k.iter().any(|(t, kind)| *kind == Kind::String && t.contains("hi")), "{k:?}");
    }

    /// A block arriving a few bytes at a time ends up exactly as one parsed
    /// whole, at every step.
    #[test]
    fn a_streaming_block_matches_a_fresh_parse() {
        let rust = for_lang("rust").unwrap();
        for end in (1..=RUST.len()).filter(|i| RUST.is_char_boundary(*i)) {
            let part = &RUST[..end];
            assert_eq!(
                spans("t-stream", rust, part),
                spans(&format!("t-fresh-{end}"), rust, part),
                "at {end}"
            );
        }
        // An edit, not an append, starts over.
        let edited = RUST.replace("main", "start");
        assert_eq!(spans("t-stream", rust, &edited), spans("t-edited", rust, &edited));
    }

    #[test]
    fn lines_are_cut_without_their_newlines() {
        let code = "a = 1\nb\n";
        let spans = [(2, Kind::Plain), (1, Kind::Operator), (3, Kind::Number), (2, Kind::Plain)];
        let lines = by_line(code, &spans);
        assert_eq!(
            lines,
            vec![
                vec![(0..2, Kind::Plain), (2..3, Kind::Operator), (3..5, Kind::Number)],
                vec![(0..1, Kind::Plain)],
                vec![],
            ]
        );
    }

    #[test]
    fn colours_follow_the_theme() {
        let dark = theme::Theme::Black.colors(true).ansi[5];
        let light = theme::Theme::Light.colors(false).ansi[5];
        assert_ne!(dark, light);
        assert!([dark, light].contains(&color(Kind::Keyword)));
    }
}
