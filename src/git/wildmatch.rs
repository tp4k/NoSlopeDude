//! Git's `wildmatch` with `WM_PATHNAME`, byte-wise and case-sensitive.

/// Whether `pattern` matches the whole of `text` under git's pathname
/// wildmatch rules. Neither side is decoded: both are raw bytes.
pub(super) fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    let _ = (pattern, text);
    todo!("git wildmatch")
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
