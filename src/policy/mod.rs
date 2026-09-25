//! Base-policy trust resolution (M2-2, `nsd-plan-implementation.md`
//! step 3): the effective `Config` always comes from a trusted `--config`
//! file or the **base** snapshot, never from the candidate under
//! inspection — "candidate config is validated and reported through
//! `C101` but cannot affect its own check" (`nsd-plan-implementation.md:76`).
//! Exit-code mapping for the diagnostics this module reports is M5-3's job,
//! not this one's.

use std::path::Path;

use git2::Repository;

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
    let base_bytes = config::root_config_bytes_from_commit(repo, base)?;
    let candidate_bytes = match candidate {
        Candidate::Commit(snapshot) => config::root_config_bytes_from_commit(repo, snapshot)?,
        Candidate::Index(snapshot) => config::root_config_bytes_from_index(repo, snapshot)?,
        Candidate::Worktree(snapshot) => config::root_config_bytes_from_worktree(repo, snapshot)?,
    };

    let mut diagnostics = Vec::new();
    if candidate_bytes != base_bytes {
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

    if let Some(path) = config_path {
        let config = config::load_trusted(path)?;
        return Ok(Resolution {
            config,
            source: ConfigSource::Trusted,
            diagnostics,
        });
    }

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
