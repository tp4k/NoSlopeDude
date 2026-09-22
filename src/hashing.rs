//! Crate-internal (WS-3): one versioned BLAKE3 digest, replacing
//! `src/clones/mod.rs`'s two `DefaultHasher` (SipHash) streams. `HASH_VERSION`
//! is hashed into every digest's input as its first byte (not merely stored
//! beside the result), so bumping it changes every digest this module
//! produces — the property M5's persisted cache key needs: an algorithm
//! change must invalidate old entries, never silently collide with them.

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
        assert_eq!(digest.finish(), 0);
    }
}
