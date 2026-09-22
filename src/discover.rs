//! Walks a scanned root and classifies every file as discovered or skipped.
//!
//! The walk itself is `ignore::WalkBuilder`; every exclusion — `.gitignore`,
//! the D16 default exclusions, and the user's `--exclude` globs — is a set
//! of negated globs added through `ignore::overrides::OverrideBuilder::add`.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use ignore::overrides::{Override, OverrideBuilder};
use ignore::{DirEntry, WalkBuilder};

use crate::model::{DiscoveredFile, LanguageFamily, ScanSettings, SkipReason, SkippedFile};

/// The name of the directory every walk below refuses to descend into.
const GIT_DIR_NAME: &str = ".git";

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

const GENERATED_GLOBS: &[&str] = &[
    "**/generated/**",
    "**/gen/**",
    "*.min.js",
    "*.bundle.js",
    "*_pb.js",
    "*.d.ts",
];

const TEST_GLOBS: &[&str] = &[
    "**/test/**",
    "**/tests/**",
    "**/__tests__/**",
    "**/__mocks__/**",
    "**/testFixtures/**",
    "**/src/test/**",
    "*Test.java",
    "*Tests.java",
    "*TestCase.java",
    "*.test.*",
    "*.spec.*",
];

/// Every discovered and skipped file from one walk of a scanned root.
#[derive(Debug, Default)]
pub struct DiscoverResult {
    pub discovered: Vec<DiscoveredFile>,
    pub skipped: Vec<SkippedFile>,
}

/// Walks `root` and classifies every one of the seven scanned extensions as
/// discovered or skipped, recording which rule skipped it.
///
/// Discovery is two walks, not three: `raw` walks everything but the
/// combined-override-excluded directories and `.git` (kept lean, purely to
/// recover which files a plain `.gitignore` alone removed); `gitignore_only`
/// applies only real `.gitignore` files. `discovered` and the D16/user/test
/// skip bucket are both derived from `gitignore_only` in one pass with
/// `is_ignored(&combined, ..)` — a file's fate never depends on glob
/// evaluation order within `combined`, since `combined` is entirely
/// `!`-prefixed globs (see `reuse:`).
pub fn discover(root: &Path, settings: &ScanSettings) -> anyhow::Result<DiscoverResult> {
    let user_exclude = build_override(root, settings.exclude.iter().map(String::as_str))?;
    let generated = build_override(root, GENERATED_GLOBS.iter().copied())?;
    let dependency_build = build_override(root, DEPENDENCY_BUILD_GLOBS.iter().copied())?;
    let combined = build_combined_override(root, settings)?;

    let mut unreadable = Vec::new();
    let mut unreadable_from_gitignore_only = Vec::new();
    let root_owned = root.to_path_buf();

    let raw = walk_known_extensions(
        root,
        {
            let combined = combined.clone();
            move |builder: &mut WalkBuilder| {
                builder.standard_filters(false);
                builder.filter_entry(move |entry| walkable_by_raw(entry, &root_owned, &combined));
            }
        },
        &mut unreadable,
    )?;
    let gitignore_only = walk_known_extensions(
        root,
        |builder: &mut WalkBuilder| {
            builder.hidden(false).git_ignore(true).require_git(false);
            builder.filter_entry(|entry| !is_git_dir(entry));
        },
        &mut unreadable_from_gitignore_only,
    )?;

    let mut discovered: Vec<DiscoveredFile> = gitignore_only
        .iter()
        .filter(|relative_path| !is_ignored(&combined, relative_path))
        .filter_map(|relative_path| {
            let extension = extension_of(relative_path)?;
            let language = LanguageFamily::from_extension(extension)?;
            Some(DiscoveredFile {
                relative_path: relative_path.clone(),
                language,
            })
        })
        .collect();
    discovered.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let mut skipped: Vec<SkippedFile> = raw
        .difference(&gitignore_only)
        .map(|relative_path| SkippedFile {
            relative_path: relative_path.clone(),
            reason: SkipReason::Gitignore,
        })
        .chain(
            gitignore_only
                .iter()
                .filter(|relative_path| is_ignored(&combined, relative_path))
                .map(|relative_path| {
                    let reason = classify_reason(
                        relative_path,
                        &user_exclude,
                        &generated,
                        &dependency_build,
                    );
                    SkippedFile {
                        relative_path: relative_path.clone(),
                        reason,
                    }
                }),
        )
        .chain(unreadable.into_iter().map(|relative_path| SkippedFile {
            relative_path,
            reason: SkipReason::Unreadable,
        }))
        .collect();
    skipped.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(DiscoverResult {
        discovered,
        skipped,
    })
}

/// The caller only ever passes a file matched by `combined` (see
/// `build_combined_override`), whose glob groups are exactly `user_exclude`,
/// `generated`, `dependency_build`, and the D16 test globs, in that
/// precedence order — so a file that matches none of the first three matched
/// the test globs.
fn classify_reason(
    relative_path: &Path,
    user_exclude: &Override,
    generated: &Override,
    dependency_build: &Override,
) -> SkipReason {
    if is_ignored(user_exclude, relative_path) {
        SkipReason::UserExclude
    } else if is_ignored(generated, relative_path) {
        SkipReason::GeneratedCode
    } else if is_ignored(dependency_build, relative_path) {
        SkipReason::DependencyOrBuildOutput
    } else {
        SkipReason::Test
    }
}

/// Checks `relative_path` itself, then each of its ancestor directories,
/// against `matcher`. A directory-anchored glob such as `node_modules/` only
/// matches the directory entry itself (`Override::matched` has no notion of
/// "and everything below it" for a single path), so a nested file is only
/// caught by walking up to that ancestor and testing it with `is_dir: true`.
fn is_ignored(matcher: &Override, relative_path: &Path) -> bool {
    if matcher.matched(relative_path, false).is_ignore() {
        return true;
    }
    let mut ancestor = relative_path.parent();
    while let Some(dir) = ancestor {
        if dir.as_os_str().is_empty() {
            break;
        }
        if matcher.matched(dir, true).is_ignore() {
            return true;
        }
        ancestor = dir.parent();
    }
    false
}

fn build_combined_override(root: &Path, settings: &ScanSettings) -> anyhow::Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    add_ignored_globs(&mut builder, DEPENDENCY_BUILD_GLOBS.iter().copied())?;
    add_ignored_globs(&mut builder, GENERATED_GLOBS.iter().copied())?;
    if !settings.include_tests {
        add_ignored_globs(&mut builder, TEST_GLOBS.iter().copied())?;
    }
    add_ignored_globs(&mut builder, settings.exclude.iter().map(String::as_str))?;
    builder
        .build()
        .context("failed to build the combined exclusion override set")
}

fn build_override<'a>(
    root: &Path,
    globs: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    add_ignored_globs(&mut builder, globs)?;
    builder.build().context("failed to build an override set")
}

fn add_ignored_globs<'a>(
    builder: &mut OverrideBuilder,
    globs: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<()> {
    for glob in globs {
        builder
            .add(&format!("!{glob}"))
            .with_context(|| format!("invalid exclusion glob {glob:?}"))?;
    }
    Ok(())
}

/// Walks `root`, collecting every entry with one of the seven scanned
/// extensions. A per-entry walk error on a *sub-path* (D18: outside the
/// three reserved fatal cases) is recorded in `unreadable` by path and the
/// walk continues instead of failing the whole scan. A walk error on the
/// scan root itself, or one that carries no path at all, stays fatal — D18
/// reserves "an unreadable target" as one of the three fatal conditions, and
/// an error with no path cannot be attributed to a sub-path safely. Only
/// `strip_prefix` on an entry that *did* resolve staying fatal too, since an
/// entry the walker itself yielded can only fail to be under `root` if
/// something is deeply wrong with the walk.
fn walk_known_extensions(
    root: &Path,
    configure: impl FnOnce(&mut WalkBuilder),
    unreadable: &mut Vec<PathBuf>,
) -> anyhow::Result<HashSet<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    configure(&mut builder);
    let mut files = HashSet::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let errored = error_path(&error);
                if errored.as_deref().is_none_or(|path| path == root) {
                    return Err(anyhow::Error::new(error))
                        .context("failed to walk the scanned root");
                }
                if let Some(path) = errored {
                    unreadable.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
                }
                continue;
            }
        };
        let is_file = entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
        if !is_file {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .context("walked entry escaped the scanned root")?
            .to_path_buf();
        if extension_of(&relative)
            .and_then(LanguageFamily::from_extension)
            .is_some()
        {
            files.insert(relative);
        }
    }
    Ok(files)
}

/// Whether `raw` should yield (and, for a directory, descend into) `entry`:
/// never `.git`, and never a directory the combined override set already
/// excludes — `node_modules/`, `target/`, `build/`, `dist/` and the rest of
/// D16's dependency/build directories are exactly the ones a real project's
/// `.gitignore` doesn't need to repeat, so leaving them unpruned here is what
/// made `raw` descend into all of them on every scan.
fn walkable_by_raw(entry: &DirEntry, root: &Path, combined: &Override) -> bool {
    if is_git_dir(entry) {
        return false;
    }
    let is_dir = entry
        .file_type()
        .map(|file_type| file_type.is_dir())
        .unwrap_or(false);
    if !is_dir {
        return true;
    }
    let relative = match entry.path().strip_prefix(root) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative,
        _ => return true,
    };
    !combined.matched(relative, true).is_ignore()
}

/// Whether `entry` is a `.git` directory — excluded from every walk so none
/// of them descends into it.
fn is_git_dir(entry: &DirEntry) -> bool {
    let is_dir = entry
        .file_type()
        .map(|file_type| file_type.is_dir())
        .unwrap_or(false);
    is_dir && entry.file_name() == OsStr::new(GIT_DIR_NAME)
}

/// Recovers the path a walk error is about, if any. `ignore` wraps a
/// walkdir I/O error (e.g. permission denied on `read_dir`) as
/// `WithPath { path, err: WithDepth { err: Io(..), .. } }`; a symlink loop
/// carries its own `child` path instead.
fn error_path(error: &ignore::Error) -> Option<PathBuf> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path.clone()),
        ignore::Error::WithLineNumber { err, .. } => error_path(err),
        ignore::Error::WithDepth { err, .. } => error_path(err),
        ignore::Error::Partial(errors) => errors.iter().find_map(error_path),
        ignore::Error::Loop { child, .. } => Some(child.clone()),
        _ => None,
    }
}

fn extension_of(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}
