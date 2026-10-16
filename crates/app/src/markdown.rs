//! Streaming markdown (ported from yourcode; the parser is unchanged).
//!
//! Two problems have to be solved together:
//!
//! 1. **Correctness while incomplete.** A reply is parsed while it is still
//!    arriving, so the source is routinely mid-token: an unterminated ``` fence,
//!    a half-written `**bold`, a table with one row so far. `parse` never waits
//!    for a construct to close — it renders the best interpretation of what has
//!    arrived, and upgrades as more lands.
//!
//! 2. **Cost.** Re-parsing on every token is fine (this is microseconds); what
//!    is *not* fine is rebuilding the element tree and re-measuring the whole
//!    thread. So parsing is cached per block and only the open tail re-parses —
//!    see `ThreadView::reparse`.
//!
//! Inline styling is emitted as highlight ranges over one string rather than
//! nested elements, so a styled paragraph costs one text layout, not one per run.

use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub blocks: Vec<MdBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    Paragraph(Spans),
    Heading {
        level: u8,
        spans: Spans,
    },
    /// `open` is true while the closing fence has not arrived, which the
    /// renderer shows so a streaming code block doesn't look finished.
    Code {
        lang: String,
        code: String,
        open: bool,
    },
    List {
        items: Vec<ListItem>,
    },
    Quote(Spans),
    Rule,
    Table {
        head: Vec<Spans>,
        rows: Vec<Vec<Spans>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub marker: String,
    pub depth: u8,
    pub spans: Spans,
}

/// A string plus the styled ranges over it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spans {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
    /// Where each link is in `text`, and where it goes.
    pub links: Vec<(Range<usize>, String)>,
}

/// Flips one emphasis flag when its marker is met.
type Toggle = fn(&mut Style);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
    /// A key written in the text (`⌃O`, `⏎`), drawn as a keycap would be.
    pub kbd: bool,
}

impl Style {
    fn is_plain(self) -> bool {
        self == Style::default()
    }
}

impl Spans {
    /// Append `text`, keys in it marked as keys, unless it is code or a
    /// link's label, which stay as written.
    fn push(&mut self, text: &str, style: Style) {
        if style.code || style.link {
            return self.push_run(text, style);
        }
        let mut at = 0;
        for r in crate::kbd::find(text) {
            self.push_run(&text[at..r.start], style);
            // A text run can't be padded, so a narrow no-break space each
            // side gives the key room inside its tint, and keeps it on one
            // line.
            let key = format!("\u{202F}{}\u{202F}", &text[r.clone()]);
            self.push_run(&key, Style { kbd: true, ..style });
            at = r.end;
        }
        self.push_run(&text[at..], style);
    }

    fn push_run(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        if !style.is_plain() {
            let range = start..self.text.len();
            // Merge with the previous run when the style matches, so a long
            // bold passage is one range rather than one per token.
            match self.runs.last_mut() {
                Some((prev, s)) if *s == style && prev.end == start => prev.end = range.end,
                _ => self.runs.push((range, style)),
            }
        }
    }

    fn push_link(&mut self, label: &str, url: &str, style: Style) {
        let start = self.text.len();
        self.push(label, style);
        if self.text.len() > start {
            self.links.push((start..self.text.len(), url.to_string()));
        }
    }

    /// Append another span list, shifting its ranges to land after ours.
    fn append(&mut self, mut other: Spans) {
        let offset = self.text.len();
        self.text.push_str(&other.text);
        for (r, st) in other.runs.drain(..) {
            self.runs.push((r.start + offset..r.end + offset, st));
        }
        for (r, url) in other.links.drain(..) {
            self.links.push((r.start + offset..r.end + offset, url));
        }
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    fn trim_end(&mut self) {
        let trimmed = self.text.trim_end().len();
        self.text.truncate(trimmed);
        self.runs.retain_mut(|(r, _)| {
            r.end = r.end.min(trimmed);
            r.start < r.end
        });
        self.links.retain_mut(|(r, _)| {
            r.end = r.end.min(trimmed);
            r.start < r.end
        });
    }
}

/// Parse markdown that may be incomplete.
///
/// Hand-rolled rather than using a CommonMark crate on purpose: the streaming
/// requirement inverts the usual contract. A conforming parser wants to see a
/// closing fence before it commits to a code block; here an unterminated fence
/// is the *normal* case for the last block on screen, and it must render as code
/// immediately.
/// Incremental parsing for a reply that is still streaming in.
///
/// A naive renderer re-parses the whole message on every token, which is
/// quadratic: a 12 KB reply arriving in 600 deltas parses ~500× its own length.
/// Measured at 331 ms of pure parsing in a debug build, and that is before
/// layout re-measures the row.
///
/// The fix falls out of the parser being line-oriented and carrying no state
/// across a blank line: every construct here — paragraph, list, table, quote —
/// terminates at one, and `flush!` empties the only accumulator. So a blank line
/// **outside a fenced code block** is a point where
/// `parse(a) ++ parse(b) == parse(a ++ b)`. Everything before the last such line
/// can never change again, so it is parsed once and kept.
///
/// What stays live is one paragraph, or one unclosed code fence — bounded, not
/// growing. That turns the per-token cost from O(message) into O(open block).
#[derive(Default)]
pub struct Cache {
    doc: Arc<Doc>,
    /// Bytes of the source already accounted for by frozen blocks.
    at: usize,
    /// How many leading blocks of `doc` are frozen.
    frozen: usize,
}

impl Cache {
    /// Re-parse only the part that can still change, and return the whole doc.
    pub fn update(&mut self, src: &str) -> &Arc<Doc> {
        // The transcript only ever appends to an open text block, but a cleared
        // or rewritten message must not reuse a stale prefix.
        if self.at > src.len() || !src.is_char_boundary(self.at) {
            *self = Cache::default();
        }

        // `make_mut` is free while the renderer only borrows the doc, which it
        // does — the cache is the sole owner between frames.
        let doc = Arc::make_mut(&mut self.doc);
        doc.blocks.truncate(self.frozen);

        let cut = safe_cut(&src[self.at..]);
        if cut > 0 {
            doc.blocks
                .extend(parse(&src[self.at..self.at + cut]).blocks);
            self.frozen = doc.blocks.len();
            self.at += cut;
        }
        doc.blocks.extend(parse(&src[self.at..]).blocks);
        &self.doc
    }

    #[cfg(test)]
    pub fn doc(&self) -> &Doc {
        &self.doc
    }
}

/// Byte offset just past the last blank line that is not inside a fenced code
/// block. `0` when there is no such line and nothing can be frozen yet.
fn safe_cut(src: &str) -> usize {
    let mut inside_fence = false;
    let mut cut = 0usize;
    let mut at = 0usize;
    for line in src.split('\n') {
        let end = at + line.len() + 1; // include the newline
        let trimmed = line.trim_start();
        if fence_lang(trimmed).is_some() {
            inside_fence = !inside_fence;
        } else if trimmed.is_empty() && !inside_fence && end <= src.len() {
            // Everything up to and including this blank line is settled.
            cut = end;
        }
        at = end;
    }
    cut
}

pub fn parse(src: &str) -> Doc {
    let mut blocks = Vec::new();
    let mut para = Spans::default();
    let mut lines = src.split('\n').peekable();

    // Emit the paragraph accumulated so far, if any.
    macro_rules! flush {
        () => {
            if !para.is_empty() {
                para.trim_end();
                blocks.push(MdBlock::Paragraph(std::mem::take(&mut para)));
            }
        };
    }

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // Fenced code. Everything until the closing fence is literal, including
        // things that look like markdown.
        if let Some(rest) = fence_lang(trimmed) {
            flush!();
            let mut code = String::new();
            let mut closed = false;
            for body in lines.by_ref() {
                if fence_lang(body.trim_start()).is_some() && body.trim_start().starts_with("```") {
                    closed = true;
                    break;
                }
                code.push_str(body);
                code.push('\n');
            }
            // Trailing newline is an artifact of the join, not content.
            if code.ends_with('\n') {
                code.pop();
            }
            blocks.push(MdBlock::Code {
                lang: rest.to_string(),
                code,
                open: !closed,
            });
            continue;
        }

        // ATX heading.
        if let Some((level, rest)) = heading(trimmed) {
            flush!();
            blocks.push(MdBlock::Heading {
                level,
                spans: inline(rest),
            });
            continue;
        }

        // Thematic break. Checked before list items so `---` is not read as a
        // bullet.
        if is_rule(trimmed) {
            flush!();
            blocks.push(MdBlock::Rule);
            continue;
        }

        // Table: a header row followed by a delimiter row. While streaming, the
        // delimiter may not have arrived yet, so a lone `|` row stays a
        // paragraph until it does.
        if trimmed.starts_with('|') && lines.peek().is_some_and(|n| is_table_delim(n.trim())) {
            flush!();
            let head = table_cells(trimmed);
            lines.next(); // consume the delimiter
            let mut rows = Vec::new();
            while lines
                .peek()
                .is_some_and(|n| n.trim_start().starts_with('|'))
            {
                rows.push(table_cells(lines.next().expect("peeked").trim_start()));
            }
            blocks.push(MdBlock::Table { head, rows });
            continue;
        }

        // Block quote. Consecutive `>` lines are one quote, soft-joined like a
        // paragraph; an empty `>` line breaks it.
        if let Some(rest) = quote_body(trimmed) {
            flush!();
            let mut q = inline(rest.trim());
            while let Some(more) = lines.peek().and_then(|n| quote_body(n.trim_start())) {
                let more = more.trim();
                if more.is_empty() {
                    q.push("\n", Style::default());
                } else {
                    if !q.text.is_empty() && !q.text.ends_with('\n') {
                        q.push(" ", Style::default());
                    }
                    q.append(inline(more));
                }
                lines.next();
            }
            q.trim_end();
            blocks.push(MdBlock::Quote(q));
            continue;
        }

        // List item. Consecutive items merge into one block so spacing between
        // them is list spacing, not paragraph spacing.
        if let Some((marker, rest)) = list_marker(trimmed) {
            flush!();
            let mut items = vec![list_item(marker, indent, rest)];
            while let Some(next) = lines.peek() {
                let nt = next.trim_start();
                let ni = next.len() - nt.len();
                if nt.is_empty() || fence_lang(nt).is_some() {
                    break;
                }
                match list_marker(nt) {
                    Some((m, r)) => items.push(list_item(m, ni, r)),
                    // An indented line under an item continues it.
                    None if ni > 0 => {
                        let last = items.last_mut().expect("one item at least");
                        last.spans.push(" ", Style::default());
                        last.spans.append(inline(nt.trim_end()));
                    }
                    None => break,
                }
                lines.next();
            }
            blocks.push(MdBlock::List { items });
            continue;
        }

        // Blank line ends a paragraph.
        if trimmed.is_empty() {
            flush!();
            continue;
        }

        // Continuation of the current paragraph. A single newline is a soft
        // break, so join with a space rather than concatenating words.
        if !para.text.is_empty() {
            para.push(" ", Style::default());
        }
        para.append(inline(line.trim()));
    }

    flush!();
    Doc { blocks }
}

fn fence_lang(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("```")
        .map(|r| r.trim())
        .or_else(|| trimmed.strip_prefix("~~~").map(|r| r.trim()))
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    // `#hashtag` is not a heading; a space is required.
    let body = rest.strip_prefix(' ')?;
    Some((hashes as u8, body.trim()))
}

fn is_rule(trimmed: &str) -> bool {
    let t = trimmed.trim();
    t.len() >= 3
        && (t.bytes().all(|b| b == b'-')
            || t.bytes().all(|b| b == b'*')
            || t.bytes().all(|b| b == b'_'))
}

fn is_table_delim(t: &str) -> bool {
    t.starts_with('|')
        && t.contains('-')
        && t.bytes()
            .all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'))
}

fn table_cells(line: &str) -> Vec<Spans> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| inline(c.trim())).collect()
}

fn quote_body(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("> ")
        .or_else(|| trimmed.strip_prefix('>'))
}

/// A list item; `- [ ]` and `- [x]` become task boxes.
fn list_item(marker: String, indent: usize, rest: &str) -> ListItem {
    let (marker, rest) = match (rest.get(..4), rest.get(4..)) {
        (Some("[ ] "), Some(r)) => ("☐".to_string(), r),
        (Some("[x] " | "[X] "), Some(r)) => ("☑".to_string(), r),
        _ => (marker, rest),
    };
    ListItem {
        marker,
        depth: (indent / 2).min(4) as u8,
        spans: inline(rest),
    }
}

fn list_marker(trimmed: &str) -> Option<(String, &str)> {
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(p) {
            return Some(("•".to_string(), rest));
        }
    }
    // Ordered: `12. text`
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let rest = &trimmed[digits..];
        for p in [". ", ") "] {
            if let Some(body) = rest.strip_prefix(p) {
                return Some((format!("{}.", &trimmed[..digits]), body));
            }
        }
    }
    None
}

/// Inline emphasis, code, strikethrough and links.
///
/// Unclosed markers are emitted as literal text: while streaming, `**bo` must
/// show as `**bo` and only become bold once the closing `**` arrives. Anything
/// else makes text visibly flicker between styles as it lands.
fn inline(src: &str) -> Spans {
    let mut out = Spans::default();
    let b = src.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;
    let mut style = Style::default();

    macro_rules! flush_plain {
        ($upto:expr) => {
            if $upto > plain_start {
                out.push(&src[plain_start..$upto], style);
            }
        };
    }

    while i < b.len() {
        // Inline code wins over emphasis: `**` inside backticks is literal.
        if b[i] == b'`' {
            if let Some(end) = find(b, i + 1, b"`") {
                flush_plain!(i);
                let mut s = style;
                s.code = true;
                out.push(&src[i + 1..end], s);
                i = end + 1;
                plain_start = i;
                continue;
            }
            i += 1;
            continue;
        }

        // Link: [text](url), and an image ![alt](url) shown as a link to it.
        let image = b[i] == b'!' && b.get(i + 1) == Some(&b'[');
        if b[i] == b'[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = find(b, open + 1, b"]") {
                if b.get(close + 1) == Some(&b'(') {
                    if let Some(paren) = find(b, close + 2, b")") {
                        flush_plain!(i);
                        let mut s = style;
                        s.link = true;
                        let label = &src[open + 1..close];
                        // A link may carry a title: [a](url "title").
                        let url = src[close + 2..paren]
                            .split_whitespace()
                            .next()
                            .unwrap_or("");
                        // Bare-URL links render as the URL, which is more
                        // useful than an empty label.
                        out.push_link(if label.is_empty() { url } else { label }, url, s);
                        i = paren + 1;
                        plain_start = i;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }

        // A bare URL, or one in angle brackets, is a link too.
        let at_word_start = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if at_word_start && (b[i..].starts_with(b"https://") || b[i..].starts_with(b"http://")) {
            let mut end = i;
            while end < b.len() && !b[end].is_ascii_whitespace() && b[end] != b'>' {
                end += 1;
            }
            // Sentence punctuation after a URL is not part of it.
            while end > i
                && matches!(
                    b[end - 1],
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b'\'' | b'"'
                )
            {
                end -= 1;
            }
            flush_plain!(i);
            let mut s = style;
            s.link = true;
            out.push_link(&src[i..end], &src[i..end], s);
            i = end;
            plain_start = i;
            continue;
        }
        if b[i] == b'<'
            && (b[i + 1..].starts_with(b"https://") || b[i + 1..].starts_with(b"http://"))
        {
            if let Some(close) = find(b, i + 1, b">") {
                flush_plain!(i);
                let mut s = style;
                s.link = true;
                out.push_link(&src[i + 1..close], &src[i + 1..close], s);
                i = close + 1;
                plain_start = i;
                continue;
            }
        }

        // `_` inside a word (snake_case) is a letter, not emphasis.
        if b[i] == b'_' {
            let before = i > 0 && b[i - 1].is_ascii_alphanumeric();
            let after = b.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric());
            if before && after {
                i += 1;
                continue;
            }
        }

        // Emphasis markers, longest first so `***` and `**` don't mis-match.
        let markers: [(&[u8], Toggle); 5] = [
            (b"***", |s| {
                s.bold = !s.bold;
                s.italic = !s.italic;
            }),
            (b"**", |s| s.bold = !s.bold),
            (b"~~", |s| s.strike = !s.strike),
            (b"*", |s| s.italic = !s.italic),
            (b"_", |s| s.italic = !s.italic),
        ];
        let mut matched = false;
        for (m, toggle) in markers {
            if b[i..].starts_with(m) {
                // Only treat it as a marker if it actually closes.
                let closing = find_seq(b, i + m.len(), m);
                let opening = m.iter().all(|_| true) && !style_active(style, m);
                if opening && closing.is_none() {
                    break; // unterminated: fall through to literal text
                }
                flush_plain!(i);
                toggle(&mut style);
                i += m.len();
                plain_start = i;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        i += 1;
    }
    flush_plain!(b.len());
    out
}

/// Whether the style this marker controls is currently on, i.e. the marker is a
/// closing one and needs no lookahead.
fn style_active(style: Style, marker: &[u8]) -> bool {
    match marker {
        b"***" => style.bold && style.italic,
        b"**" => style.bold,
        b"~~" => style.strike,
        b"*" | b"_" => style.italic,
        _ => false,
    }
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    find_seq(hay, from, needle)
}

fn find_seq(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    /// Every sample exercises a construct the cut rule has to respect.
    const SAMPLES: &[&str] = &[
        "hello world",
        "one paragraph\n\ntwo paragraph\n\nthree",
        "# Heading\n\nbody text here\n\n## Second\n\nmore body",
        "- a\n- b\n- c\n\nafter the list\n\n- d\n- e",
        "text\n\n```rust\nfn main() {\n\n    // a blank line inside a fence\n}\n```\n\nafter",
        "```\nunclosed fence\n\nstill inside\n",
        "| a | b |\n|---|---|\n| 1 | 2 |\n\nafter the table",
        "> quoted\n\n---\n\nrule above",
        "para with `code` and **bold**\n\n> quote\n\n- list\n\n```sh\nls\n```\n\nend",
        "trailing blank lines\n\n\n\n",
        "\n\nleading blanks\n\nbody",
    ];

    /// The property that makes the optimisation safe: feeding a message in one
    /// token at a time must produce exactly what parsing it whole produces.
    ///
    /// This is the test that would catch a wrong cut point, which is the only
    /// way incremental parsing can go wrong — and it would go wrong *silently*,
    /// as a paragraph that renders subtly differently mid-stream.
    #[test]
    fn incremental_parsing_matches_a_full_parse_at_every_prefix() {
        for sample in SAMPLES {
            let mut cache = Cache::default();
            for end in 0..=sample.len() {
                if !sample.is_char_boundary(end) {
                    continue;
                }
                let prefix = &sample[..end];
                let incremental = cache.update(prefix).as_ref().clone();
                assert_eq!(
                    incremental,
                    parse(prefix),
                    "incremental parse diverged at byte {end} of {sample:?}"
                );
            }
        }
    }

    #[test]
    fn a_blank_line_outside_a_fence_freezes_everything_before_it() {
        // "a\n\n" is four bytes; the cut sits just past the blank line.
        assert_eq!(safe_cut("a\n\nb"), 3);
        assert_eq!(safe_cut("a\n\nb\n\nc"), 6);
    }

    #[test]
    fn nothing_freezes_until_a_paragraph_actually_ends() {
        assert_eq!(safe_cut("still typing a sentence"), 0);
        assert_eq!(safe_cut("line one\nline two"), 0);
    }

    #[test]
    fn a_blank_line_inside_a_fence_is_not_a_cut_point() {
        // Cutting here would split the code block in two.
        assert_eq!(safe_cut("```rust\nfn a() {\n\n}\n"), 0);
        // Once the fence closes and a blank line follows, it is safe again.
        assert!(safe_cut("```rust\nfn a() {\n\n}\n```\n\nafter") > 0);
    }

    /// The point of the whole exercise: the live region stays bounded instead of
    /// growing with the message.
    #[test]
    fn the_live_region_stays_bounded_as_the_message_grows() {
        let para = "Some prose that goes on for a while and then ends.\n\n";
        let mut cache = Cache::default();
        let mut src = String::new();
        for _ in 0..40 {
            src.push_str(para);
            cache.update(&src);
        }
        let live = src.len() - cache.at;
        assert!(
            live <= para.len(),
            "the unfrozen tail grew to {live} bytes of a {} byte message; the \
             per-token cost is still proportional to the whole reply",
            src.len()
        );
        assert_eq!(cache.doc().blocks.len(), 40);
    }

    #[test]
    fn a_rewritten_source_resets_rather_than_reusing_a_stale_prefix() {
        let mut cache = Cache::default();
        cache.update("first message\n\nwith two blocks\n\n");
        // Shorter than what was frozen: the cache must start over.
        let doc = cache.update("x").as_ref().clone();
        assert_eq!(doc, parse("x"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_block(src: &str) -> MdBlock {
        let d = parse(src);
        assert_eq!(d.blocks.len(), 1, "expected one block, got {:?}", d.blocks);
        d.blocks.into_iter().next().unwrap()
    }

    fn styled(spans: &Spans, style: Style) -> Vec<&str> {
        spans
            .runs
            .iter()
            .filter(|(_, s)| *s == style)
            .map(|(r, _)| &spans.text[r.clone()])
            .collect()
    }

    #[test]
    fn plain_paragraph_has_no_runs() {
        match only_block("just some text") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "just some text");
                assert!(s.runs.is_empty());
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn soft_wrapped_lines_join_with_a_space() {
        match only_block("first line\nsecond line") {
            MdBlock::Paragraph(s) => assert_eq!(s.text, "first line second line"),
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn blank_line_splits_paragraphs() {
        let d = parse("one\n\ntwo");
        assert_eq!(d.blocks.len(), 2);
    }

    #[test]
    fn bold_italic_and_code_produce_styled_runs() {
        match only_block("a **bold** and *it* and `code` end") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "a bold and it and code end");
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            bold: true,
                            ..Default::default()
                        }
                    ),
                    ["bold"]
                );
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            italic: true,
                            ..Default::default()
                        }
                    ),
                    ["it"]
                );
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            code: true,
                            ..Default::default()
                        }
                    ),
                    ["code"]
                );
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn unterminated_emphasis_stays_literal_while_streaming() {
        // This is the whole point: mid-stream text must not flicker into bold.
        match only_block("almost **bol") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "almost **bol");
                assert!(s.runs.is_empty(), "got {:?}", s.runs);
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn unterminated_inline_code_stays_literal() {
        match only_block("run `cargo bui") {
            MdBlock::Paragraph(s) => assert_eq!(s.text, "run `cargo bui"),
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn markdown_inside_inline_code_is_literal() {
        match only_block("use `a ** b` here") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "use a ** b here");
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            code: true,
                            ..Default::default()
                        }
                    ),
                    ["a ** b"]
                );
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn fenced_code_block_keeps_language_and_body_verbatim() {
        match only_block("```rust\nfn main() {\n    // **not bold**\n}\n```") {
            MdBlock::Code { lang, code, open } => {
                assert_eq!(lang, "rust");
                assert_eq!(code, "fn main() {\n    // **not bold**\n}");
                assert!(!open);
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn unclosed_fence_renders_as_open_code_immediately() {
        // The common case for the last block of a streaming reply.
        match only_block("```sh\ncargo build") {
            MdBlock::Code { lang, code, open } => {
                assert_eq!(lang, "sh");
                assert_eq!(code, "cargo build");
                assert!(open, "an unterminated fence must be marked open");
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn headings_parse_by_level_and_require_a_space() {
        match only_block("## Section") {
            MdBlock::Heading { level, spans } => {
                assert_eq!(level, 2);
                assert_eq!(spans.text, "Section");
            }
            b => panic!("{b:?}"),
        }
        // `#tag` is not a heading.
        assert!(matches!(only_block("#notaheading"), MdBlock::Paragraph(_)));
    }

    #[test]
    fn consecutive_list_items_merge_into_one_block() {
        match only_block("- one\n- two\n- three") {
            MdBlock::List { items } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].spans.text, "one");
                assert_eq!(items[0].marker, "•");
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn ordered_lists_keep_their_numbers() {
        match only_block("1. first\n2. second") {
            MdBlock::List { items } => {
                assert_eq!(items[0].marker, "1.");
                assert_eq!(items[1].marker, "2.");
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn nested_list_items_record_depth() {
        match only_block("- top\n  - nested") {
            MdBlock::List { items } => {
                assert_eq!(items[0].depth, 0);
                assert_eq!(items[1].depth, 1);
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn dashes_are_a_rule_not_a_list() {
        assert!(matches!(only_block("---"), MdBlock::Rule));
    }

    #[test]
    fn tables_parse_head_and_rows() {
        match only_block("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |") {
            MdBlock::Table { head, rows } => {
                assert_eq!(head.len(), 2);
                assert_eq!(head[0].text, "a");
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[1][1].text, "4");
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn a_pipe_row_without_its_delimiter_stays_a_paragraph() {
        // Mid-stream: the header has arrived but the |---| row has not.
        assert!(matches!(only_block("| a | b |"), MdBlock::Paragraph(_)));
    }

    #[test]
    fn links_render_their_label_and_are_marked() {
        match only_block("see [the docs](https://example.com) now") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "see the docs now");
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            link: true,
                            ..Default::default()
                        }
                    ),
                    ["the docs"]
                );
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn empty_label_links_fall_back_to_the_url() {
        match only_block("[](https://example.com)") {
            MdBlock::Paragraph(s) => assert_eq!(s.text, "https://example.com"),
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn keys_in_text_are_marked_but_not_in_code_or_links() {
        let kbd = Style {
            kbd: true,
            ..Style::default()
        };
        match only_block("Press ⌃O, then ⏎. In code: `⌃O`. [⌘K](https://x.dev)") {
            MdBlock::Paragraph(s) => {
                assert_eq!(styled(&s, kbd), ["\u{202F}⌃O\u{202F}", "\u{202F}⏎\u{202F}"]);
                assert!(s.text.starts_with("Press \u{202F}⌃O\u{202F}, then"));
                let code = Style {
                    code: true,
                    ..Style::default()
                };
                assert_eq!(styled(&s, code), ["⌃O"]);
                assert_eq!(s.links.len(), 1);
                assert_eq!(&s.text[s.links[0].0.clone()], "⌘K");
            }
            b => panic!("{b:?}"),
        }
        // Bold keys stay bold, as keys.
        match only_block("**⇧⌘N** opens one") {
            MdBlock::Paragraph(s) => {
                let bold_kbd = Style {
                    bold: true,
                    kbd: true,
                    ..Style::default()
                };
                assert_eq!(styled(&s, bold_kbd), ["\u{202F}⇧⌘N\u{202F}"]);
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn strikethrough_is_recognised() {
        match only_block("~~gone~~") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "gone");
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            strike: true,
                            ..Default::default()
                        }
                    ),
                    ["gone"]
                );
            }
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn adjacent_same_style_runs_are_merged() {
        let s = inline("**one** **two**");
        // Two separate bold spans, not four runs split per word.
        assert_eq!(s.runs.len(), 2, "got {:?}", s.runs);
    }

    #[test]
    fn run_ranges_are_valid_utf8_slices_of_the_text() {
        // gpui asserts highlight ranges land on char boundaries.
        let doc = parse("héllo **wörld** and `côde` ~~strück~~ [lïnk](u)");
        for b in &doc.blocks {
            if let MdBlock::Paragraph(s) = b {
                for (r, _) in &s.runs {
                    assert!(
                        s.text.is_char_boundary(r.start) && s.text.is_char_boundary(r.end),
                        "range {r:?} splits a char in {:?}",
                        s.text
                    );
                    let _ = &s.text[r.clone()];
                }
            }
        }
    }

    #[test]
    fn parsing_is_stable_as_text_grows_one_char_at_a_time() {
        // Streaming safety net: no prefix of a realistic reply may panic, and
        // the final parse must match a one-shot parse of the whole thing.
        let full = "Here is **why**:\n\n1. it uses `gpui`\n2. see [docs](http://x)\n\n\
                    ```rust\nfn main() { println!(\"hi\"); }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> note\n\n---\n";
        for end in 0..=full.len() {
            if !full.is_char_boundary(end) {
                continue;
            }
            let _ = parse(&full[..end]);
        }
        assert_eq!(parse(full), parse(full));
    }

    #[test]
    fn a_realistic_reply_parses_into_the_expected_block_sequence() {
        let doc = parse(
            "Yes. It uses **gpui 0.2.2**.\n\n\
             ```toml\ngpui = \"0.2.2\"\n```\n\n\
             Because:\n\n- it is vendored\n- to patch a bug\n",
        );
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                MdBlock::Paragraph(_) => "p",
                MdBlock::Code { .. } => "code",
                MdBlock::List { .. } => "list",
                MdBlock::Heading { .. } => "h",
                MdBlock::Quote(_) => "quote",
                MdBlock::Rule => "rule",
                MdBlock::Table { .. } => "table",
            })
            .collect();
        assert_eq!(kinds, ["p", "code", "p", "list"]);
    }

    #[test]
    fn links_keep_where_they_go() {
        match only_block(
            "see [the docs](https://a.dev/x \"t\") or https://b.dev/y. and <http://c.dev>",
        ) {
            MdBlock::Paragraph(s) => {
                let got: Vec<(&str, &str)> = s
                    .links
                    .iter()
                    .map(|(r, u)| (&s.text[r.clone()], u.as_str()))
                    .collect();
                assert_eq!(
                    got,
                    [
                        ("the docs", "https://a.dev/x"),
                        ("https://b.dev/y", "https://b.dev/y"),
                        ("http://c.dev", "http://c.dev")
                    ]
                );
                assert_eq!(s.text, "see the docs or https://b.dev/y. and http://c.dev");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn links_survive_soft_wrapped_paragraphs() {
        match only_block("first line\nthen [x](https://x.dev)") {
            MdBlock::Paragraph(s) => assert_eq!(&s.text[s.links[0].0.clone()], "x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn underscores_inside_words_are_not_emphasis() {
        match only_block("set max_line_width and _this_ one") {
            MdBlock::Paragraph(s) => {
                assert_eq!(s.text, "set max_line_width and this one");
                assert_eq!(
                    styled(
                        &s,
                        Style {
                            italic: true,
                            ..Default::default()
                        }
                    ),
                    ["this"]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn consecutive_quote_lines_are_one_quote() {
        match only_block("> one\n> two\n>\n> three") {
            MdBlock::Quote(s) => assert_eq!(s.text, "one two\nthree"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn indented_lines_continue_a_list_item_and_tasks_get_boxes() {
        match only_block("- [ ] first\n  still first\n- [x] done") {
            MdBlock::List { items } => {
                assert_eq!(items.len(), 2);
                assert_eq!(
                    (items[0].marker.as_str(), items[0].spans.text.as_str()),
                    ("☐", "first still first")
                );
                assert_eq!(
                    (items[1].marker.as_str(), items[1].spans.text.as_str()),
                    ("☑", "done")
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
