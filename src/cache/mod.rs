//! M5-1: the per-blob analysis cache store under
//! `$GIT_COMMON_DIR/nsd/cache/v1`. An entry is a header line (version, key
//! parts, payload digest), a newline, then the payload JSON. Any read that
//! does not check out is a miss; a write goes to a temp file in the same
//! directory and is renamed into place.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use git2::{Oid, Repository};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::hashing::Digest;
use crate::model::Grammar;
use crate::profile::{fingerprint, MeasurementProfileInputs};

mod evict;
mod payload;

#[cfg(test)]
mod tests;

pub use evict::EvictionLimits;
pub use payload::{
    AnalyzedPayload, CachedAnalysis, CachedCallable, CachedCallableKind, CachedCandidate,
    CachedDamage, CachedDamageKind, CachedFinding, CachedIdentity, CachedReason, Hydrated,
    HydratedDamage,
};

/// The entry-format version; an entry carrying another one is a miss.
const CACHE_VERSION: u32 = 1;
const ROOT_COMPONENTS: [&str; 3] = ["nsd", "cache", "v1"];
const KEY_FAMILY_PREFIX: &str = "analysis-cache-key";
const PAYLOAD_FAMILY_PREFIX: &str = "analysis-cache-payload";
const FAN_OUT_HEX_DIGITS: usize = 2;
const ENTRY_EXTENSION: &str = "json";
const HEADER_TERMINATOR: u8 = b'\n';

/// A cache I/O failure; the caller turns it into one warning and recomputes.
#[derive(Debug)]
pub enum CacheError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Io { path, source } => {
                write!(f, "cache I/O failure at {}: {source}", path.display())
            }
            CacheError::Serialize(source) => write!(f, "cache entry not serializable: {source}"),
        }
    }
}

impl std::error::Error for CacheError {}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> CacheError + '_ {
    move |source| CacheError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn grammar_label(grammar: Grammar) -> &'static str {
    match grammar {
        Grammar::Java => "java",
        Grammar::JavaScript => "javascript",
        Grammar::TypeScript => "typescript",
        Grammar::Tsx => "tsx",
    }
}

fn digest_hex(family: &str, parts: &[&[u8]]) -> String {
    let mut digest = Digest::new(family);
    for part in parts {
        digest.push(part);
    }
    format!("{:032x}", digest.finish())
}

#[derive(Serialize, Deserialize)]
struct Header {
    cache_version: u32,
    blob: String,
    grammar: String,
    measurement_fingerprint: String,
    payload_digest: String,
}

/// The identity of one entry: blob OID, grammar and measurement fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    blob: String,
    grammar: Grammar,
    fingerprint: String,
}

impl CacheKey {
    /// A key under the current measurement profile with the effective
    /// `min_clone_lines`.
    pub fn new(blob: Oid, grammar: Grammar, min_clone_lines: u32) -> CacheKey {
        let inputs = MeasurementProfileInputs {
            min_clone_lines,
            ..MeasurementProfileInputs::current()
        };
        CacheKey::from_parts(blob, grammar, &fingerprint(&inputs))
    }

    pub fn from_parts(blob: Oid, grammar: Grammar, fingerprint: &str) -> CacheKey {
        CacheKey {
            blob: blob.to_string(),
            grammar,
            fingerprint: fingerprint.to_string(),
        }
    }

    /// The key's 32 lowercase hex digits, which name its entry file.
    pub fn id_hex(&self) -> String {
        digest_hex(
            KEY_FAMILY_PREFIX,
            &[
                self.blob.as_bytes(),
                grammar_label(self.grammar).as_bytes(),
                self.fingerprint.as_bytes(),
            ],
        )
    }

    fn header(&self, payload_digest: String) -> Header {
        Header {
            cache_version: CACHE_VERSION,
            blob: self.blob.clone(),
            grammar: grammar_label(self.grammar).to_string(),
            measurement_fingerprint: self.fingerprint.clone(),
            payload_digest,
        }
    }
}

/// A handle on `$GIT_COMMON_DIR/nsd/cache/v1`.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Opens the cache of `repo`'s common Git directory, creating the root.
    pub fn open(repo: &Repository) -> Result<Cache, CacheError> {
        Cache::open_at(repo, EvictionLimits::DEFAULT, SystemTime::now())
    }

    fn open_root(repo: &Repository) -> Result<Cache, CacheError> {
        let root = ROOT_COMPONENTS
            .iter()
            .fold(repo.commondir().to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(&root).map_err(io_error(&root))?;
        Ok(Cache { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn fan_out_dir(&self, id_hex: &str) -> PathBuf {
        self.root.join(&id_hex[..FAN_OUT_HEX_DIGITS])
    }

    /// Where `key`'s entry lives: `<root>/<first two hex>/<32 hex>.json`.
    pub fn entry_path(&self, key: &CacheKey) -> PathBuf {
        let id = key.id_hex();
        self.fan_out_dir(&id)
            .join(format!("{id}.{ENTRY_EXTENSION}"))
    }

    /// The payload stored under `key`, or `None` for any miss: no file, an
    /// unreadable or unparsable one, another version, a header that is not
    /// this key's, or a payload that does not match its digest.
    pub fn get(&self, key: &CacheKey) -> Option<CachedAnalysis> {
        let path = self.entry_path(key);
        let bytes = fs::read(&path).ok()?;
        let split = bytes.iter().position(|byte| *byte == HEADER_TERMINATOR)?;
        let payload_bytes = &bytes[split + 1..];
        let header: Header = serde_json::from_slice(&bytes[..split]).ok()?;
        let expected = key.header(digest_hex(PAYLOAD_FAMILY_PREFIX, &[payload_bytes]));
        let matches = header.cache_version == expected.cache_version
            && header.blob == expected.blob
            && header.grammar == expected.grammar
            && header.measurement_fingerprint == expected.measurement_fingerprint
            && header.payload_digest == expected.payload_digest;
        if !matches {
            return None;
        }
        let analysis = serde_json::from_slice(payload_bytes).ok()?;
        self.refresh_recency(&path);
        Some(analysis)
    }

    /// Writes `analysis` under `key` through a temp file renamed into place,
    /// so a concurrent reader sees the old entry or the new one, never a
    /// partial one.
    pub fn put(&self, key: &CacheKey, analysis: &CachedAnalysis) -> Result<(), CacheError> {
        let payload_bytes = serde_json::to_vec(analysis).map_err(CacheError::Serialize)?;
        let header = key.header(digest_hex(PAYLOAD_FAMILY_PREFIX, &[&payload_bytes]));
        let mut contents = serde_json::to_vec(&header).map_err(CacheError::Serialize)?;
        contents.push(HEADER_TERMINATOR);
        contents.extend_from_slice(&payload_bytes);

        let target = self.entry_path(key);
        let dir = self.fan_out_dir(&key.id_hex());
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        let mut staged = NamedTempFile::new_in(&dir).map_err(io_error(&dir))?;
        staged.write_all(&contents).map_err(io_error(&dir))?;
        staged.persist(&target).map_err(|failure| CacheError::Io {
            path: target.clone(),
            source: failure.error,
        })?;
        Ok(())
    }
}
