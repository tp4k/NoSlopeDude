//! M5-2: best-effort LRU eviction of the cache store.

mod common;

use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use git2::{ObjectType, Oid};
use nsd::analysis::analyze_file;
use nsd::cache::{Cache, CacheKey, CachedAnalysis, EvictionLimits};
use nsd::model::Grammar;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const KIB: usize = 1024;
const MIN_LINES: u32 = 10;
const ROOT_COMPONENTS: [&str; 3] = ["nsd", "cache", "v1"];
const UNLIMITED: EvictionLimits = EvictionLimits {
    max_bytes: u64::MAX,
    max_age: Duration::from_secs(30 * 24 * 60 * 60),
};

fn plant_file(path: &Path, size: usize, age: Duration, now: SystemTime) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("create the fan-out dir");
    fs::write(path, vec![b'x'; size]).expect("write the file");
    let file = File::options().write(true).open(path).expect("open to age");
    file.set_times(FileTimes::new().set_modified(now - age))
        .expect("set the mtime");
}

fn entry_name(index: u32) -> String {
    format!("{index:02x}{index:030x}")
}

fn entry_at(root: &Path, index: u32) -> PathBuf {
    let name = entry_name(index);
    root.join(&name[..2]).join(format!("{name}.json"))
}

fn plant_entry(root: &Path, index: u32, size: usize, age: Duration, now: SystemTime) -> PathBuf {
    let path = entry_at(root, index);
    plant_file(&path, size, age, now);
    path
}

fn cache_root(repo: &git2::Repository) -> PathBuf {
    ROOT_COMPONENTS
        .iter()
        .fold(repo.commondir().to_path_buf(), |path, part| path.join(part))
}

fn open_empty() -> (tempfile::TempDir, git2::Repository, Cache) {
    let (dir, repo) = common::init_repo();
    let cache = Cache::open(&repo).expect("open the cache");
    (dir, repo, cache)
}

#[test]
fn test_default_limits_are_one_gib_and_thirty_days() {
    assert_eq!(EvictionLimits::DEFAULT.max_bytes, 1 << 30);
    assert_eq!(EvictionLimits::DEFAULT.max_age, DAY * 30);
}

#[test]
fn test_entries_older_than_the_age_limit_are_evicted() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let ages: [u32; 6] = [31, 31, 29, 29, 1, 1];
    let paths: Vec<PathBuf> = ages
        .iter()
        .enumerate()
        .map(|(index, days)| plant_entry(cache.root(), index as u32, KIB, DAY * *days, now))
        .collect();

    cache.evict(UNLIMITED, now);

    let kept: Vec<bool> = paths.iter().map(|path| path.exists()).collect();
    assert_eq!(kept, [false, false, true, true, true, true]);
}

#[test]
fn test_size_cap_evicts_least_recently_used_first() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let paths: Vec<PathBuf> = (0..5u32)
        .map(|index| plant_entry(cache.root(), index, KIB, DAY * (5 - index), now))
        .collect();
    let cap = |bytes: usize| EvictionLimits {
        max_bytes: bytes as u64,
        ..UNLIMITED
    };

    cache.evict(cap(3 * KIB), now);
    let kept: Vec<bool> = paths.iter().map(|path| path.exists()).collect();
    assert_eq!(kept, [false, false, true, true, true]);

    cache.evict(cap(3 * KIB - 1), now);
    let kept: Vec<bool> = paths.iter().map(|path| path.exists()).collect();
    assert_eq!(kept, [false, false, false, true, true]);
}

#[test]
fn test_size_exactly_at_the_cap_evicts_nothing() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let paths: Vec<PathBuf> = (0..3u32)
        .map(|index| plant_entry(cache.root(), index, KIB, DAY * (3 - index), now))
        .collect();

    let removed = cache.evict(
        EvictionLimits {
            max_bytes: 3 * KIB as u64,
            ..UNLIMITED
        },
        now,
    );

    assert_eq!(removed, 0);
    assert!(paths.iter().all(|path| path.exists()));
}

fn real_entry(cache: &Cache, source: &str) -> (CacheKey, PathBuf) {
    let analysis = analyze_file(Path::new("a.js"), source.as_bytes()).expect("fixture analyzes");
    let blob = Oid::hash_object(ObjectType::Blob, source.as_bytes()).expect("hash a blob");
    let key = CacheKey::new(blob, Grammar::JavaScript, MIN_LINES);
    cache
        .put(
            &key,
            &CachedAnalysis::from_analysis(&analysis, source, MIN_LINES),
        )
        .expect("put an entry");
    let path = cache.entry_path(&key);
    (key, path)
}

fn age_to(path: &Path, age: Duration, now: SystemTime) {
    let file = File::options().write(true).open(path).expect("open to age");
    file.set_times(FileTimes::new().set_modified(now - age))
        .expect("set the mtime");
}

#[test]
fn test_a_hit_refreshes_recency() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let (key_a, path_a) = real_entry(&cache, "function a(x) { return x; }\n");
    let (_key_b, path_b) = real_entry(&cache, "function b(x, y) { return x + y; }\n");
    age_to(&path_a, DAY * 10, now);
    age_to(&path_b, DAY * 5, now);
    let cap = fs::metadata(&path_a)
        .expect("size a")
        .len()
        .max(fs::metadata(&path_b).expect("size b").len());

    assert!(cache.get(&key_a).is_some());
    cache.evict(
        EvictionLimits {
            max_bytes: cap,
            ..UNLIMITED
        },
        SystemTime::now(),
    );

    assert!(path_a.exists(), "the entry that was hit survives");
    assert!(!path_b.exists(), "the entry nobody hit is evicted");
}

#[test]
fn test_stale_temp_files_are_evicted_fresh_ones_kept() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let fan_out = cache.root().join("ab");
    let stale = fan_out.join(".tmpStale1");
    let fresh = fan_out.join(".tmpFresh2");
    plant_file(&stale, KIB, DAY * 31, now);
    plant_file(&fresh, KIB, DAY, now);

    cache.evict(UNLIMITED, now);

    assert!(!stale.exists(), "an orphaned temp file is removed");
    assert!(fresh.exists(), "a writer in flight keeps its temp file");
}

#[cfg(unix)]
#[test]
fn test_only_entries_and_temp_files_in_real_fan_out_dirs_are_touched() {
    let (dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let outside = dir.path().join("outside");
    let victim = entry_at(&outside, 1);
    plant_file(&victim, KIB, DAY * 40, now);
    let outside_target = outside.join("target.bin");
    plant_file(&outside_target, KIB, DAY * 40, now);
    std::os::unix::fs::symlink(victim.parent().expect("dir"), cache.root().join("cd"))
        .expect("symlinked fan-out dir");
    let real = cache.root().join("ab");
    fs::create_dir_all(&real).expect("real fan-out dir");
    std::os::unix::fs::symlink(
        &outside_target,
        real.join(format!("{}.json", entry_name(2))),
    )
    .expect("symlinked entry");
    let stranger = real.join("notes.txt");
    plant_file(&stranger, KIB, DAY * 40, now);
    let top_level = cache.root().join("README");
    plant_file(&top_level, KIB, DAY * 40, now);

    cache.evict(UNLIMITED, now);

    assert!(victim.exists() && outside_target.exists());
    assert!(stranger.exists() && top_level.exists());
}

#[cfg(unix)]
#[test]
fn test_unremovable_entry_is_skipped_and_the_pass_continues() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let locked = plant_entry(cache.root(), 1, KIB, DAY * 40, now);
    let removable = plant_entry(cache.root(), 0x20, KIB, DAY * 40, now);
    let locked_dir = locked.parent().expect("dir").to_path_buf();
    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o500)).expect("lock the dir");

    let removed = cache.evict(UNLIMITED, now);

    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o700)).expect("unlock the dir");
    assert!(locked.exists(), "an entry that cannot be removed stays");
    assert!(!removable.exists(), "the pass goes on to the next entry");
    assert_eq!(removed, 1);
}

#[test]
fn test_eviction_runs_at_most_once_per_day() {
    let (_dir, repo) = common::init_repo();
    let root = cache_root(&repo);
    let t0 = SystemTime::now();
    let first = plant_entry(&root, 1, KIB, DAY * 40, t0);

    Cache::open_at(&repo, UNLIMITED, t0).expect("first open");
    assert!(!first.exists(), "the first open runs the pass");

    let second = plant_entry(&root, 2, KIB, DAY * 40, t0);
    Cache::open_at(&repo, UNLIMITED, t0 + DAY / 24).expect("open within a day");
    assert!(second.exists(), "a second open within 24 h does not");

    Cache::open_at(&repo, UNLIMITED, t0 + DAY - Duration::from_secs(1))
        .expect("open at 24 h - 1 s");
    assert!(second.exists(), "just under 24 h does not");

    Cache::open_at(&repo, UNLIMITED, t0 + DAY + Duration::from_secs(1))
        .expect("open at 24 h + 1 s");
    assert!(!second.exists(), "24 h + 1 s after the stamp runs it again");
}

#[test]
fn test_plain_open_runs_the_pass() {
    let (_dir, repo) = common::init_repo();
    let stale = plant_entry(&cache_root(&repo), 1, KIB, DAY * 40, SystemTime::now());

    Cache::open(&repo).expect("open");

    assert!(!stale.exists());
}

#[test]
fn test_linked_worktree_open_evicts_the_shared_cache_once() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"A.java".to_vec(), 0o100644, b"class A {}\n".to_vec())],
    );
    let linked_dir = tempfile::TempDir::new().expect("temp dir");
    let linked_path = linked_dir.path().join("linked");
    repo.worktree("linked", &linked_path, None)
        .expect("add a linked worktree");
    let linked = git2::Repository::open(&linked_path).expect("open linked");
    let root = cache_root(&repo);
    let t0 = SystemTime::now();
    let first = plant_entry(&root, 1, KIB, DAY * 40, t0);

    Cache::open_at(&linked, UNLIMITED, t0).expect("open from the linked worktree");
    assert!(!first.exists(), "the pass reaches the shared cache");

    let second = plant_entry(&root, 2, KIB, DAY * 40, t0);

    Cache::open_at(&repo, UNLIMITED, t0 + DAY / 24).expect("open from the main worktree");
    assert!(second.exists(), "the stamp is shared by both worktrees");
}

#[cfg(unix)]
#[test]
fn test_a_symlinked_entry_is_not_an_entry_under_any_cap() {
    let (dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let target = dir.path().join("target.bin");
    plant_file(&target, KIB, DAY, now);
    let real = cache.root().join("ab");
    fs::create_dir_all(&real).expect("real fan-out dir");
    let link = real.join(format!("{}.json", entry_name(2)));
    std::os::unix::fs::symlink(&target, &link).expect("symlinked entry");

    let removed = cache.evict(
        EvictionLimits {
            max_bytes: 0,
            ..UNLIMITED
        },
        now,
    );

    assert_eq!(removed, 0);
    assert!(link.symlink_metadata().is_ok() && target.exists());
}

#[test]
fn test_an_out_of_range_stamp_counts_as_due() {
    let (_dir, repo) = common::init_repo();
    let root = cache_root(&repo);
    let now = SystemTime::now();
    let stale = plant_entry(&root, 1, KIB, DAY * 31, now);
    fs::write(root.join("eviction-stamp"), u64::MAX.to_string()).expect("plant the stamp");

    Cache::open_at(&repo, EvictionLimits::DEFAULT, now).expect("open with an out-of-range stamp");

    assert!(!stale.exists(), "an unreadable stamp means the pass is due");
}

#[cfg(unix)]
#[test]
fn test_unwritable_stamp_skips_the_pass() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, repo) = common::init_repo();
    let root = cache_root(&repo);
    let t0 = SystemTime::now();
    let later = t0 + DAY + DAY / 24;
    Cache::open_at(&repo, UNLIMITED, t0).expect("first open");
    let stale = plant_entry(&root, 1, KIB, DAY * 31, later);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).expect("lock the root");

    let reopened = Cache::open_at(&repo, UNLIMITED, later);

    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("unlock the root");
    reopened.expect("open succeeds without a stamp");
    assert!(stale.exists(), "no stamp could be written, so no pass runs");
}

#[test]
fn test_a_hit_on_a_fresh_entry_keeps_its_mtime() {
    let (_dir, _repo, cache) = open_empty();
    let now = SystemTime::now();
    let (key, path) = real_entry(&cache, "function a(x) { return x; }\n");
    age_to(&path, DAY / 2, now);
    let before = fs::metadata(&path).expect("stat").modified().expect("mtime");

    assert!(cache.get(&key).is_some());

    let after = fs::metadata(&path).expect("stat").modified().expect("mtime");
    assert_eq!(after, before, "an entry under a day old is not touched");
}
