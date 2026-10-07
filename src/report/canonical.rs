//! The canonical-only parts of the scan `report.json` (M6-1/M6-2): snapshot
//! ID, fingerprints, per-reason skip counts and the complete callable entity
//! set. `mod.rs` serializes them beside the sections it already built; the
//! shared canonical writer (`format::canonical_document_pretty`) sorts keys.

use std::collections::BTreeMap;
use std::path::Path;

use git2::{ErrorCode, Repository};
use serde::Serialize;

use crate::git::snapshot::WorktreeSnapshot;
use crate::git::snapshot_id::SnapshotId;
use crate::hashing::Digest;
use crate::model::{Callable, Revision, ScanSettings};
use crate::profile;

use super::ReportSkippedFile;

/// `snapshots.unavailable_reason` for a target below its checkout's root: a
/// worktree ID covers the whole checkout, not the scanned subtree (D21).
const TARGET_NOT_GIT_ROOT: &str = "target_not_git_root";
/// `snapshots.unavailable_reason` when the target is a work-tree root whose
/// worktree snapshot could not be read.
const WORKTREE_SNAPSHOT_FAILED: &str = "worktree_snapshot_failed";

/// The `Digest` domain of `scan_configuration_fingerprint`; bump the suffix
/// when the settings it covers change.
const SCAN_CONFIGURATION_FAMILY_PREFIX: &str = "nsd-scan-config-v1";

/// Every `skipped_files` reason a scan can name, sorted: the discovery
/// labels (`model::SkipReason`) and the `parse_`-prefixed parse failures
/// (`model::ParseFailureReason`). Each is present in `skipped`, zeros too.
pub const SCAN_SKIP_KEYS: [&str; 10] = [
    "dependency_or_build_output",
    "generated_code",
    "gitignore",
    "parse_grammar_setup",
    "parse_syntax_error",
    "parse_unreadable",
    "parse_unsupported_extension",
    "test",
    "unreadable",
    "user_exclude",
];

/// The scanned tree's content-addressed ID, or why there is none.
#[derive(Debug, Clone, Serialize)]
pub struct ReportSnapshots {
    pub scan: Option<String>,
    pub unavailable_reason: Option<String>,
}

/// The configuration and measurement fingerprints of the run.
#[derive(Debug, Clone, Serialize)]
pub struct ReportFingerprints {
    pub configuration: String,
    pub measurement: String,
}

/// One measured callable: where it is and what was measured, no excerpt.
#[derive(Debug, Clone, Serialize)]
pub struct ReportEntity {
    pub path: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
    pub mass: f64,
}

/// `root`'s snapshot ID when it is a git work-tree root (D21). A local
/// target outside git carries `revision`'s own reason.
pub fn scan_snapshots(root: &Path, revision: &Revision) -> ReportSnapshots {
    let unavailable = |reason: &str| ReportSnapshots {
        scan: None,
        unavailable_reason: Some(reason.to_string()),
    };
    if let Some(reason) = &revision.unavailable_reason {
        return unavailable(reason);
    }
    let repository = match Repository::open(root) {
        Ok(repository) => repository,
        Err(error) if error.code() == ErrorCode::NotFound => {
            return unavailable(TARGET_NOT_GIT_ROOT)
        }
        Err(_) => return unavailable(WORKTREE_SNAPSHOT_FAILED),
    };
    let is_root = repository
        .workdir()
        .and_then(|workdir| Some((workdir.canonicalize().ok()?, root.canonicalize().ok()?)))
        .is_some_and(|(workdir, root)| workdir == root);
    if !is_root {
        return unavailable(TARGET_NOT_GIT_ROOT);
    }
    match WorktreeSnapshot::open(&repository)
        .and_then(|snapshot| SnapshotId::of_worktree(&repository, &snapshot))
    {
        Ok(id) => ReportSnapshots {
            scan: Some(id.to_string()),
            unavailable_reason: None,
        },
        Err(_) => unavailable(WORKTREE_SNAPSHOT_FAILED),
    }
}

/// The scan's fingerprints: its own configuration digest and the measurement
/// profile of the run's `--min-clone-lines` (D7).
pub fn scan_fingerprints(settings: &ScanSettings) -> ReportFingerprints {
    ReportFingerprints {
        configuration: scan_configuration_fingerprint(settings),
        measurement: profile::measurement_fingerprint(settings.min_clone_lines),
    }
}

/// A versioned digest of the settings that decide which files are measured
/// and how: never a path, never raw bytes (D7). The output directory is not
/// covered, since it shapes no measurement.
fn scan_configuration_fingerprint(settings: &ScanSettings) -> String {
    let mut digest = Digest::new(SCAN_CONFIGURATION_FAMILY_PREFIX);
    digest.push(&[u8::from(settings.include_tests)]);
    digest.push(&(settings.exclude.len() as u64).to_le_bytes());
    for pattern in &settings.exclude {
        digest.push(pattern.as_bytes());
    }
    digest.push(&settings.min_clone_lines.to_le_bytes());
    format!("blake3:{:032x}", digest.finish())
}

/// `skipped_files` tallied by reason, with every key of `SCAN_SKIP_KEYS`
/// present.
pub fn skip_counts(skipped_files: &[ReportSkippedFile]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = SCAN_SKIP_KEYS
        .iter()
        .map(|key| (key.to_string(), 0))
        .collect();
    for file in skipped_files {
        *counts.entry(file.reason.clone()).or_insert(0) += 1;
    }
    counts
}

/// Every measured callable, uncapped, in a total order: path, start line,
/// end line, name, then the measurements (so no tie is left to the order the
/// callables were produced in).
pub fn entities(callables: &[Callable]) -> Vec<ReportEntity> {
    let mut entities: Vec<ReportEntity> = callables
        .iter()
        .map(|callable| ReportEntity {
            path: callable.relative_path.to_string_lossy().into_owned(),
            name: callable.name.clone(),
            start_line: callable.start_line,
            end_line: callable.end_line,
            cc: callable.cc,
            sloc: callable.sloc,
            mass: callable.mass,
        })
        .collect();
    entities.sort_by(|a, b| {
        (&a.path, a.start_line, a.end_line, &a.name, a.cc, a.sloc)
            .cmp(&(&b.path, b.start_line, b.end_line, &b.name, b.cc, b.sloc))
            .then(a.mass.total_cmp(&b.mass))
    });
    entities
}
