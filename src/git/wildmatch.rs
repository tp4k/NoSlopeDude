//! Git's `wildmatch` with `WM_PATHNAME`, byte-wise and case-sensitive.

/// Whether `pattern` matches the whole of `text` under git's pathname
/// wildmatch rules. Neither side is decoded: both are raw bytes.
pub(super) fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    let _ = (pattern, text);
    todo!("git wildmatch")
}
