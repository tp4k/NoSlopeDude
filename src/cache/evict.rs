//! M5-2: best-effort eviction of the cache store. Recency is the entry
//! file's mtime. The pass walks real fan-out directories only and removes
//! nothing but entry files and stale temp files, so a planted symlink or a
//! stray file is never followed or deleted.

use std::fs::{self, File, FileTimes};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use git2::Repository;
use tempfile::NamedTempFile;

use super::{Cache, CacheError, ENTRY_EXTENSION, FAN_OUT_HEX_DIGITS};

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const DEFAULT_MAX_BYTES: u64 = 1 << 30;
const DEFAULT_MAX_AGE_DAYS: u64 = 30;
/// A hit refreshes an entry's mtime only when it is older than this.
const TOUCH_AFTER: Duration = Duration::from_secs(SECONDS_PER_DAY);
/// The pass runs at most once per this interval, per the stamp file.
const PASS_INTERVAL: Duration = Duration::from_secs(SECONDS_PER_DAY);
const STAMP_FILE: &str = "eviction-stamp";
const ENTRY_ID_HEX_DIGITS: usize = 32;
const TEMP_PREFIX: &str = ".tmp";

/// The size and age limits of one eviction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionLimits {
    pub max_bytes: u64,
    pub max_age: Duration,
}

impl EvictionLimits {
    pub const DEFAULT: EvictionLimits = EvictionLimits {
        max_bytes: DEFAULT_MAX_BYTES,
        max_age: Duration::from_secs(DEFAULT_MAX_AGE_DAYS * SECONDS_PER_DAY),
    };
}

struct Stored {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_fan_out_name(name: &str) -> bool {
    name.len() == FAN_OUT_HEX_DIGITS && is_lower_hex(name)
}

fn is_entry_name(name: &str) -> bool {
    name.strip_suffix(ENTRY_EXTENSION)
        .and_then(|stem| stem.strip_suffix('.'))
        .is_some_and(|id| id.len() == ENTRY_ID_HEX_DIGITS && is_lower_hex(id))
}

fn age_of(modified: SystemTime, now: SystemTime) -> Duration {
    now.duration_since(modified).unwrap_or(Duration::ZERO)
}

impl Cache {
    /// Opens the cache like `Cache::open`, running an eviction pass under
    /// `limits` unless one ran within the last day before `now`.
    pub fn open_at(
        repo: &Repository,
        limits: EvictionLimits,
        now: SystemTime,
    ) -> Result<Cache, CacheError> {
        let cache = Cache::open_root(repo)?;
        if cache.pass_is_due(now) {
            if cache.record_pass(now) {
                cache.evict(limits, now);
            }
        }
        Ok(cache)
    }

    /// One eviction pass: removes entries older than `limits.max_age` and
    /// stale temp files, then the least recently used entries until the
    /// total is at most `limits.max_bytes`. Returns how many files it found
    /// gone after removal; a file it cannot remove is skipped.
    pub fn evict(&self, limits: EvictionLimits, now: SystemTime) -> usize {
        self.run_pass(limits, now, &|path| fs::remove_file(path))
    }

    pub(super) fn run_pass(
        &self,
        limits: EvictionLimits,
        now: SystemTime,
        remove: &dyn Fn(&Path) -> io::Result<()>,
    ) -> usize {
        let gone = |result: io::Result<()>| match result {
            Err(error) => error.kind() == io::ErrorKind::NotFound,
            Ok(()) => true,
        };
        let (mut entries, temps) = self.walk();
        let mut removed = 0;
        for temp in temps {
            if age_of(temp.modified, now) > limits.max_age && gone(remove(&temp.path)) {
                removed += 1;
            }
        }
        entries.sort_by(|a, b| (a.modified, &a.path).cmp(&(b.modified, &b.path)));
        let mut total: u64 = entries
            .iter()
            .fold(0, |sum, entry| u64::saturating_add(sum, entry.len));
        for entry in entries {
            let expired = age_of(entry.modified, now) > limits.max_age;
            if (expired || total > limits.max_bytes) && gone(remove(&entry.path)) {
                removed += 1;
                total = total.saturating_sub(entry.len);
            }
        }
        removed
    }

    /// Entry files and temp files of the real fan-out directories.
    fn walk(&self) -> (Vec<Stored>, Vec<Stored>) {
        let mut entries = Vec::new();
        let mut temps = Vec::new();
        let Ok(fan_outs) = fs::read_dir(&self.root) else {
            return (entries, temps);
        };
        for fan_out in fan_outs.flatten() {
            let is_real_dir = fan_out.file_name().to_str().is_some_and(is_fan_out_name)
                && fs::symlink_metadata(fan_out.path()).is_ok_and(|meta| meta.is_dir());
            if !is_real_dir {
                continue;
            }
            let Ok(files) = fs::read_dir(fan_out.path()) else {
                continue;
            };
            for file in files.flatten() {
                let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Ok(meta) = fs::symlink_metadata(file.path()) else {
                    continue;
                };
                let Ok(modified) = meta.modified() else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let stored = Stored {
                    path: file.path(),
                    len: meta.len(),
                    modified,
                };
                if is_entry_name(&name) {
                    entries.push(stored);
                } else if name.starts_with(TEMP_PREFIX) {
                    temps.push(stored);
                }
            }
        }
        (entries, temps)
    }

    fn stamp_path(&self) -> PathBuf {
        self.root.join(STAMP_FILE)
    }

    fn pass_is_due(&self, now: SystemTime) -> bool {
        let path = self.stamp_path();
        let recorded = fs::symlink_metadata(&path)
            .ok()
            .filter(|meta| meta.is_file())
            .and_then(|_| fs::read_to_string(&path).ok())
            .and_then(|text| text.trim().parse::<u64>().ok());
        match recorded {
            Some(seconds) => UNIX_EPOCH
                .checked_add(Duration::from_secs(seconds))
                .and_then(|stamped| now.duration_since(stamped).ok())
                .map_or(true, |elapsed| elapsed >= PASS_INTERVAL),
            None => true,
        }
    }

    /// Writes the stamp through a rename, so a symlink planted at its path
    /// is replaced, not followed. Returns whether the stamp was persisted.
    fn record_pass(&self, now: SystemTime) -> bool {
        let seconds = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let written = NamedTempFile::new_in(&self.root).and_then(|mut staged| {
            write!(staged, "{seconds}")?;
            staged
                .persist(self.stamp_path())
                .map_err(|failure| failure.error)
        });
        written.is_ok()
    }

    /// Best effort: moves a hit entry's mtime to now once it is a day old.
    pub(super) fn refresh_recency(&self, path: &Path) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        let stale = meta
            .modified()
            .is_ok_and(|modified| age_of(modified, SystemTime::now()) > TOUCH_AFTER);
        if !meta.is_file() || !stale {
            return;
        }
        let touched = File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_times(FileTimes::new().set_modified(SystemTime::now())));
        drop(touched);
    }
}
