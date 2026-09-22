//! Crate-internal (WS-3): one versioned BLAKE3 digest, replacing
//! `src/clones/mod.rs`'s two `DefaultHasher` (SipHash) streams. `HASH_VERSION`
//! is hashed into every digest's input as its first byte (not merely stored
//! beside the result), so bumping it changes every digest this module
//! produces — the property M5's persisted cache key needs: an algorithm
//! change must invalidate old entries, never silently collide with them.

use blake3::Hasher;

/// The count of leading BLAKE3 output bytes `Digest::finish` folds into the
/// `u128` it returns.
const LEADING_BYTES: usize = 16;

/// Fed as the first byte of every digest's input (not merely stored beside
/// the resulting hash), current as of this module's introduction. Bump this
/// if the framing or input order below ever changes, so a persisted M5 cache
/// key computed under the old scheme is never mistaken for one computed
/// under the new one.
const HASH_VERSION: u8 = 1;

/// A versioned BLAKE3 digest builder. Frame-safe: `push` prefixes each slice
/// with its own length, so `push(b"ab"); push(b"c")` and `push(b"a");
/// push(b"bc")` can never land on the same digest. Non-consuming: `finish`
/// reads the current state without invalidating further `push` calls, the
/// same contract `src/clones/mod.rs`'s incremental candidate enumeration
/// needs (it calls `finish` once per extended window of the same run).
pub(crate) struct Digest {
    hasher: Hasher,
}

impl Digest {
    /// Starts a new digest for one `family_prefix` domain (D14's Java vs
    /// JS/TS separation), under the current `HASH_VERSION`.
    pub(crate) fn new(family_prefix: &str) -> Self {
        Self::with_version(HASH_VERSION, family_prefix)
    }

    /// Same as `new`, but with an explicit version tag — exists so the unit
    /// tests below can prove the version is fed into the hash input, not
    /// merely stored beside it.
    fn with_version(version: u8, family_prefix: &str) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(&[version]);
        let mut digest = Digest { hasher };
        digest.push(family_prefix.as_bytes());
        digest
    }

    /// Feeds one more byte slice into the digest, framed by its own
    /// 8-byte little-endian length so no ambiguity can arise between how
    /// two adjacent slices' boundaries are placed.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let length = bytes.len() as u64;
        self.hasher.update(&length.to_le_bytes());
        self.hasher.update(bytes);
    }

    /// The leading 16 bytes of the BLAKE3 output, interpreted as a
    /// little-endian `u128`.
    pub(crate) fn finish(&self) -> u128 {
        let output = self.hasher.finalize();
        let output_bytes = output.as_bytes();
        let mut leading = [0u8; LEADING_BYTES];
        leading.copy_from_slice(&output_bytes[..LEADING_BYTES]);
        u128::from_le_bytes(leading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_tag_changes_the_digest() {
        let mut v1 = Digest::with_version(1, "family");
        v1.push(b"same input");
        let mut v2 = Digest::with_version(2, "family");
        v2.push(b"same input");
        assert_ne!(v1.finish(), v2.finish());
    }

    #[test]
    fn test_digest_is_stable_for_known_input() {
        let mut digest = Digest::new("family");
        digest.push(b"pinned input");
        // Pinned against `blake3 1.8.7`: a future dependency bump that
        // changes this value must fail loudly here rather than silently
        // invalidating every persisted M5 cache key.
        assert_eq!(digest.finish(), 0x35008639293bab655a8da970ad70a70a);
    }

    /// Scope item 4: `push` must frame its own length so `["ab", "c"]` and
    /// `["a", "bc"]` cannot collide, unlike a raw `update(bytes)` would.
    #[test]
    fn test_framing_prevents_split_boundary_collision() {
        let mut split_early = Digest::new("family");
        split_early.push(b"ab");
        split_early.push(b"c");
        let mut split_late = Digest::new("family");
        split_late.push(b"a");
        split_late.push(b"bc");
        assert_ne!(split_early.finish(), split_late.finish());
    }
}
