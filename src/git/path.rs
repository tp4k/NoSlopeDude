//! `RepoPath`: a repository-relative path stored as raw bytes, exactly as
//! Git stores tree and index entries. Never a `PathBuf` (do-not-reuse,
//! `src/model.rs::SkippedFile.relative_path`): that type loses non-UTF-8
//! bytes on non-Unix platforms and silently accepts absolute paths.

use std::cmp::Ordering;
use std::fmt;

/// A repository-relative byte path (D3, D5). Comparison and ordering are
/// byte-wise (D10: deterministic ordering by raw path bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    /// Builds a `RepoPath` from raw bytes, exactly as read from a Git tree,
    /// index or worktree walk. No validation is performed: a repository-
    /// relative path may legitimately be invalid UTF-8.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> RepoPath {
        RepoPath(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// D3: deterministic, injective percent-escaped rendering. A path whose
    /// bytes are valid UTF-8 renders verbatim, `%` included. Otherwise every
    /// literal `%` and every byte of an invalid UTF-8 sequence renders as
    /// uppercase `%XX`, with valid runs kept verbatim.
    pub fn render(&self) -> String {
        if let Ok(valid) = std::str::from_utf8(&self.0) {
            return valid.to_string();
        }
        let mut out = String::with_capacity(self.0.len());
        for chunk in self.0.utf8_chunks() {
            push_escaping_percent(chunk.valid(), &mut out);
            for &byte in chunk.invalid() {
                push_percent_byte(byte, &mut out);
            }
        }
        out
    }
}

impl PartialOrd for RepoPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RepoPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render())
    }
}

fn push_escaping_percent(valid: &str, out: &mut String) {
    for ch in valid.chars() {
        if ch == '%' {
            out.push_str("%25");
        } else {
            out.push(ch);
        }
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

fn push_percent_byte(byte: u8, out: &mut String) {
    out.push('%');
    out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
    out.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_with_percent_renders_verbatim() {
        let path = RepoPath::from_bytes(b"src/100%.ts".to_vec());
        assert_eq!(path.render(), "src/100%.ts");
    }

    #[test]
    fn invalid_byte_and_percent_both_escape() {
        let mut bytes = b"src/a".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b"%.ts");
        let path = RepoPath::from_bytes(bytes);
        assert_eq!(path.render(), "src/a%FF%25.ts");
    }

    #[test]
    fn ordering_is_by_raw_bytes() {
        let a = RepoPath::from_bytes(b"a.ts".to_vec());
        let b = RepoPath::from_bytes(b"b.ts".to_vec());
        assert!(a < b);
    }
}
