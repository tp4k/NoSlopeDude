//! M5-1: the per-blob analysis cache store. Tests that need crate-private
//! types (clone candidates, executable lines, payload tampering) live in
//! `src/cache/tests.rs`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use git2::{ObjectType, Oid, Repository};
use nsd::analysis::{analyze_file, FileAnalysis, UnanalyzableReason};
use nsd::cache::{Cache, CacheError, CacheKey, CachedAnalysis};
use nsd::identity::OwnerDigest;
use nsd::model::{Grammar, LanguageFamily};
use tempfile::TempDir;

const MIN_LINES: u32 = 10;
const MIN_LINES_OTHER: u32 = 11;
const CONCURRENT_THREADS: usize = 8;
const ROUNDS_PER_THREAD: usize = 40;
const ENTRY_EXTENSION: &str = "json";

const JAVA_SOURCE: &str = "class Outer {
    int run(int x) {
        int total = 0;
        for (int i = 0; i < x; i++) {
            total += i;
            total += i * 2;
            total += i * 3;
        }
        try {
            total += x;
        } catch (Exception e) {
        }
        Runnable r = () -> { total(); };
        return total;
    }
    class Inner {
        int deep(int y) {
            if (y > 1 && y < 9) {
                return y;
            }
            return 0;
        }
    }
}
";

fn analyze(path: &str, source: &str) -> FileAnalysis {
    analyze_file(Path::new(path), source.as_bytes()).expect("fixture analyzes")
}

fn blob_oid(source: &str) -> Oid {
    Oid::hash_object(ObjectType::Blob, source.as_bytes()).expect("hash a blob")
}

fn open_cache() -> (TempDir, Repository, Cache) {
    let (dir, repo) = common::init_repo();
    let cache = Cache::open(&repo).expect("open the cache");
    (dir, repo, cache)
}

fn java_key(source: &str) -> CacheKey {
    CacheKey::new(blob_oid(source), Grammar::Java, MIN_LINES)
}

fn payload_of(analysis: &FileAnalysis, source: &str) -> CachedAnalysis {
    CachedAnalysis::from_analysis(analysis, source, MIN_LINES)
}

fn stored_and_read(cache: &Cache, source: &str) -> (CacheKey, CachedAnalysis) {
    let analysis = analyze("A.java", source);
    let key = java_key(source);
    cache
        .put(&key, &payload_of(&analysis, source))
        .expect("put an entry");
    let read = cache.get(&key).expect("read").expect("hit after put");
    (key, read)
}

#[test]
fn test_rehydrated_entry_carries_the_requested_path() {
    let (_dir, _repo, cache) = open_cache();
    let (key, read) = stored_and_read(&cache, JAVA_SOURCE);

    for path in ["one/First.java", "two/deeper/Second.java"] {
        let expected = analyze(path, JAVA_SOURCE);
        let hydrated = read
            .hydrate(Path::new(path), LanguageFamily::Java)
            .expect("hydrates");
        assert!(!expected.callables.is_empty() && !expected.findings.is_empty());
        assert_eq!(hydrated.callables, expected.callables);
        assert_eq!(hydrated.findings, expected.findings);
    }

    let bytes = fs::read(cache.entry_path(&key)).expect("read the entry file");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("First.java") && !text.contains("A.java") && !text.contains("deeper"));
}

#[test]
fn test_recomputed_mass_is_bit_identical() {
    let (_dir, _repo, cache) = open_cache();
    let (_key, read) = stored_and_read(&cache, JAVA_SOURCE);
    let analysis = analyze("A.java", JAVA_SOURCE);
    let hydrated = read
        .hydrate(Path::new("A.java"), LanguageFamily::Java)
        .expect("hydrates");

    assert!(analysis.callables.iter().any(|c| c.metrics.mass > 0.0));
    let analyzed_bits: Vec<u64> = analysis
        .callables
        .iter()
        .map(|c| c.metrics.mass.to_bits())
        .collect();
    let read_bits: Vec<u64> = hydrated
        .callables
        .iter()
        .map(|c| c.metrics.mass.to_bits())
        .collect();
    assert_eq!(read_bits, analyzed_bits);
}

#[test]
fn test_same_blob_other_grammar_misses() {
    let (_dir, _repo, cache) = open_cache();
    let source = "function f(a) { return a; }\n";
    let blob = blob_oid(source);
    let analysis = analyze("a.js", source);
    let payload = payload_of(&analysis, source);

    cache
        .put(
            &CacheKey::new(blob, Grammar::JavaScript, MIN_LINES),
            &payload,
        )
        .expect("put js");
    cache
        .put(
            &CacheKey::new(blob, Grammar::TypeScript, MIN_LINES),
            &payload,
        )
        .expect("put ts");

    assert!(cache
        .get(&CacheKey::new(blob, Grammar::JavaScript, MIN_LINES))
        .expect("read")
        .is_some());
    assert!(cache
        .get(&CacheKey::new(blob, Grammar::Java, MIN_LINES))
        .expect("read")
        .is_none());
    assert!(cache
        .get(&CacheKey::new(blob, Grammar::Tsx, MIN_LINES))
        .expect("read")
        .is_none());
    assert!(cache
        .get(&CacheKey::new(blob, Grammar::TypeScript, MIN_LINES))
        .expect("read")
        .is_some());
}

#[test]
fn test_other_min_clone_lines_misses() {
    let (_dir, _repo, cache) = open_cache();
    let (_key, _read) = stored_and_read(&cache, JAVA_SOURCE);
    let blob = blob_oid(JAVA_SOURCE);

    assert!(cache
        .get(&CacheKey::new(blob, Grammar::Java, MIN_LINES))
        .expect("read")
        .is_some());
    assert!(cache
        .get(&CacheKey::new(blob, Grammar::Java, MIN_LINES_OTHER))
        .expect("read")
        .is_none());
}

#[test]
fn test_corrupt_entry_is_a_miss_and_put_repairs_it() {
    let (_dir, _repo, cache) = open_cache();
    let analysis = analyze("A.java", JAVA_SOURCE);
    let payload = payload_of(&analysis, JAVA_SOURCE);
    let key = java_key(JAVA_SOURCE);
    cache.put(&key, &payload).expect("put");
    let path = cache.entry_path(&key);
    let good = fs::read(&path).expect("read entry");

    let damaged: [Vec<u8>; 4] = [
        good[..good.len() / 2].to_vec(),
        good[..5].to_vec(),
        vec![0xff, 0xfe, 0x00, 0x9c, 0x01],
        Vec::new(),
    ];
    for bytes in damaged {
        fs::write(&path, &bytes).expect("damage entry");
        assert!(cache.get(&key).expect("read").is_none());
        cache.put(&key, &payload).expect("repair");
        assert_eq!(cache.get(&key).expect("read"), Some(payload.clone()));
    }
}

#[test]
fn test_other_cache_version_is_a_miss() {
    let (_dir, _repo, cache) = open_cache();
    let (key, read) = stored_and_read(&cache, JAVA_SOURCE);
    let path = cache.entry_path(&key);
    let bytes = fs::read(&path).expect("read entry");
    let split = bytes.iter().position(|&b| b == b'\n').expect("header line");
    let mut header: serde_json::Value = serde_json::from_slice(&bytes[..split]).expect("header");
    assert_eq!(header["cache_version"], 1);
    header["cache_version"] = serde_json::json!(2);
    let mut rewritten = serde_json::to_vec(&header).expect("encode header");
    rewritten.extend_from_slice(&bytes[split..]);
    fs::write(&path, rewritten).expect("write entry");

    assert!(cache.get(&key).expect("read").is_none());
    cache.put(&key, &read).expect("repair");
    assert_eq!(cache.get(&key).expect("read"), Some(read));
}

#[test]
fn test_entry_under_another_keys_name_is_a_miss() {
    let (_dir, _repo, cache) = open_cache();
    let (key, _read) = stored_and_read(&cache, JAVA_SOURCE);
    let blob = blob_oid(JAVA_SOURCE);
    let other = CacheKey::new(blob, Grammar::TypeScript, MIN_LINES);
    let target = cache.entry_path(&other);
    fs::create_dir_all(target.parent().expect("fan-out dir")).expect("mkdir");
    fs::copy(cache.entry_path(&key), &target).expect("copy entry");

    assert!(cache.get(&other).expect("read").is_none());

    let payload = cache.get(&key).expect("read").expect("stored entry");
    let other_blob = blob_oid("class Other {}\n");
    assert_ne!(blob, other_blob);
    let differing_in_one_part = [
        CacheKey::new(other_blob, Grammar::Java, MIN_LINES),
        CacheKey::new(blob, Grammar::Java, MIN_LINES_OTHER),
    ];
    for other in &differing_in_one_part {
        let target = cache.entry_path(other);
        fs::create_dir_all(target.parent().expect("fan-out dir")).expect("mkdir");
        fs::copy(cache.entry_path(&key), &target).expect("copy entry");

        assert!(cache.get(other).expect("read").is_none());
        cache.put(other, &payload).expect("repair");
        assert_eq!(cache.get(other).expect("read"), Some(payload.clone()));
    }
}

#[test]
fn test_nested_callable_identity_round_trips() {
    let (_dir, _repo, cache) = open_cache();
    let (_key, read) = stored_and_read(&cache, JAVA_SOURCE);
    let analysis = analyze("A.java", JAVA_SOURCE);
    let hydrated = read
        .hydrate(Path::new("A.java"), LanguageFamily::Java)
        .expect("hydrates");

    assert!(analysis
        .callables
        .iter()
        .any(|c| c.identity.owner_digest != OwnerDigest::default()));
    let analyzed: Vec<_> = analysis.callables.iter().map(|c| &c.identity).collect();
    let read_ids: Vec<_> = hydrated.callables.iter().map(|c| &c.identity).collect();
    assert_eq!(read_ids, analyzed);
}

#[test]
fn test_concurrent_writers_never_expose_a_partial_entry() {
    let (_dir, _repo, cache) = open_cache();
    let analysis = analyze("A.java", JAVA_SOURCE);
    let payload = payload_of(&analysis, JAVA_SOURCE);
    let key = java_key(JAVA_SOURCE);
    cache.put(&key, &payload).expect("seed");

    std::thread::scope(|scope| {
        for _ in 0..CONCURRENT_THREADS {
            scope.spawn(|| {
                for _ in 0..ROUNDS_PER_THREAD {
                    cache.put(&key, &payload).expect("concurrent put");
                    assert_eq!(cache.get(&key).expect("read"), Some(payload.clone()));
                }
            });
        }
    });

    let fan_out = cache
        .entry_path(&key)
        .parent()
        .expect("fan-out")
        .to_path_buf();
    let names: Vec<String> = fs::read_dir(&fan_out)
        .expect("read fan-out")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![format!("{}.{ENTRY_EXTENSION}", key.id_hex())]);
}

#[test]
fn test_unwritable_cache_root_reports_an_io_error() {
    let (_dir, repo) = common::init_repo();
    let nsd_dir = repo.commondir().join("nsd");
    fs::write(&nsd_dir, b"not a directory").expect("block nsd");
    assert!(matches!(Cache::open(&repo), Err(CacheError::Io { .. })));
    fs::remove_file(&nsd_dir).expect("unblock");

    let cache = Cache::open(&repo).expect("open");
    fs::remove_dir_all(&nsd_dir).expect("remove nsd");
    fs::write(&nsd_dir, b"not a directory").expect("block nsd again");
    let analysis = analyze("A.java", JAVA_SOURCE);
    let key = java_key(JAVA_SOURCE);

    let result = cache.put(&key, &payload_of(&analysis, JAVA_SOURCE));
    assert!(matches!(result, Err(CacheError::Io { .. })));
    assert!(matches!(cache.get(&key), Err(CacheError::Io { .. })));
}

#[cfg(unix)]
#[test]
fn test_unreadable_entry_is_an_io_error_not_a_miss() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, _repo, cache) = open_cache();
    let (key, _read) = stored_and_read(&cache, JAVA_SOURCE);
    let path = cache.entry_path(&key);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("lock the entry");

    let locked = cache.get(&key);
    let absent = cache.get(&CacheKey::new(
        blob_oid("class Other {}\n"),
        Grammar::Java,
        MIN_LINES,
    ));

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("unlock the entry");
    match locked {
        Err(CacheError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied)
        }
        other => panic!("expected a PermissionDenied I/O error, got {other:?}"),
    }
    assert!(matches!(absent, Ok(None)));
}

#[test]
fn test_linked_worktree_shares_the_cache_root() {
    let (_dir, repo) = common::init_repo();
    common::commit_entries(
        &repo,
        &[(b"A.java".to_vec(), 0o100644, b"class A {}\n".to_vec())],
    );
    let linked_dir = TempDir::new().expect("temp dir");
    let linked_path: PathBuf = linked_dir.path().join("linked");
    repo.worktree("linked", &linked_path, None)
        .expect("add a linked worktree");
    let linked = Repository::open(&linked_path).expect("open linked");

    let main_root = Cache::open(&repo).expect("main cache");
    let linked_root = Cache::open(&linked).expect("linked cache");
    assert_eq!(
        fs::canonicalize(main_root.root()).expect("canonical main"),
        fs::canonicalize(linked_root.root()).expect("canonical linked")
    );
    assert!(main_root.root().ends_with("nsd/cache/v1"));
}

#[test]
fn test_key_is_pinned_and_stable() {
    let (_dir, _repo, cache) = open_cache();
    let blob = Oid::from_str("ce013625030ba8dba906f756967f9e9ca394464a").expect("oid");
    let key = CacheKey::from_parts(blob, Grammar::Java, "fixture-fingerprint");

    assert_eq!(key.id_hex(), PINNED_KEY_HEX);
    let expected_path = cache
        .root()
        .join(&PINNED_KEY_HEX[..2])
        .join(format!("{PINNED_KEY_HEX}.{ENTRY_EXTENSION}"));
    assert_eq!(cache.entry_path(&key), expected_path);
}

const PINNED_KEY_HEX: &str = "c21c2661ff773f69a9839bd920992129";

#[test]
fn test_cache_sources_name_no_default_hasher() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cache");
    let forbidden = ["DefaultHasher", "RandomState", "std::hash::Hasher"];
    let mut scanned = 0;
    for entry in fs::read_dir(&root).expect("read src/cache") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        scanned += 1;
        let text = fs::read_to_string(&path).expect("read source");
        for name in forbidden {
            assert!(!text.contains(name), "{} names {name}", path.display());
        }
    }
    assert!(scanned >= 2, "scanned {scanned} files");
}

#[test]
fn test_unanalyzable_outcome_is_cached() {
    let (_dir, _repo, cache) = open_cache();
    let bytes: &[u8] = &[0x63, 0x6c, 0x61, 0x73, 0x73, 0xff, 0xfe];
    let reason = analyze_file(Path::new("Bad.java"), bytes)
        .err()
        .expect("invalid utf-8");
    assert_eq!(reason, UnanalyzableReason::InvalidEncoding);
    let key = CacheKey::new(
        Oid::hash_object(ObjectType::Blob, bytes).expect("hash"),
        Grammar::Java,
        MIN_LINES,
    );
    let payload = CachedAnalysis::unanalyzable(reason).expect("cacheable reason");
    cache.put(&key, &payload).expect("put");

    let read = cache.get(&key).expect("read").expect("hit");
    assert_eq!(
        read.unanalyzable_reason(),
        Some(UnanalyzableReason::InvalidEncoding)
    );
    assert_eq!(
        read.hydrate(Path::new("Bad.java"), LanguageFamily::Java)
            .err(),
        Some(UnanalyzableReason::InvalidEncoding)
    );
    assert!(CachedAnalysis::unanalyzable(UnanalyzableReason::TooLarge).is_none());
}

#[test]
fn test_unparsable_header_line_is_a_miss() {
    let (_dir, _repo, cache) = open_cache();
    let (key, read) = stored_and_read(&cache, JAVA_SOURCE);
    let path = cache.entry_path(&key);
    let good = fs::read(&path).expect("read entry");
    let split = good.iter().position(|&b| b == b'\n').expect("header line");

    let mut cut_header = good[..split - 3].to_vec();
    cut_header.extend_from_slice(&good[split..]);
    for bytes in [cut_header, b"not json\n{}".to_vec()] {
        fs::write(&path, &bytes).expect("damage entry");
        assert!(cache.get(&key).expect("read").is_none());
        cache.put(&key, &read).expect("repair");
        assert_eq!(cache.get(&key).expect("read"), Some(read.clone()));
    }
}

#[test]
fn test_payload_round_trip_keeps_unanalyzed_lines() {
    const DAMAGED_UNANALYZED_LINES: usize = 2;
    let damaged = include_str!("fixtures/salvage/Mixed.java");
    let analysis = analyze("Mixed.java", damaged);
    assert_eq!(analysis.unanalyzed_lines, DAMAGED_UNANALYZED_LINES);
    let hydrated = CachedAnalysis::from_analysis(&analysis, damaged, MIN_LINES)
        .hydrate(Path::new("Mixed.java"), LanguageFamily::Java)
        .expect("hydrates");
    assert_eq!(hydrated.unanalyzed_lines, analysis.unanalyzed_lines);

    let clean = analyze("A.java", JAVA_SOURCE);
    let hydrated = CachedAnalysis::from_analysis(&clean, JAVA_SOURCE, MIN_LINES)
        .hydrate(Path::new("A.java"), LanguageFamily::Java)
        .expect("hydrates");
    assert_eq!(hydrated.unanalyzed_lines, 0);
}
