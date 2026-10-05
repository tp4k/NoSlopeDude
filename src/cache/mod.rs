//! M5-1: the per-blob analysis cache store.

use std::path::{Path, PathBuf};

use git2::{Oid, Repository};

use crate::model::Grammar;

mod payload;

#[cfg(test)]
mod tests;

pub use payload::{
    AnalyzedPayload, CachedAnalysis, CachedCallable, CachedCandidate, CachedDamage, CachedFinding,
    CachedIdentity, CachedReason, Hydrated,
};

/// Why a cache read-side setup or a write failed.
#[derive(Debug)]
pub enum CacheError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
}

/// The identity of one entry: blob OID, grammar and measurement fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    blob: String,
    grammar: Grammar,
    fingerprint: String,
}

impl CacheKey {
    pub fn new(blob: Oid, grammar: Grammar, min_clone_lines: u32) -> CacheKey {
        let _ = (blob, grammar, min_clone_lines);
        todo!()
    }

    pub fn from_parts(blob: Oid, grammar: Grammar, fingerprint: &str) -> CacheKey {
        let _ = (blob, grammar, fingerprint);
        todo!()
    }

    pub fn id_hex(&self) -> String {
        todo!()
    }
}

/// A handle on `$GIT_COMMON_DIR/nsd/cache/v1`.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn open(repo: &Repository) -> Result<Cache, CacheError> {
        let _ = repo;
        todo!()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entry_path(&self, key: &CacheKey) -> PathBuf {
        let _ = key;
        todo!()
    }

    pub fn get(&self, key: &CacheKey) -> Option<CachedAnalysis> {
        let _ = key;
        todo!()
    }

    pub fn put(&self, key: &CacheKey, analysis: &CachedAnalysis) -> Result<(), CacheError> {
        let _ = (key, analysis);
        todo!()
    }
}
