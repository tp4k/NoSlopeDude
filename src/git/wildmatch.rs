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
/// text position. Created on first use, so a pattern without such a run
/// allocates nothing.
type Memo = Option<HashMap<(usize, usize), Outcome>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Match,
    NoMatch,
    AbortAll,
    AbortToDoubleStar,
}

/// Whether `pattern` matches the whole of `text` under git's pathname
/// wildmatch rules. Neither side is decoded: both are raw bytes.
pub(super) fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    walk(pattern, 0, text, 0, &mut None) == Outcome::Match
}

/// `walk(pat, p, text, t)`, computed once per `(p, t)` for the caller.
fn walk_memoized(pat: &[u8], p: usize, text: &[u8], t: usize, memo: &mut Memo) -> Outcome {
    if let Some(known) = memo.as_ref().and_then(|table| table.get(&(p, t))) {
        return *known;
    }
    let outcome = walk(pat, p, text, t, memo);
    memo.get_or_insert_with(HashMap::new)
        .insert((p, t), outcome);
    outcome
}

/// Matches `pat[p..]` against `text[t..]`; `p == 0` is the pattern start.
fn walk(pat: &[u8], mut p: usize, text: &[u8], mut t: usize, memo: &mut Memo) -> Outcome {
    while p < pat.len() {
        let pattern_byte = pat[p];
        if t >= text.len() && pattern_byte != STAR {
            return Outcome::AbortAll;
        }
        match pattern_byte {
            STAR => return star(pat, p, text, t, memo),
            b'?' => {
                if text[t] == SLASH {
                    return Outcome::NoMatch;
                }
            }
            b'[' => match bracket(pat, p, text[t]) {
                None => return Outcome::AbortAll,
                Some((close, matched)) => {
                    if !matched || text[t] == SLASH {
                        return Outcome::NoMatch;
                    }
                    p = close;
                }
            },
            BACKSLASH => {
                p += 1;
                if pat.get(p) != Some(&text[t]) {
                    return Outcome::NoMatch;
                }
            }
            literal => {
                if literal != text[t] {
                    return Outcome::NoMatch;
                }
            }
        }
        p += 1;
        t += 1;
    }
    if t < text.len() {
        Outcome::NoMatch
    } else {
        Outcome::Match
    }
}

/// The star run starting at `pat[p]`, matched against `text[t..]`.
fn star(pat: &[u8], p: usize, text: &[u8], t: usize, memo: &mut Memo) -> Outcome {
    let mut end = p + 1;
    let mut crosses_slash = false;
    if pat.get(end) == Some(&STAR) {
        while pat.get(end) == Some(&STAR) {
            end += 1;
        }
        let at_segment_start = p == 0 || pat[p - 1] == SLASH;
        let at_segment_end = match pat.get(end) {
            None | Some(&SLASH) => true,
            Some(&BACKSLASH) => pat.get(end + 1) == Some(&SLASH),
            Some(_) => false,
        };
        if at_segment_start && at_segment_end {
            // `**/` may match no directory at all, but only before a
            // literal `/`, not before an escaped one.
            if pat.get(end) == Some(&SLASH)
                && walk_memoized(pat, end + 1, text, t, memo) == Outcome::Match
            {
                return Outcome::Match;
            }
            crosses_slash = true;
        }
    }
    let rest = &text[t..];
    let Some(&next) = pat.get(end) else {
        return if crosses_slash || !rest.contains(&SLASH) {
            Outcome::Match
        } else {
            Outcome::NoMatch
        };
    };
    let literal_next = !matches!(next, STAR | b'?' | b'[' | BACKSLASH);
    let mut cur = t;
    loop {
        if cur >= text.len() {
            return Outcome::AbortAll;
        }
        if literal_next {
            while cur < text.len() && (crosses_slash || text[cur] != SLASH) && text[cur] != next {
                cur += 1;
            }
            if cur >= text.len() {
                return Outcome::AbortAll;
            }
            if text[cur] != next {
                return Outcome::AbortToDoubleStar;
            }
        }
        let outcome = if crosses_slash {
            walk_memoized(pat, end, text, cur, memo)
        } else {
            walk(pat, end, text, cur, memo)
        };
        match outcome {
            Outcome::NoMatch => {
                if !crosses_slash && text[cur] == SLASH {
                    return Outcome::AbortToDoubleStar;
                }
            }
            Outcome::AbortToDoubleStar if crosses_slash => {}
            decided => return decided,
        }
        cur += 1;
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
        let range_high = pat.get(i + 1).copied().filter(|next| *next != b']');
        if item == BACKSLASH {
            i += 1;
            let escaped = *pat.get(i)?;
            found |= escaped == byte;
            previous = escaped;
        } else if item == b'-' && previous != 0 && range_high.is_some() {
            i += 1;
            let mut high = pat[i];
            if high == BACKSLASH {
                i += 1;
                high = *pat.get(i)?;
            }
            found |= previous <= byte && byte <= high;
            previous = 0;
        } else if item == b'[' && pat.get(i + 1) == Some(&b':') {
            let name_start = i + 2;
            let close = name_start + pat[name_start..].iter().position(|b| *b == b']')?;
            if close > name_start && pat[close - 1] == b':' {
                found |= class_contains(&pat[name_start..close - 1], byte)?;
                previous = 0;
                i = close;
            } else {
                found |= byte == b'[';
                previous = b'[';
            }
        } else {
            found |= item == byte;
            previous = item;
        }
        i += 1;
    }
}

/// Whether the POSIX class `name` holds `byte` (ASCII only, git's own
/// `isspace`: tab, newline, carriage return and space); `None` for an
/// unknown class name.
fn class_contains(name: &[u8], byte: u8) -> Option<bool> {
    Some(match name {
        b"alnum" => byte.is_ascii_alphanumeric(),
        b"alpha" => byte.is_ascii_alphabetic(),
        b"blank" => byte == b' ' || byte == b'\t',
        b"cntrl" => byte.is_ascii_control(),
        b"digit" => byte.is_ascii_digit(),
        b"graph" => byte.is_ascii_graphic(),
        b"lower" => byte.is_ascii_lowercase(),
        b"print" => byte.is_ascii_graphic() || byte == b' ',
        b"punct" => byte.is_ascii_punctuation(),
        b"space" => matches!(byte, b' ' | b'\t' | b'\n' | b'\r'),
        b"upper" => byte.is_ascii_uppercase(),
        b"xdigit" => byte.is_ascii_hexdigit(),
        _ => return None,
    })
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
