//! Base-policy trust resolution (M2-2, `nsd-plan-implementation.md`
//! step 3): the effective `Config` always comes from a trusted `--config`
//! file or the **base** snapshot, never from the candidate under
//! inspection — "candidate config is validated and reported through
//! `C101` but cannot affect its own check" (`nsd-plan-implementation.md:76`).
//! Exit-code mapping for the diagnostics this module reports is M5-3's job,
//! not this one's.

use std::path::Path;

use git2::{ObjectType, Oid, Repository};

use crate::config::{self, Config, ConfigError, CODE_CONFIG_CHANGED, CODE_INVALID_CONFIG};
use crate::git::snapshot::{CommitSnapshot, IndexSnapshot, WorktreeSnapshot};

/// Where the effective `Config` in a `Resolution` came from (Settled
/// decisions: "Config trust | Policy read from the base snapshot").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// A trusted `--config` file, which completely replaces repository
    /// policy for this invocation.
    Trusted,
    /// The base snapshot's own `nsd.yml`.
    Base,
    /// Neither a trusted file nor a base `nsd.yml` exists.
    BuiltInDefaults,
}

/// One candidate-configuration diagnostic: only the stable code, since
/// severity and exit-code mapping are M5-3's job. `PolicyConfig`'s
/// `Severity` is for the five policy codes a parsed `Config` carries, not
/// these two fixed-severity ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
}

/// The candidate snapshot kind `resolve` compares against the base:
/// `--staged` reads only the index, a worktree run reads the worktree, and
/// a second commit (e.g. in a test) reads a commit.
pub enum Candidate<'a> {
    Commit(&'a CommitSnapshot),
    Index(&'a IndexSnapshot),
    Worktree(&'a WorktreeSnapshot),
}

/// The effective config, its source, and the candidate's diagnostics.
#[derive(Debug)]
pub struct Resolution {
    pub config: Config,
    pub source: ConfigSource,
    pub diagnostics: Vec<Diagnostic>,
}

/// How `diagnostics_for_candidate` decides whether the candidate's
/// `nsd.yml` changed (A4): by raw bytes, the base case, or — when the base
/// itself could not be read (trusted mode's "an unreadable base must not
/// fail this resolution") — by blob object id, using whatever snapshot
/// metadata is still available.
enum BaseIdentity<'a> {
    Bytes(Option<&'a Vec<u8>>),
    Oid(Option<Oid>),
}

/// Fetches the candidate's raw repository-root `nsd.yml` bytes (M2-2's diff
/// seam), dispatching on which snapshot kind it is.
fn fetch_candidate_bytes(
    repo: &Repository,
    candidate: &Candidate<'_>,
) -> Result<Option<Vec<u8>>, ConfigError> {
    match candidate {
        Candidate::Commit(snapshot) => config::root_config_bytes_from_commit(repo, snapshot),
        Candidate::Index(snapshot) => config::root_config_bytes_from_index(repo, snapshot),
        Candidate::Worktree(snapshot) => config::root_config_bytes_from_worktree(repo, snapshot),
    }
}

/// The candidate diagnostics for one invocation (M2-2's diff seam): an
/// over-ceiling or non-regular candidate `nsd.yml` is reported as
/// `[NSD-C101, NSD-C102]` rather than failing the whole resolution — "only
/// a changed candidate is validated ... cannot affect its own check"
/// applies to that read failure too. Any other (Git-domain) candidate
/// read error still propagates, since it is not the candidate's shape at
/// fault.
///
/// In `BaseIdentity::Oid` mode (A4), "changed" is decided from object ids
/// rather than bytes: a commit/index candidate's blob oid is already known
/// from its own snapshot entry (`Entry.oid`, D2), so an unchanged one is
/// never even read; a worktree entry carries no oid, so that case reads
/// the candidate once and hashes those same bytes with `Oid::hash_object`
/// (no ODB write, and no second, raw-fd read the way
/// `git::diff::worktree_blob_oid`'s over-ceiling fallback does).
fn diagnostics_for_candidate(
    repo: &Repository,
    candidate: Candidate<'_>,
    base: BaseIdentity<'_>,
) -> Result<Vec<Diagnostic>, ConfigError> {
    match base {
        BaseIdentity::Bytes(base_bytes) => {
            let candidate_bytes = match fetch_candidate_bytes(repo, &candidate) {
                Ok(bytes) => bytes,
                Err(err) if err.code() == CODE_INVALID_CONFIG => {
                    return Ok(vec![
                        Diagnostic {
                            code: CODE_CONFIG_CHANGED,
                        },
                        Diagnostic {
                            code: CODE_INVALID_CONFIG,
                        },
                    ]);
                }
                Err(err) => return Err(err),
            };

            let mut diagnostics = Vec::new();
            if candidate_bytes.as_ref() != base_bytes {
                diagnostics.push(Diagnostic {
                    code: CODE_CONFIG_CHANGED,
                });
                if let Some(bytes) = &candidate_bytes {
                    if Config::parse(bytes).is_err() {
                        diagnostics.push(Diagnostic {
                            code: CODE_INVALID_CONFIG,
                        });
                    }
                }
            }
            Ok(diagnostics)
        }
        BaseIdentity::Oid(base_oid) => {
            // A worktree entry never carries an oid (D2), so only that
            // case needs a read; a commit/index candidate's oid comes
            // straight from its own snapshot entry, and an unchanged one is
            // never read at all (the precise gap `test_trusted_config_
            // overrides_an_unreadable_base` pins: base and candidate are
            // both the same oversized, otherwise-unreadable commit).
            let (candidate_oid, candidate_bytes) = match &candidate {
                Candidate::Commit(snapshot) => (
                    config::find_root_entry(&snapshot.entries).and_then(|entry| entry.oid),
                    None,
                ),
                Candidate::Index(snapshot) => (
                    config::find_root_entry(&snapshot.entries).and_then(|entry| entry.oid),
                    None,
                ),
                Candidate::Worktree(_) => {
                    let bytes = match fetch_candidate_bytes(repo, &candidate) {
                        Ok(bytes) => bytes,
                        Err(err) if err.code() == CODE_INVALID_CONFIG => {
                            return Ok(vec![
                                Diagnostic {
                                    code: CODE_CONFIG_CHANGED,
                                },
                                Diagnostic {
                                    code: CODE_INVALID_CONFIG,
                                },
                            ]);
                        }
                        Err(err) => return Err(err),
                    };
                    let oid = match &bytes {
                        Some(content) => Some(
                            Oid::hash_object(ObjectType::Blob, content).map_err(|err| {
                                ConfigError::new(format!(
                                    "cannot hash worktree nsd.yml content: {err}"
                                ))
                            })?,
                        ),
                        None => None,
                    };
                    (oid, bytes)
                }
            };
            let mut candidate_bytes = candidate_bytes;

            if base_oid == candidate_oid {
                return Ok(Vec::new());
            }

            let mut diagnostics = vec![Diagnostic {
                code: CODE_CONFIG_CHANGED,
            }];
            if candidate_bytes.is_none() && !matches!(candidate, Candidate::Worktree(_)) {
                candidate_bytes = match fetch_candidate_bytes(repo, &candidate) {
                    Ok(bytes) => bytes,
                    Err(err) if err.code() == CODE_INVALID_CONFIG => {
                        diagnostics.push(Diagnostic {
                            code: CODE_INVALID_CONFIG,
                        });
                        return Ok(diagnostics);
                    }
                    Err(err) => return Err(err),
                };
            }
            if let Some(bytes) = &candidate_bytes {
                if Config::parse(bytes).is_err() {
                    diagnostics.push(Diagnostic {
                        code: CODE_INVALID_CONFIG,
                    });
                }
            }
            Ok(diagnostics)
        }
    }
}

/// Resolves the effective `Config` for one invocation (M2-2):
///
/// - A trusted `config_path`, when given, completely replaces repository
///   policy: the base is never parsed, and a missing, unreadable,
///   over-ceiling or invalid-shape trusted file is `NSD-C102`.
/// - Otherwise the base snapshot's own `nsd.yml` governs (built-in
///   defaults when it has none); an invalid base is `NSD-C102`.
/// - Independently of the source above, a candidate whose raw `nsd.yml`
///   bytes differ from the base's carries `NSD-C101`; only a changed
///   candidate is then parsed, and an invalid one additionally carries
///   `NSD-C102` — but this never changes the effective `Config`.
pub fn resolve(
    repo: &Repository,
    config_path: Option<&Path>,
    base: &CommitSnapshot,
    candidate: Candidate<'_>,
) -> Result<Resolution, ConfigError> {
    if let Some(path) = config_path {
        // A trusted config completely replaces repository policy, so an
        // unreadable base must not fail this resolution (triage-ws2-r1.md
        // row 1). Its raw bytes are then unavailable for the candidate
        // diff, but its tree entry still carries a blob oid (A4), so the
        // diff falls back to comparing object ids instead of skipping it
        // outright — the candidate is still diffed and independently
        // validated, exactly as it would be with a readable base.
        let diagnostics = match config::root_config_bytes_from_commit(repo, base) {
            Ok(base_bytes) => diagnostics_for_candidate(
                repo,
                candidate,
                BaseIdentity::Bytes(base_bytes.as_ref()),
            )?,
            Err(_) => {
                let base_oid = config::find_root_entry(&base.entries).and_then(|entry| entry.oid);
                diagnostics_for_candidate(repo, candidate, BaseIdentity::Oid(base_oid))?
            }
        };
        let config = config::load_trusted(path)?;
        return Ok(Resolution {
            config,
            source: ConfigSource::Trusted,
            diagnostics,
        });
    }

    let base_bytes = config::root_config_bytes_from_commit(repo, base)?;
    let diagnostics =
        diagnostics_for_candidate(repo, candidate, BaseIdentity::Bytes(base_bytes.as_ref()))?;

    match base_bytes {
        Some(bytes) => Ok(Resolution {
            config: Config::parse(&bytes)?,
            source: ConfigSource::Base,
            diagnostics,
        }),
        None => Ok(Resolution {
            config: Config::default(),
            source: ConfigSource::BuiltInDefaults,
            diagnostics,
        }),
    }
}
