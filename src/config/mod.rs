//! Strict repository-root `nsd.yml` parsing (WS-3): every invalid shape
//! surfaces as `NSD-C102` (D12-D17); a missing file yields built-in
//! defaults. `measurement.min_clone_lines` is parsed and exposed here but
//! not wired into any fingerprint or cache key (that wiring is the
//! `nsd-v1` freeze's job, out of this track's scope).

use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use git2::Repository;
use ignore::overrides::{Override, OverrideBuilder};
use serde::Deserialize;

use crate::git::snapshot::{
    CommitSnapshot, Entry, IndexSnapshot, WorktreeSnapshot, SOURCE_CEILING_BYTES,
};
use crate::git::GitError;
use crate::model::DEFAULT_MIN_CLONE_LINES;

/// The diagnostic code every invalid `nsd.yml` shape carries (D21): the
/// twelve-code enum is M3's `src/policy/` and is not created here.
pub const CODE_INVALID_CONFIG: &str = "NSD-C102";

/// The informational diagnostic a changed candidate `nsd.yml` carries
/// (M2-2, `nsd-plan-final.md` *Diagnostics*): its raw bytes differ from
/// the base's, whether added, removed or modified.
pub const CODE_CONFIG_CHANGED: &str = "NSD-C101";

/// The repository-root file name the loader looks for (D17): matched by
/// raw entry path bytes, never opened by filesystem name (APFS is
/// case-insensitive, so an `open("nsd.yml")` could read `NSD.yml`).
const CONFIG_FILE_NAME: &str = "nsd.yml";

/// The only supported `nsd.yml` schema version (D12).
const SUPPORTED_VERSION: u32 = 1;

/// Default `output.max_terminal_diagnostics` (`nsd-plan-final.md` M6-M7).
const DEFAULT_MAX_TERMINAL_DIAGNOSTICS: u32 = 50;
/// Default `output.max_agent_diagnostics` (`nsd-plan-final.md` M6-M7).
const DEFAULT_MAX_AGENT_DIAGNOSTICS: u32 = 30;

/// Root passed to `OverrideBuilder` (D15): `Override::matched` strips a
/// common prefix, or else assumes the matched path lives in the same
/// directory as this root, so glob validation and matching depend only on
/// the patterns themselves, never on this placeholder's value.
const GLOB_VALIDATION_ROOT: &str = ".";

/// A strictly parsed, defaulted `nsd.yml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub version: u32,
    /// `None` means omitted (every supported source path); `Some` is
    /// never empty (D14: an explicit `include: []` is `NSD-C102`).
    pub include: Option<Vec<String>>,
    pub exclude: Vec<String>,
    pub measurement: MeasurementConfig,
    pub policy: PolicyConfig,
    pub output: OutputConfig,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            version: SUPPORTED_VERSION,
            include: None,
            exclude: Vec::new(),
            measurement: MeasurementConfig::defaults(),
            policy: PolicyConfig::defaults(),
            output: OutputConfig::defaults(),
        }
    }
}

impl Config {
    /// Parses repository-root `nsd.yml` bytes strictly (D12-D17): unknown
    /// or duplicate fields, unsupported versions or codes, invalid globs,
    /// invalid severities, a non-positive `min_clone_lines`, a present null
    /// value on any key, and an empty tagged scalar on a container key all
    /// produce `NSD-C102`.
    pub fn parse(bytes: &[u8]) -> Result<Config, ConfigError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|err| ConfigError::new(format!("nsd.yml is not valid UTF-8: {err}")))?;
        let raw: RawConfig = serde_yaml_ng::from_str(text).map_err(|err| {
            // A generic parse prefixes the key onto a few errors the typed
            // one reports bare (e.g. `measurement: !!null`), so prefer it
            // whenever it fails too.
            let err = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text)
                .err()
                .unwrap_or(err);
            ConfigError::new(format!("nsd.yml failed to parse: {err}"))
        })?;
        reject_empty_tagged_containers(text)?;
        Config::try_from_raw(raw)
    }

    fn try_from_raw(raw: RawConfig) -> Result<Config, ConfigError> {
        if raw.version != SUPPORTED_VERSION {
            return Err(ConfigError::new(format!(
                "unsupported nsd.yml version {}; only {SUPPORTED_VERSION} is supported",
                raw.version
            )));
        }

        if let Some(include) = &raw.include {
            if include.is_empty() {
                return Err(ConfigError::new(
                    "include: [] is not allowed; omit `include` to select every supported path",
                ));
            }
            build_override(include)?;
        }
        build_override(&raw.exclude)?;

        let measurement = match raw.measurement {
            Some(raw_measurement) => {
                let min_clone_lines = raw_measurement
                    .min_clone_lines
                    .unwrap_or(DEFAULT_MIN_CLONE_LINES);
                if min_clone_lines == 0 {
                    return Err(ConfigError::new(format!(
                        "measurement.min_clone_lines must be a positive integer, got {min_clone_lines}"
                    )));
                }
                MeasurementConfig { min_clone_lines }
            }
            None => MeasurementConfig::defaults(),
        };

        let policy = match raw.policy {
            Some(raw_policy) => PolicyConfig {
                nsd_e101: raw_policy.nsd_e101.unwrap_or(Severity::Deny),
                nsd_e102: raw_policy.nsd_e102.unwrap_or(Severity::Deny),
                nsd_v101: raw_policy.nsd_v101.unwrap_or(Severity::Deny),
                nsd_v102: raw_policy.nsd_v102.unwrap_or(Severity::Deny),
                nsd_s102: raw_policy.nsd_s102.unwrap_or(Severity::Warn),
            },
            None => PolicyConfig::defaults(),
        };

        let output = match raw.output {
            Some(raw_output) => OutputConfig {
                max_terminal_diagnostics: raw_output
                    .max_terminal_diagnostics
                    .unwrap_or(DEFAULT_MAX_TERMINAL_DIAGNOSTICS),
                max_agent_diagnostics: raw_output
                    .max_agent_diagnostics
                    .unwrap_or(DEFAULT_MAX_AGENT_DIAGNOSTICS),
            },
            None => OutputConfig::defaults(),
        };

        Ok(Config {
            version: raw.version,
            include: raw.include,
            exclude: raw.exclude,
            measurement,
            policy,
            output,
        })
    }

    /// Compiles `include`/`exclude` into `ignore::overrides::Override`
    /// matchers (D15), the scope WS-4 consumes. `Config` only ever holds
    /// already-validated patterns (`parse`/`load_from_commit` reject an
    /// invalid one as `NSD-C102`), so rebuilding here is deterministic.
    pub fn compiled_scope(&self) -> Result<CompiledScope, ConfigError> {
        let include = match &self.include {
            Some(patterns) => Some(build_override(patterns)?),
            None => None,
        };
        let exclude = build_override(&self.exclude)?;
        Ok(CompiledScope { include, exclude })
    }
}

/// `measurement.min_clone_lines` (A2): parsed and exposed, not yet wired
/// into any fingerprint or cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurementConfig {
    pub min_clone_lines: u32,
}

impl MeasurementConfig {
    fn defaults() -> MeasurementConfig {
        MeasurementConfig {
            min_clone_lines: DEFAULT_MIN_CLONE_LINES,
        }
    }
}

/// `deny`/`warn`/`off`, exactly as written (D13); any other spelling
/// (including different casing) is an unsupported severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Deny,
    Warn,
    Off,
}

/// The five policy codes this track recognises (D13); any other code,
/// including a short form such as `E101`, is unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyConfig {
    pub nsd_e101: Severity,
    pub nsd_e102: Severity,
    pub nsd_v101: Severity,
    pub nsd_v102: Severity,
    pub nsd_s102: Severity,
}

impl PolicyConfig {
    fn defaults() -> PolicyConfig {
        PolicyConfig {
            nsd_e101: Severity::Deny,
            nsd_e102: Severity::Deny,
            nsd_v101: Severity::Deny,
            nsd_v102: Severity::Deny,
            nsd_s102: Severity::Warn,
        }
    }
}

/// `output.max_terminal_diagnostics`/`output.max_agent_diagnostics` (D16):
/// any non-negative integer fitting `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputConfig {
    pub max_terminal_diagnostics: u32,
    pub max_agent_diagnostics: u32,
}

impl OutputConfig {
    fn defaults() -> OutputConfig {
        OutputConfig {
            max_terminal_diagnostics: DEFAULT_MAX_TERMINAL_DIAGNOSTICS,
            max_agent_diagnostics: DEFAULT_MAX_AGENT_DIAGNOSTICS,
        }
    }
}

/// The compiled `include`/`exclude` glob matchers WS-4 consumes (D15),
/// built with the same glob engine `src/discover.rs` uses.
pub struct CompiledScope {
    pub include: Option<Override>,
    pub exclude: Override,
}

/// A Git-domain or shape error raised while loading `nsd.yml` (D21, 3a): an
/// invalid shape carries `NSD-C102`, but a Git-domain failure (the
/// repository-root blob missing from the ODB, e.g. a partial clone or a
/// corrupted object store) keeps the `GitError`'s own code so it is never
/// reported as an invalid configuration.
#[derive(Debug)]
pub struct ConfigError {
    code: &'static str,
    message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> ConfigError {
        ConfigError {
            code: CODE_INVALID_CONFIG,
            message: message.into(),
        }
    }

    fn from_git(err: GitError) -> ConfigError {
        ConfigError {
            code: err.code(),
            message: format!("cannot read {CONFIG_FILE_NAME}: {err}"),
        }
    }

    /// The stable diagnostic code: `"NSD-C102"` for an invalid shape, or the
    /// wrapped `GitError`'s own code (3a).
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Validates `patterns` (D15: no `!`-negated/re-inclusion pattern, and
/// every pattern a syntactically valid glob) and compiles them into an
/// `Override`.
fn build_override(patterns: &[String]) -> Result<Override, ConfigError> {
    let mut builder = OverrideBuilder::new(Path::new(GLOB_VALIDATION_ROOT));
    for pattern in patterns {
        if pattern.starts_with('!') {
            return Err(ConfigError::new(format!(
                "negated/re-inclusion glob pattern {pattern:?} is not supported"
            )));
        }
        if pattern.trim().is_empty() || pattern.starts_with('#') {
            return Err(ConfigError::new(format!(
                "empty or comment-only glob pattern {pattern:?} is not supported"
            )));
        }
        builder
            .add(pattern)
            .map_err(|err| ConfigError::new(format!("invalid glob pattern {pattern:?}: {err}")))?;
    }
    builder
        .build()
        .map_err(|err| ConfigError::new(format!("failed to compile glob patterns: {err}")))
}

/// Finds the snapshot entry whose raw path is exactly `nsd.yml` (D17): no
/// `sub/nsd.yml`, no `NSD.yml`.
fn find_root_entry(entries: &[Entry]) -> Option<&Entry> {
    entries
        .iter()
        .find(|entry| entry.path.as_bytes() == CONFIG_FILE_NAME.as_bytes())
}

/// Loads `nsd.yml` from a `Commit` snapshot (D17): built-in defaults when
/// no repository-root entry exists, `NSD-C102` for an invalid one.
pub fn load_from_commit(
    repo: &Repository,
    snapshot: &CommitSnapshot,
) -> Result<Config, ConfigError> {
    match root_config_bytes_from_commit(repo, snapshot)? {
        Some(bytes) => Config::parse(&bytes),
        None => Ok(Config::default()),
    }
}

/// Raw repository-root `nsd.yml` bytes from a `Commit` snapshot (M2-2's
/// diff seam, `src/policy`): `Ok(None)` when no root entry exists; an
/// over-ceiling or non-regular entry is `NSD-C102`, and a Git-domain read
/// failure keeps the wrapped `GitError`'s own code (3a).
pub fn root_config_bytes_from_commit(
    repo: &Repository,
    snapshot: &CommitSnapshot,
) -> Result<Option<Vec<u8>>, ConfigError> {
    root_config_bytes(&snapshot.entries, |entry| snapshot.read(repo, entry))
}

/// Raw repository-root `nsd.yml` bytes from an `Index` snapshot (M2-2's
/// diff seam, `--staged`'s candidate): same contract as
/// `root_config_bytes_from_commit`.
pub fn root_config_bytes_from_index(
    repo: &Repository,
    snapshot: &IndexSnapshot,
) -> Result<Option<Vec<u8>>, ConfigError> {
    root_config_bytes(&snapshot.entries, |entry| snapshot.read(repo, entry))
}

/// Raw repository-root `nsd.yml` bytes from a `Worktree` snapshot (M2-2's
/// diff seam): same contract as `root_config_bytes_from_commit`.
pub fn root_config_bytes_from_worktree(
    repo: &Repository,
    snapshot: &WorktreeSnapshot,
) -> Result<Option<Vec<u8>>, ConfigError> {
    root_config_bytes(&snapshot.entries, |entry| snapshot.read(repo, entry))
}

/// Shared by the three `root_config_bytes_from_*` accessors above (reusing
/// `find_root_entry`, D17): `Ok(None)` when no repository-root entry
/// exists, `NSD-C102` when it exceeds `SOURCE_CEILING_BYTES` or is not a
/// regular file, or the wrapped `GitError`'s own code for a Git-domain read
/// failure (3a).
fn root_config_bytes(
    entries: &[Entry],
    read: impl FnOnce(&Entry) -> Result<Option<Vec<u8>>, GitError>,
) -> Result<Option<Vec<u8>>, ConfigError> {
    let Some(entry) = find_root_entry(entries) else {
        return Ok(None);
    };
    let bytes = read(entry).map_err(ConfigError::from_git)?.ok_or_else(|| {
        ConfigError::new(format!(
            "{CONFIG_FILE_NAME} exceeds the {SOURCE_CEILING_BYTES}-byte read ceiling or is \
             not a regular file"
        ))
    })?;
    Ok(Some(bytes))
}

/// Loads a trusted `--config` file from the filesystem (M2-2): bound by
/// `SOURCE_CEILING_BYTES` the same way a snapshot's own bytes are, since
/// this path is attacker-controlled the same way a repository entry is
/// (D21's "candidate config validated through C101 but cannot affect its
/// own check" companion: a trusted file is validated too). Missing,
/// unreadable, over-ceiling and invalid-shape all report `NSD-C102`
/// (`ConfigError::new`'s default code).
///
/// A directory, FIFO, socket or other non-regular path is also
/// `NSD-C102` (A1), checked with `fs::metadata` right before `File::open`
/// so a FIFO with no writer is rejected instead of blocking there
/// indefinitely. This is `fs::metadata`, which follows a symlink, not the
/// snapshot layer's never-follow `fs::symlink_metadata`: a repository
/// entry's on-disk shape is untrusted worktree content, but a trusted
/// `--config` path is operator-chosen, so a symlink to a regular file is
/// meant to keep working. The check still only narrows, not closes, the
/// window where the path is swapped for a non-regular file between it and
/// `File::open` below — the same residual race `WorktreeSnapshot::read`'s
/// D25 re-check documents at `src/git/snapshot.rs:235-237`; closing it
/// needs `O_NONBLOCK`, a `libc` dependency the anti-scope forbids.
pub fn load_trusted(path: &Path) -> Result<Config, ConfigError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        _ => {
            return Err(ConfigError::new(format!(
                "trusted config {} is missing or not a regular file",
                path.display()
            )))
        }
    }
    let file = fs::File::open(path).map_err(|err| {
        ConfigError::new(format!(
            "cannot read trusted config {}: {err}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take(SOURCE_CEILING_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| {
            ConfigError::new(format!(
                "cannot read trusted config {}: {err}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > SOURCE_CEILING_BYTES {
        return Err(ConfigError::new(format!(
            "trusted config {} exceeds the {SOURCE_CEILING_BYTES}-byte read ceiling",
            path.display()
        )));
    }
    Config::parse(&bytes)
}

/// Rejects an empty scalar carrying an explicit tag (`exclude: !!seq`,
/// `policy: !!str`, `output: !custom`) on a container key. It is not a
/// null spelling, so `deserialize_present` lets it through, but
/// `deserialize_seq`/`deserialize_map` then read any empty plain scalar as
/// an empty container (`serde_yaml_ng-0.10.0/src/de.rs:1620-1632`) and the
/// key silently defaults. The typed value carries no tag, so this re-reads
/// the already-valid document as a generic `Value`, where such a scalar is
/// an empty string (a core tag) or a `Tagged` null (any other tag).
fn reject_empty_tagged_containers(text: &str) -> Result<(), ConfigError> {
    use serde_yaml_ng::Value;
    let value: Value = serde_yaml_ng::from_str(text)
        .map_err(|err| ConfigError::new(format!("nsd.yml failed to parse: {err}")))?;
    let Value::Mapping(map) = value else {
        return Ok(());
    };
    for key in ["exclude", "measurement", "policy", "output"] {
        let is_empty_scalar = match map.get(key) {
            Some(Value::String(text)) => text.is_empty(),
            Some(Value::Tagged(tagged)) => match &tagged.value {
                Value::Null => true,
                Value::String(text) => text.is_empty(),
                _ => false,
            },
            _ => false,
        };
        if is_empty_scalar {
            return Err(ConfigError::new(format!(
                "nsd.yml failed to parse: {key}: an empty tagged scalar is not allowed; \
                 write the value out or omit the key"
            )));
        }
    }
    Ok(())
}

/// Deserializes a *present* `include` key as `Vec<String>` (D14: only an
/// *omitted* key means every supported path). `#[serde(default)]` on the
/// field already covers the omitted case without calling this; a present
/// but blank/`~`/`null` value fails here instead of silently becoming
/// `None`.
fn deserialize_present_include<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(Some)
}

/// Deserializes a *present* scalar leaf as `T` (D30, same present-then-
/// `T::deserialize(d).map(Some)` shape as `deserialize_present_include`):
/// `#[serde(default)]` on the field already covers the omitted case
/// without calling this. For a scalar leaf, `T::deserialize` rejects a
/// present null itself (e.g. `u32`/`Severity` deserialization does not
/// accept a unit value or an unknown variant), and serde_yaml_ng prefixes
/// the full key path onto that error
/// (`serde_yaml_ng-0.10.0/src/error.rs:210-212`), so no key name needs to
/// be added here.
fn deserialize_present_scalar<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Rejects a present YAML null (blank, `~`, `null`, `Null`, `NULL`) on
/// `key`, deserializing normally otherwise (D30). Unlike a scalar leaf,
/// `T::deserialize` on `exclude`/`measurement`/`policy`/`output` would
/// itself turn a present null into an empty sequence or an empty mapping
/// (`deserialize_seq`/`deserialize_map`,
/// `serde_yaml_ng-0.10.0/src/de.rs:1612-1683`) instead of erroring, so the
/// null must be caught earlier, through `deserialize_option`. That path
/// does not prefix the key the way `T::deserialize` does — it has no
/// `fix_mark` call (`serde_yaml_ng-0.10.0/src/de.rs:1517-1561`) — so `key`
/// is named in the error explicitly.
fn deserialize_present<'de, D, T>(key: &'static str, deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct RejectNull<T> {
        key: &'static str,
        marker: std::marker::PhantomData<T>,
    }

    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for RejectNull<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a present, non-null {}", self.key)
        }

        fn visit_none<E>(self) -> Result<T, E>
        where
            E: serde::de::Error,
        {
            Err(E::custom(format!(
                "{}: null value is not allowed",
                self.key
            )))
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<T, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            T::deserialize(deserializer)
        }
    }

    deserializer.deserialize_option(RejectNull {
        key,
        marker: std::marker::PhantomData,
    })
}

/// Deserializes a *present* `exclude` key as `Vec<String>`, rejecting a
/// present null instead of letting it become `[]` (D30).
fn deserialize_present_exclude<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_present("exclude", deserializer)
}

/// Deserializes a *present* `measurement` key, rejecting a present null
/// instead of letting it become an empty mapping (D30).
fn deserialize_present_measurement<'de, D>(
    deserializer: D,
) -> Result<Option<RawMeasurement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_present("measurement", deserializer).map(Some)
}

/// Deserializes a *present* `policy` key, rejecting a present null instead
/// of letting it become an empty mapping (D30).
fn deserialize_present_policy<'de, D>(deserializer: D) -> Result<Option<RawPolicy>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_present("policy", deserializer).map(Some)
}

/// Deserializes a *present* `output` key, rejecting a present null instead
/// of letting it become an empty mapping (D30).
fn deserialize_present_output<'de, D>(deserializer: D) -> Result<Option<RawOutput>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_present("output", deserializer).map(Some)
}

/// The raw, unvalidated shape `serde_yaml_ng` deserializes `nsd.yml`
/// into: strict at every level (`deny_unknown_fields`), so an unknown or
/// duplicate field at any depth fails before semantic validation runs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u32,
    #[serde(default, deserialize_with = "deserialize_present_include")]
    include: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_present_exclude")]
    exclude: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_present_measurement")]
    measurement: Option<RawMeasurement>,
    #[serde(default, deserialize_with = "deserialize_present_policy")]
    policy: Option<RawPolicy>,
    #[serde(default, deserialize_with = "deserialize_present_output")]
    output: Option<RawOutput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMeasurement {
    #[serde(default, deserialize_with = "deserialize_present_scalar")]
    min_clone_lines: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    #[serde(
        rename = "NSD-E101",
        default,
        deserialize_with = "deserialize_present_scalar"
    )]
    nsd_e101: Option<Severity>,
    #[serde(
        rename = "NSD-E102",
        default,
        deserialize_with = "deserialize_present_scalar"
    )]
    nsd_e102: Option<Severity>,
    #[serde(
        rename = "NSD-V101",
        default,
        deserialize_with = "deserialize_present_scalar"
    )]
    nsd_v101: Option<Severity>,
    #[serde(
        rename = "NSD-V102",
        default,
        deserialize_with = "deserialize_present_scalar"
    )]
    nsd_v102: Option<Severity>,
    #[serde(
        rename = "NSD-S102",
        default,
        deserialize_with = "deserialize_present_scalar"
    )]
    nsd_s102: Option<Severity>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    #[serde(default, deserialize_with = "deserialize_present_scalar")]
    max_terminal_diagnostics: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_scalar")]
    max_agent_diagnostics: Option<u32>,
}
