//! Fuzzy matching for the command palette: the query's characters in order,
//! anywhere in the text, scored so the match you meant ranks first. Starts of
//! words and runs of consecutive characters score high, gaps cost a little,
//! and a match at the very start (or of the whole text) beats everything.

/// Where the query matched (byte offsets into `text`, one per query
/// character) and how well. `None` when some query character is missing.
pub fn score(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|(_, c)| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    // Greedy from the left, but each query character prefers a word start
    // further on over a mid-word hit right here, which is what people mean
    // by "gs" in "git status".
    let mut picks = Vec::with_capacity(q.len());
    let mut from = 0;
    for (qi, &qc) in q.iter().enumerate() {
        let remaining = q.len() - qi - 1;
        let mut found = None;
        for i in from..lower.len() {
            if lower[i] != qc {
                continue;
            }
            if found.is_none() {
                found = Some(i);
            }
            if boundary(&chars, i) {
                // Take the word start only if the rest can still match after it.
                if fits(&q[qi + 1..], &lower[i + 1..]) || remaining == 0 {
                    found = Some(i);
                }
                break;
            }
        }
        let i = found?;
        picks.push(i);
        from = i + 1;
    }

    let mut s = 0i32;
    for (k, &i) in picks.iter().enumerate() {
        s += 1;
        if boundary(&chars, i) {
            s += 8;
        }
        if k > 0 {
            let gap = (i - picks[k - 1] - 1) as i32;
            if gap == 0 {
                s += 6;
            } else {
                s -= gap.min(10);
            }
        }
    }
    if picks.first() == Some(&0) {
        s += 12;
    }
    if q.len() == lower.len() {
        s += 20;
    }
    // Shorter texts are more likely what was meant.
    s -= (lower.len() as i32 / 12).min(8);
    Some((s, picks.iter().map(|&i| chars[i].0).collect()))
}

/// Whether `q` occurs in order within `rest`.
fn fits(q: &[char], rest: &[char]) -> bool {
    let mut it = rest.iter();
    q.iter().all(|c| it.any(|r| r == c))
}

/// The start of a word: the text's start, after a separator, or a capital
/// after a lowercase letter (`newTab`).
fn boundary(chars: &[(usize, char)], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = chars[i - 1].1;
    let here = chars[i].1;
    !prev.is_alphanumeric() || (prev.is_lowercase() && here.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, t: &str) -> i32 {
        score(q, t).map_or(i32::MIN, |(s, _)| s)
    }

    #[test]
    fn missing_characters_do_not_match() {
        assert!(score("xyz", "New Terminal").is_none());
        assert!(score("lw", "New Terminal").is_none());
        assert!(score("nt", "New Terminal").is_some());
    }

    #[test]
    fn word_starts_and_runs_rank_first() {
        assert!(s("nt", "New Terminal") > s("nt", "Inspect Element"));
        assert!(s("set", "Settings") > s("set", "Reset zoom"));
        assert!(s("main", "src/main.rs") > s("main", "crates/domain/remain.rs"));
        assert!(s("ws", "workspace.rs") < s("ws", "web search"));
    }

    #[test]
    fn positions_point_at_the_matched_characters() {
        let (_, at) = score("gs", "git status").unwrap();
        assert_eq!(at, vec![0, 4]);
        let (_, at) = score("é", "café").unwrap();
        assert_eq!(at, vec![3]);
    }
}
