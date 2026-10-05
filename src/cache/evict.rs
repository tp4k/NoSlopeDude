//! M5-2: best-effort eviction of the cache store.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::{Cache, CacheError};

use git2::Repository;

/// The size and age limits of one eviction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionLimits {
    pub max_bytes: u64,
    pub max_age: Duration,
}

impl EvictionLimits {
    pub const DEFAULT: EvictionLimits = EvictionLimits {
        max_bytes: 0,
        max_age: Duration::ZERO,
    };
}

impl Cache {
    /// Opens the cache like `Cache::open`, running an eviction pass under
    /// `limits` when none ran in the last day before `now`.
    pub fn open_at(
        _repo: &Repository,
        _limits: EvictionLimits,
        _now: SystemTime,
    ) -> Result<Cache, CacheError> {
        unimplemented!()
    }

    /// One eviction pass; returns how many files it removed.
    pub fn evict(&self, _limits: EvictionLimits, _now: SystemTime) -> usize {
        unimplemented!()
    }

    pub(super) fn run_pass(
        &self,
        _limits: EvictionLimits,
        _now: SystemTime,
        _remove: &dyn Fn(&Path) -> io::Result<()>,
    ) -> usize {
        unimplemented!()
    }
}
