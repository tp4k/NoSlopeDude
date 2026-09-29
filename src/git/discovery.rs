//! Git-backed discovery (WS-4): classifies every entry of a WS-1 snapshot
//! as included (with language, OID, size and the `too_large`/
//! `non_utf8_path` flags) or skipped with an explicit reason, independent
//! of any candidate `.gitignore` (`nsd-plan-final.md` *M1-M2*). Immutable
//! built-in exclusions (dependency/build/generated/minified/WebJar paths
//! and any `.git` path component) cannot be re-added by `include`; D18:
//! this module emits classifications, not diagnostics — turning
//! `too_large`/`non_utf8_path` into `A102` is M3-M5.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::Oid;
use ignore::overrides::{Override, OverrideBuilder};

use crate::config::CompiledScope;
use crate::model::LanguageFamily;

use super::path::RepoPath;
use super::snapshot::{Entry, EntryKind, SOURCE_CEILING_BYTES};

/// Root passed to `OverrideBuilder` for the built-in exclusion set: every
/// `Entry::path` is already repository-relative, so glob compilation and
/// matching depend only on the patterns themselves (mirrors WS-3's
/// `GLOB_VALIDATION_ROOT`, `src/config/mod.rs`).
const GLOB_VALIDATION_ROOT: &str = ".";

/// The `.git` path component every built-in exclusion also checks for
/// directly (D19 "defence in depth"): trees/indexes cannot normally carry
/// one (D7), but a directory-anchored glob alone would not catch it at
/// arbitrary depth without the ancestor walk below, so it is checked as
/// its own rule instead.
const GIT_DIR_NAME: &str = ".git";

/// `src/discover.rs::DEPENDENCY_BUILD_GLOBS` contents, copied verbatim
/// (D19): that module is outside this task's edit scope and its list is
/// private.
const DEPENDENCY_BUILD_GLOBS: &[&str] = &[
    "node_modules/",
    "target/",
    "build/",
    "dist/",
    "out/",
    "bin/",
    ".gradle/",
    ".mvn/",
    "vendor/",
    "coverage/",
    ".next/",
    ".nuxt/",
];

/// `src/discover.rs::GENERATED_GLOBS` contents, copied verbatim (D19).
const GENERATED_GLOBS: &[&str] = &[
    "**/generated/**",
    "**/gen/**",
    "*.min.js",
    "*.bundle.js",
    "*_pb.js",
    "*.d.ts",
];

/// WebJar resource paths (D19), the one built-in exclusion this stream
/// adds beyond the scan-discovery list.
const WEBJAR_GLOB: &str = "**/META-INF/resources/webjars/**";

/// One entry the compiled scope includes, carrying enough for M3-M5's
/// later checks to work from without re-reading the snapshot (D18:
/// classification, not diagnostics).
#[derive(Debug, Clone)]
pub struct IncludedEntry {
    pub path: RepoPath,
    pub language: LanguageFamily,
    pub oid: Option<Oid>,
    pub size: u64,
    /// `size` exceeds `SOURCE_CEILING_BYTES` (M3's `A102`, not raised here).
    pub too_large: bool,
    /// The raw path bytes are not valid UTF-8 (M3's `A102`, not raised
    /// here); `path.render()` is still the deterministic D3 rendering.
    pub non_utf8_path: bool,
}

/// The reason one entry was not included (D20: first match wins, in this
/// order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Symlink,
    Submodule,
    NestedCheckout,
    /// D26: a non-regular worktree entry (`EntryKind::Special`) — a FIFO,
    /// socket, block or character device — extension-gated like `Symlink`.
    SpecialFile,
    BuiltinExclusion,
    OutsideInclude,
    ConfigExclude,
}

impl SkipReason {
    /// The D20 spelling of this reason, for M3+ callers (reports, JSON) that
    /// need a stable string without inventing their own (mirrors the
    /// sibling `nsd::model::SkipReason::label`).
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::Symlink => "symlink",
            SkipReason::Submodule => "submodule",
            SkipReason::NestedCheckout => "nested_checkout",
            SkipReason::SpecialFile => "special_file",
            SkipReason::BuiltinExclusion => "builtin_exclusion",
            SkipReason::OutsideInclude => "outside_include",
            SkipReason::ConfigExclude => "config_exclude",
        }
    }
}

/// One entry that was walked but not included, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub path: RepoPath,
    pub reason: SkipReason,
}

/// Every included and skipped entry from one classification pass, both
/// sorted by raw path bytes (D10).
#[derive(Debug, Clone, Default)]
pub struct DiscoveryResult {
    pub included: Vec<IncludedEntry>,
    pub skipped: Vec<SkippedEntry>,
}

/// Classifies every entry of a WS-1 snapshot against `scope` (D20/D26
/// precedence: entry kind — symlink/submodule/nested_checkout/special_file
/// — then built-in exclusion, then outside-include, then config-exclude).
/// An entry without a supported extension (`LanguageFamily::from_extension`)
/// is not listed at all, except a submodule or nested-checkout entry,
/// which is always listed as skipped regardless of its path's extension
/// (a `Special` entry follows the same extension gate as `Symlink`, D26).
pub fn discover(entries: &[Entry], scope: &CompiledScope) -> DiscoveryResult {
    let builtins = builtin_override();
    let mut included = Vec::new();
    let mut skipped = Vec::new();
    // One directory-verdict memo per matcher (D19's ancestor walk), so a
    // directory shared by many sibling entries is matched at most once per
    // matcher per `discover` call, not once per entry (perf, triage row 7).
    // A memo is valid only for the matcher it was filled against (D31), so
    // `include` gets its own map rather than sharing `exclude`'s.
    let mut builtin_dir_verdicts: HashMap<&[u8], bool> = HashMap::new();
    let mut include_dir_verdicts: HashMap<&[u8], bool> = HashMap::new();
    let mut exclude_dir_verdicts: HashMap<&[u8], bool> = HashMap::new();

    for entry in entries {
        match entry.kind {
            EntryKind::Submodule => {
                skipped.push(skip(entry, SkipReason::Submodule));
                continue;
            }
            EntryKind::NestedCheckout => {
                skipped.push(skip(entry, SkipReason::NestedCheckout));
                continue;
            }
            _ => {}
        }

        let Some(extension) = extension_of(entry.path.as_bytes()) else {
            continue; // Unsupported extension: not listed at all (D20).
        };
        let Some(language) = LanguageFamily::from_extension(extension) else {
            continue;
        };

        if entry.kind == EntryKind::Symlink {
            skipped.push(skip(entry, SkipReason::Symlink));
            continue;
        }
        if entry.kind == EntryKind::Special {
            skipped.push(skip(entry, SkipReason::SpecialFile));
            continue;
        }
        // `Submodule`/`NestedCheckout` entries never reach this call (they
        // are skipped above), so `discover`'s own verdicts never depend on
        // `is_dir`: every entry left standing here is a file.
        if is_builtin_excluded(&entry.path, false, &builtins, &mut builtin_dir_verdicts) {
            skipped.push(skip(entry, SkipReason::BuiltinExclusion));
            continue;
        }
        if is_outside_include(&entry.path, &scope.include, &mut include_dir_verdicts) {
            skipped.push(skip(entry, SkipReason::OutsideInclude));
            continue;
        }
        if is_config_excluded(&entry.path, &scope.exclude, &mut exclude_dir_verdicts) {
            skipped.push(skip(entry, SkipReason::ConfigExclude));
            continue;
        }

        included.push(IncludedEntry {
            path: entry.path.clone(),
            language,
            oid: entry.oid,
            size: entry.size,
            too_large: entry.size > SOURCE_CEILING_BYTES,
            non_utf8_path: std::str::from_utf8(entry.path.as_bytes()).is_err(),
        });
    }

    included.sort_by(|a, b| a.path.cmp(&b.path));
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    DiscoveryResult { included, skipped }
}

fn skip(entry: &Entry, reason: SkipReason) -> SkippedEntry {
    SkippedEntry {
        path: entry.path.clone(),
        reason,
    }
}

/// Whether `path` is a built-in exclusion (D19): any `.git` path
/// component, or a match (direct or via an ancestor directory) against the
/// dependency/build, generated or WebJar glob lists. D29: exposed to
/// `diff.rs` so WS-4's worktree-diff mask (D28) and `discover` share one
/// predicate, never a second copy of the glob lists.
///
/// `is_dir` is whether `path` itself denotes a directory (a `NestedCheckout`
/// or `Submodule` worktree/tree entry, WS-4 r2 row 1): `matches_including_
/// ancestors` always matches `path` itself with `is_dir = false`, since it
/// exists to catch a *file* nested under a directory-anchored glob such as
/// `target/`, so it never matches the glob against the directory entry
/// itself. When `is_dir` is true, `dir_verdict` is also consulted directly
/// against `path`'s own bytes, checking `path` as a directory (and, through
/// its own recursion, every ancestor above it too).
pub(super) fn is_builtin_excluded<'e>(
    path: &'e RepoPath,
    is_dir: bool,
    builtins: &Override,
    dir_verdicts: &mut HashMap<&'e [u8], bool>,
) -> bool {
    has_git_component(path.as_bytes())
        || matches_including_ancestors(builtins, path, dir_verdicts)
        || (is_dir && dir_verdict(builtins, path.as_bytes(), dir_verdicts))
}

/// Whether `path` falls outside `include` (D20's `outside_include`);
/// `None` means every supported path is in scope. A `Some` matcher gets
/// the same ancestor-directory walk as `exclude` (D31), so a
/// directory-anchored `include: ["src/"]` covers files nested beneath it,
/// not only entries whose own path matches directly.
fn is_outside_include<'e>(
    path: &'e RepoPath,
    include: &Option<Override>,
    dir_verdicts: &mut HashMap<&'e [u8], bool>,
) -> bool {
    match include {
        None => false,
        Some(matcher) => !matches_including_ancestors(matcher, path, dir_verdicts),
    }
}

/// Whether `path` is subtracted by `exclude` (D20's `config_exclude`); the
/// same ancestor-directory walk as the built-in check applies here too,
/// since a user's directory-anchored exclude pattern has the same
/// single-entry-only matching limitation. `exclude` is empty by default
/// (no user `exclude:` entries), so that case returns before the allocation
/// `matches_including_ancestors` would otherwise pay on every entry.
fn is_config_excluded<'e>(
    path: &'e RepoPath,
    exclude: &Override,
    dir_verdicts: &mut HashMap<&'e [u8], bool>,
) -> bool {
    if exclude.is_empty() {
        return false;
    }
    matches_including_ancestors(exclude, path, dir_verdicts)
}

/// Checks `path` itself (unmemoised: each entry's own path is only ever
/// checked once), then its parent directory's memoised verdict, against
/// `matcher` (D19: mirrors `src/discover.rs`'s private `is_ignored`,
/// `src/discover.rs:178-198`, since a directory-anchored override glob such
/// as `node_modules/` only matches the directory entry itself). `dir_verdicts`
/// is shared across every entry `discover` classifies with this matcher, so
/// a directory is matched against `matcher` at most once, however many
/// sibling files it contains.
fn matches_including_ancestors<'e>(
    matcher: &Override,
    path: &'e RepoPath,
    dir_verdicts: &mut HashMap<&'e [u8], bool>,
) -> bool {
    let bytes = path.as_bytes();
    if matcher
        .matched(path_for_matching(bytes), false)
        .is_whitelist()
    {
        return true;
    }
    let parent = match bytes.iter().rposition(|&byte| byte == b'/') {
        Some(slash) => &bytes[..slash],
        None => return false, // No parent directory at all.
    };
    dir_verdict(matcher, parent, dir_verdicts)
}

/// The memoised verdict of directory `dir` against `matcher`: whether `dir`
/// itself matches, or its parent's verdict does, recursing up to the root
/// (whose parent, the empty prefix, is always `false`). Computed once per
/// distinct directory per `dir_verdicts` map, then looked up by every later
/// entry or ancestor that shares it.
fn dir_verdict<'e>(
    matcher: &Override,
    dir: &'e [u8],
    dir_verdicts: &mut HashMap<&'e [u8], bool>,
) -> bool {
    if dir.is_empty() {
        return false;
    }
    if let Some(&verdict) = dir_verdicts.get(dir) {
        return verdict;
    }
    let parent = match dir.iter().rposition(|&byte| byte == b'/') {
        Some(slash) => &dir[..slash],
        None => &[][..],
    };
    let verdict = matcher.matched(path_for_matching(dir), true).is_whitelist()
        || dir_verdict(matcher, parent, dir_verdicts);
    dir_verdicts.insert(dir, verdict);
    verdict
}

/// Whether any `/`-separated component of `path` is literally `.git`
/// (D19 defence in depth).
fn has_git_component(path: &[u8]) -> bool {
    path.split(|&byte| byte == b'/')
        .any(|component| component == GIT_DIR_NAME.as_bytes())
}

/// The extension of the last path component, if any, as a `&str`; the
/// bytes before it may still be invalid UTF-8 (a non-UTF-8 path ending in
/// a plain-ASCII extension is still classified, D3/D18). A name that is
/// only a leading dot (e.g. `.gitignore`) has no extension.
fn extension_of(path: &[u8]) -> Option<&str> {
    let file_name = path.rsplit(|&byte| byte == b'/').next().unwrap_or(path);
    let dot = file_name.iter().rposition(|&byte| byte == b'.')?;
    if dot == 0 {
        return None;
    }
    std::str::from_utf8(&file_name[dot + 1..]).ok()
}

/// Builds the immutable built-in exclusion matcher (D19) from the
/// dependency/build, generated and WebJar glob lists above. These are
/// fixed, already-tested glob strings (verbatim copies of a working
/// module's own constants), so a compile failure here can only be an
/// internal defect caught by this module's own tests. D29: exposed to
/// `diff.rs`, same reason as `is_builtin_excluded` above.
pub(super) fn builtin_override() -> Override {
    let mut builder = OverrideBuilder::new(Path::new(GLOB_VALIDATION_ROOT));
    let globs = DEPENDENCY_BUILD_GLOBS
        .iter()
        .copied()
        .chain(GENERATED_GLOBS.iter().copied())
        .chain(std::iter::once(WEBJAR_GLOB));
    for glob in globs {
        builder.add(glob).expect("built-in exclusion glob compiles");
    }
    builder.build().expect("built-in exclusion set compiles")
}

/// Converts repository-relative raw bytes into a standalone `Path` for
/// `Override::matched` (never joined with a workdir, unlike
/// `snapshot::repo_path_to_fs`): on unix, rebuilt from its exact bytes so
/// no byte is ever substituted; only a platform without a byte-oriented
/// `OsStr` falls back to a lossy conversion.
#[cfg(unix)]
fn path_for_matching(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_for_matching(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
