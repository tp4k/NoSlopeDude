//! Git's `wildmatch` with `WM_PATHNAME`, byte-wise and case-sensitive,
//! written from git's documented and observed behaviour (`git check-ignore`
//! is the arbiter; the differential harness pins it).
//!
//! The backtracking result is three-valued, as in git: besides a plain
//! no-match there are "abort everything" (the text ran out, so no later
//! alignment can match) and "abort to the nearest enclosing `**`" (a single
//! `*` met a `/`, so only a `**` further out can still help). Those two keep
//! patterns such as `*a` repeated 64 times linear instead of exponential.

use std::collections::HashMap;

const SLASH: u8 = b'/';
const STAR: u8 = b'*';
const BACKSLASH: u8 = b'\\';

/// Verdicts of `walk` calls made by a special `**` run, keyed by pattern and
/// text position. Kept only for a pattern with two or more such runs (one
/// run is linear without it), and created on first use.
struct Memo {
    enabled: bool,
    table: Option<HashMap<(usize, usize), Outcome>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Match,
    NoMatch,
    AbortAll,
    AbortToDoubleStar,
}

/// Whether `pattern` matches the whole of `text` under git's pathname
/// wildmatch rules. Neither side is decoded: both are raw bytes.
#[cfg(test)]
pub(super) fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    wildmatch_with_runs(pattern, text, special_runs(pattern))
}

/// `wildmatch` for a caller that already knows `special_runs(pattern)`.
pub(super) fn wildmatch_with_runs(pattern: &[u8], text: &[u8], runs: usize) -> bool {
    let mut memo = Memo {
        enabled: runs >= 2,
        table: None,
    };
    walk(pattern, 0, text, 0, &mut memo) == Outcome::Match
}

/// `walk(pat, p, text, t)`, computed once per `(p, t)` for the caller.
fn walk_memoized(pat: &[u8], p: usize, text: &[u8], t: usize, memo: &mut Memo) -> Outcome {
    if !memo.enabled {
        return walk(pat, p, text, t, memo);
    }
    if let Some(known) = memo.table.as_ref().and_then(|table| table.get(&(p, t))) {
        return *known;
    }
    let outcome = walk(pat, p, text, t, memo);
    memo.table
        .get_or_insert_with(HashMap::new)
        .insert((p, t), outcome);
    outcome
}

/// Matches `pat[p..]` against `text[t..]`; `p == 0` is the pattern start.
fn walk(pat: &[u8], mut p: usize, text: &[u8], mut t: usize, memo: &mut Memo) -> Outcome {
    while p < pat.len() {
        if pat[p] == STAR {
            return star(pat, p, text, t, memo);
        }
        if t >= text.len() {
            return Outcome::AbortAll;
        }
        match single(pat, p, text[t]) {
            Ok(last) => p = last + 1,
            Err(outcome) => return outcome,
        }
        t += 1;
    }
    if t < text.len() {
        Outcome::NoMatch
    } else {
        Outcome::Match
    }
}

/// The one-byte item at `pat[p]` (not a star) against `byte`: the index of
/// the item's last pattern byte, or the outcome that ends the walk.
fn single(pat: &[u8], p: usize, byte: u8) -> Result<usize, Outcome> {
    let (last, matched) = match pat[p] {
        b'?' => (p, byte != SLASH),
        b'[' => bracket_single(pat, p, byte)?,
        BACKSLASH => (p + 1, pat.get(p + 1) == Some(&byte)),
        literal => (p, literal == byte),
    };
    if matched {
        Ok(last)
    } else {
        Err(Outcome::NoMatch)
    }
}

/// A bracket expression as a one-byte item: a malformed one aborts the
/// whole match, and none ever matches a `/`.
fn bracket_single(pat: &[u8], open: usize, byte: u8) -> Result<(usize, bool), Outcome> {
    let (close, matched) = bracket(pat, open, byte).ok_or(Outcome::AbortAll)?;
    Ok((close, matched && byte != SLASH))
}

/// Whether the star run `pat[start..end]` is a whole-segment `**`: two or
/// more stars after the pattern start or a `/`, before the end, `/` or `\/`.
fn is_special_run(pat: &[u8], start: usize, end: usize) -> bool {
    let at_segment_start = start == 0 || pat[start - 1] == SLASH;
    let at_segment_end = match pat.get(end) {
        None | Some(&SLASH) => true,
        Some(&BACKSLASH) => pat.get(end + 1) == Some(&SLASH),
        Some(_) => false,
    };
    end - start >= 2 && at_segment_start && at_segment_end
}

/// How many whole-segment `**` runs `pat` holds, as `star` sees them.
pub(super) fn special_runs(pat: &[u8]) -> usize {
    let mut count = 0;
    let mut start = 0;
    while start < pat.len() {
        if pat[start] != STAR {
            start += 1;
            continue;
        }
        let mut end = start;
        while pat.get(end) == Some(&STAR) {
            end += 1;
        }
        count += usize::from(is_special_run(pat, start, end));
        start = end;
    }
    count
}

/// The star run starting at `pat[p]`, matched against `text[t..]`.
fn star(pat: &[u8], p: usize, text: &[u8], t: usize, memo: &mut Memo) -> Outcome {
    let end = p + pat[p..].iter().take_while(|b| **b == STAR).count();
    let crosses_slash = is_special_run(pat, p, end);
    // `**/` may match no directory at all, but only before a literal `/`,
    // not before an escaped one.
    if crosses_slash
        && pat.get(end) == Some(&SLASH)
        && walk_memoized(pat, end + 1, text, t, memo) == Outcome::Match
    {
        return Outcome::Match;
    }
    if end == pat.len() {
        return trailing_star(crosses_slash, &text[t..]);
    }
    align_after_star(pat, end, text, t, crosses_slash, memo)
}

/// A star run that ends the pattern: it takes the rest of the text, which
/// may hold a `/` only when the run is a whole-segment `**`.
fn trailing_star(crosses_slash: bool, rest: &[u8]) -> Outcome {
    if crosses_slash || !rest.contains(&SLASH) {
        Outcome::Match
    } else {
        Outcome::NoMatch
    }
}

/// Tries `pat[end..]` at each text position from `t` on, as the star run
/// before `pat[end]` takes ever more of the text.
fn align_after_star(
    pat: &[u8],
    end: usize,
    text: &[u8],
    t: usize,
    crosses_slash: bool,
    memo: &mut Memo,
) -> Outcome {
    let next = pat[end];
    let literal_next = !matches!(next, STAR | b'?' | b'[' | BACKSLASH);
    let mut cur = t;
    loop {
        if cur >= text.len() {
            return Outcome::AbortAll;
        }
        if literal_next {
            match skip_to_literal(text, cur, next, crosses_slash) {
                Ok(found) => cur = found,
                Err(outcome) => return outcome,
            }
        }
        let outcome = if crosses_slash {
            walk_memoized(pat, end, text, cur, memo)
        } else {
            walk(pat, end, text, cur, memo)
        };
        if let Some(decided) = after_alignment(outcome, crosses_slash, text[cur]) {
            return decided;
        }
        cur += 1;
    }
}

/// The fast path for a literal after the star run: the first position from
/// `cur` holding `next`, or the outcome when the run cannot reach one (the
/// text ran out, or a single `*` met a `/` first).
fn skip_to_literal(
    text: &[u8],
    mut cur: usize,
    next: u8,
    crosses_slash: bool,
) -> Result<usize, Outcome> {
    while cur < text.len() && (crosses_slash || text[cur] != SLASH) && text[cur] != next {
        cur += 1;
    }
    if cur >= text.len() {
        Err(Outcome::AbortAll)
    } else if text[cur] != next {
        Err(Outcome::AbortToDoubleStar)
    } else {
        Ok(cur)
    }
}

/// What one alignment's `outcome` means for the star run: `None` to let
/// the run take the byte `taken` and try again, else the run's outcome.
fn after_alignment(outcome: Outcome, crosses_slash: bool, taken: u8) -> Option<Outcome> {
    match outcome {
        Outcome::NoMatch if !crosses_slash && taken == SLASH => Some(Outcome::AbortToDoubleStar),
        Outcome::NoMatch => None,
        Outcome::AbortToDoubleStar if crosses_slash => None,
        decided => Some(decided),
    }
}

/// The bracket expression opened at `pat[open]`, tested against `byte`:
/// the index of its closing `]` and whether `byte` is selected (negation
/// applied), or `None` when the expression is malformed (unclosed, or an
/// unknown `[:name:]`), which aborts the whole match.
fn bracket(pat: &[u8], open: usize, byte: u8) -> Option<(usize, bool)> {
    let mut i = open + 1;
    let negated = matches!(pat.get(i), Some(b'!' | b'^'));
    if negated {
        i += 1;
    }
    let mut found = false;
    // The previous plain item, the low end of a following `a-b` range; zero
    // after a range or a class so a `-` right after them is literal.
    let mut previous = 0u8;
    let mut first = true;
    loop {
        let item = *pat.get(i)?;
        if item == b']' && !first {
            return Some((i, found != negated));
        }
        first = false;
        let (last, selected, low) = bracket_item(pat, i, byte, previous)?;
        found |= selected;
        previous = low;
        i = last + 1;
    }
}

/// One item of a bracket expression at `pat[i]`, tested against `byte`:
/// the index of its last pattern byte, whether it selects `byte`, and the
/// `previous` it leaves for a following range.
fn bracket_item(pat: &[u8], i: usize, byte: u8, previous: u8) -> Option<(usize, bool, u8)> {
    let item = pat[i];
    let range_high = pat.get(i + 1).is_some_and(|next| *next != b']');
    if item == BACKSLASH {
        let escaped = *pat.get(i + 1)?;
        Some((i + 1, escaped == byte, escaped))
    } else if item == b'-' && previous != 0 && range_high {
        bracket_range(pat, i + 1, previous, byte)
    } else if item == b'[' && pat.get(i + 1) == Some(&b':') {
        bracket_class(pat, i, byte)
    } else {
        Some((i, item == byte, item))
    }
}

/// The `low-high` range whose high end starts at `pat[i]` (possibly
/// escaped), tested against `byte`.
fn bracket_range(pat: &[u8], mut i: usize, low: u8, byte: u8) -> Option<(usize, bool, u8)> {
    let mut high = pat[i];
    if high == BACKSLASH {
        i += 1;
        high = *pat.get(i)?;
    }
    Some((i, low <= byte && byte <= high, 0))
}

/// A `[:name:]` class opened at `pat[i]`, tested against `byte`; without a
/// closing `:]` the `[` is a plain item.
fn bracket_class(pat: &[u8], i: usize, byte: u8) -> Option<(usize, bool, u8)> {
    let name_start = i + 2;
    let close = name_start + pat[name_start..].iter().position(|b| *b == b']')?;
    if close > name_start && pat[close - 1] == b':' {
        let selected = class_contains(&pat[name_start..close - 1], byte)?;
        Some((close, selected, 0))
    } else {
        Some((i, byte == b'[', b'['))
    }
}

/// Whether a POSIX class holds a byte.
type ClassTest = fn(&u8) -> bool;

/// The POSIX classes a bracket expression may name, ASCII only, with git's
/// own `isspace`: tab, newline, carriage return and space.
const POSIX_CLASSES: [(&[u8], ClassTest); 12] = [
    (b"alnum", u8::is_ascii_alphanumeric),
    (b"alpha", u8::is_ascii_alphabetic),
    (b"blank", is_blank),
    (b"cntrl", u8::is_ascii_control),
    (b"digit", u8::is_ascii_digit),
    (b"graph", u8::is_ascii_graphic),
    (b"lower", u8::is_ascii_lowercase),
    (b"print", is_print),
    (b"punct", u8::is_ascii_punctuation),
    (b"space", is_space),
    (b"upper", u8::is_ascii_uppercase),
    (b"xdigit", u8::is_ascii_hexdigit),
];

/// Whether the POSIX class `name` holds `byte`; `None` for an unknown
/// class name.
fn class_contains(name: &[u8], byte: u8) -> Option<bool> {
    POSIX_CLASSES
        .iter()
        .find(|(class, _)| *class == name)
        .map(|(_, holds)| holds(&byte))
}

fn is_blank(byte: &u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn is_print(byte: &u8) -> bool {
    byte.is_ascii_graphic() || *byte == b' '
}

fn is_space(byte: &u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

#[cfg(test)]
mod tests {
    use super::wildmatch;
    use std::sync::mpsc;
    use std::time::Duration;

    type Row = (&'static [u8], &'static [u8], bool);

    const PATHOLOGICAL_STAR_COUNT: usize = 64;
    const PATHOLOGICAL_TEXT_LEN: usize = 255;
    const PATHOLOGICAL_DEADLINE: Duration = Duration::from_secs(1);
    const REPEATED_SEGMENT_COUNT: usize = 400;
    const LONG_TEXT_SEGMENTS: usize = 30;
    const HUGE_SEGMENT_COUNT: usize = 1000;

    fn assert_rows(rows: &[Row]) {
        for (pattern, text, expected) in rows {
            assert_eq!(
                wildmatch(pattern, text),
                *expected,
                "pattern {:?} against {:?}",
                String::from_utf8_lossy(pattern),
                String::from_utf8_lossy(text),
            );
        }
    }

    #[test]
    fn test_a_pathological_star_pattern_returns_promptly() {
        let mut pattern = b"*a".repeat(PATHOLOGICAL_STAR_COUNT);
        pattern.push(b'b');
        let text = vec![b'a'; PATHOLOGICAL_TEXT_LEN];
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(wildmatch(&pattern, &text));
        });
        let verdict = receiver.recv_timeout(PATHOLOGICAL_DEADLINE);
        assert_eq!(verdict, Ok(false));
    }

    #[test]
    fn test_a_thousand_double_star_segments_return_promptly() {
        let mut pattern = b"**/".repeat(HUGE_SEGMENT_COUNT);
        pattern.push(b'x');
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(wildmatch(&pattern, b"a/b/c/d/e/f.log"));
        });
        let verdict = receiver.recv_timeout(PATHOLOGICAL_DEADLINE);
        assert_eq!(verdict, Ok(false));
    }

    #[test]
    fn test_repeated_double_star_segments_return_promptly() {
        let mut pattern = b"**/".repeat(REPEATED_SEGMENT_COUNT);
        pattern.push(b'x');
        let mut long_text = b"a/".repeat(LONG_TEXT_SEGMENTS);
        long_text.push(b'y');
        let rows: [(Vec<u8>, bool); 3] = [
            (b"a/b/c/d/e/f.log".to_vec(), false),
            (long_text, false),
            (b"a/b/x".to_vec(), true),
        ];
        for (text, expected) in rows {
            let pattern = pattern.clone();
            let shown = String::from_utf8_lossy(&text).into_owned();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(wildmatch(&pattern, &text));
            });
            let verdict = receiver.recv_timeout(PATHOLOGICAL_DEADLINE);
            assert_eq!(verdict, Ok(expected), "text {shown:?}");
        }
    }

    #[test]
    fn test_stars_and_question_marks_stop_at_a_slash_and_count_bytes() {
        let rows: &[Row] = &[
            (b"*", b"abc", true),
            (b"*", b"a/b", false),
            (b"a*", b"a/b", false),
            (b"p*.java", b"p/a.java", false),
            (b"p*.java", b"pxa.java", true),
            (b"*.java", b"a.java", true),
            (b"*/*", b"a/b", true),
            (b"*/*", b"a/b/c", false),
            (b"a*/b", b"a/b", true),
            (b"a*/b", b"axy/b", true),
            (b"a*/b", b"axy/z/b", false),
            (b"p?a.java", b"p/a.java", false),
            (b"p?a.java", b"pya.java", true),
            (b"X?y.java", b"X\xc3\xa9y.java", false),
            (b"X??y.java", b"X\xc3\xa9y.java", true),
            (b"a.java", b"A.java", false),
            (b"[a-z].java", b"A.java", false),
            (b"a/b", b"a/b", true),
            (b"a\\", b"a", false),
            (b"a\\", b"a\\", false),
            (b"\\*", b"*", true),
            (b"\\*", b"x", false),
            (b"a\\b", b"ab", true),
            (b"a\\b", b"a\\b", false),
            (b"a\\ ", b"a ", true),
        ];
        assert_rows(rows);
    }

    #[test]
    fn test_double_stars_cross_directories_only_as_a_whole_segment() {
        let rows: &[Row] = &[
            (b"**", b"a/b/c", true),
            (b"**/foo", b"foo", true),
            (b"**/foo", b"a/b/foo", true),
            (b"**/foo", b"a/xfoo", false),
            (b"a/**", b"a", false),
            (b"a/**", b"a/b/c", true),
            (b"a/**/b", b"a/b", true),
            (b"a/**/b", b"a/x/y/b", true),
            (b"a/**/b", b"a/xb", false),
            (b"a**b", b"a/b", false),
            (b"a**b", b"axyb", true),
            (b"**a", b"x/a", false),
            (b"a/**x", b"a/b/x", false),
            (b"**/deep/*.java", b"x/deep/Y.java", true),
            (b"**/deep/*.java", b"deep/Y.java", true),
            (b"**/deep/*.java", b"x/deep/z/Y.java", false),
            (b"a/***/Keep.java", b"a/q/r/Keep.java", true),
            (b"a/***/Keep.java", b"a/Keep.java", true),
            (b"a/***", b"a/q/r", true),
            (b"a***b", b"a/b", false),
            (b"a/**\\/Keep.java", b"a/q/r/Keep.java", true),
            (b"a/**\\/Keep.java", b"a/q/Keep.java", true),
            (b"a/**\\/Keep.java", b"a/Keep.java", false),
            (b"a/*/Keep.java", b"a/q/Keep.java", true),
            (b"a/*/Keep.java", b"a/q/r/Keep.java", false),
        ];
        assert_rows(rows);
    }

    #[test]
    fn test_bracket_expressions_follow_git() {
        let rows: &[Row] = &[
            (b"p[!x]a.java", b"p/a.java", false),
            (b"p[.-0]a.java", b"p/a.java", false),
            (b"p[ -~]a.java", b"p/a.java", false),
            (b"p[!x]a.java", b"pya.java", true),
            (b"p[!x]a.java", b"pxa.java", false),
            (b"[a-c]", b"a", true),
            (b"[a-c]", b"c", true),
            (b"[a-c]", b"d", false),
            (b"[c-a]", b"b", false),
            (b"Cls[^Q].java", b"ClsQ.java", false),
            (b"Cls[^Q].java", b"Clsa.java", true),
            (b"Cls[!Q].java", b"ClsQ.java", false),
            (b"[a-\xff]", b"z", true),
            (b"[a-\xff]", b"\xe9", true),
            (b"[a-\xff]", b"A", false),
            (b"[]a]", b"]", true),
            (b"[]a]", b"a", true),
            (b"[]a]", b"b", false),
            (b"[!]a]", b"b", true),
            (b"[!]a]", b"]", false),
            (b"[a\\]]", b"]", true),
            (b"[a-]", b"-", true),
            (b"[[]", b"[", true),
            (b"[[:]", b":", true),
            (b"Cls[[:upper:]].java", b"ClsQ.java", true),
            (b"Cls[[:upper:]].java", b"Clsu.java", false),
            (b"Cls[![:upper:]].java", b"Clsu.java", true),
            (b"[[:lower:]]", b"q", true),
            (b"[[:lower:]]", b"Q", false),
            (b"[[:alpha:]]", b"\xe9", false),
            (b"[[:digit:]x]", b"x", true),
            (b"[[:digit:]x]", b"7", true),
            (b"[[:space:]]", b"\t", true),
            (b"[[:space:]]", b"\x0b", false),
            (b"[[:blank:]]", b" ", true),
            (b"[[:blank:]]", b"\r", false),
            (b"[[:punct:]]", b"!", true),
            (b"[[:punct:]]", b"a", false),
            (b"[[:xdigit:]]", b"f", true),
            (b"[[:xdigit:]]", b"g", false),
            (b"[[:alnum:]]", b"_", false),
            (b"[[:cntrl:]]", b"\x7f", true),
            (b"[[:graph:]]", b" ", false),
            (b"[[:print:]]", b" ", true),
            (b"[[:bogus:]]", b"b", false),
            (b"[[:bogus:]]", b":", false),
            (b"[[:upper:]", b"Q", false),
            (b"a[", b"a[", false),
            (b"a[b", b"a[b", false),
            (b"Cls[a.java", b"Cls[a.java", false),
            (b"a]", b"a]", true),
        ];
        assert_rows(rows);
    }
}
