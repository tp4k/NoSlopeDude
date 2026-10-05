use std::fs;
use std::path::Path;

use git2::{ObjectType, Oid, Repository};
use tempfile::TempDir;

use super::{Cache, CacheKey, CachedAnalysis};
use crate::analysis::{analyze_file, FileAnalysis};
use crate::clones::enumerate_candidates;
use crate::model::{Grammar, LanguageFamily};
use crate::rules::executable_lines_from_ir;

const MIN_LINES: u32 = 2;
const FILE_INDEX: u32 = 7;
const FORGED_KEY_HEX: &str = "ffffffffffffffffffffffffffffffff";

const CLEAN_SOURCE: &str = "class Outer {
    int run(int x) {
        int total = 0;
        for (int i = 0; i < x; i++) {
            total += i;
            total += i * 2;
        }
        try {
            total += x;
        } catch (Exception e) {
        }
        return total;
    }
}
";

const DAMAGED_SOURCE: &str = "class Outer {
    int run(int x) {
        int total = 0;
        total += x;
        return total;
    }
    int broken( {
        int y = ;
    }
}
";

fn cache() -> (TempDir, Cache) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init");
    let cache = Cache::open(&repo).expect("open");
    (dir, cache)
}

fn analysis_of(source: &str) -> FileAnalysis {
    analyze_file(Path::new("A.java"), source.as_bytes()).expect("analyzes")
}

fn key_of(source: &str) -> CacheKey {
    let blob = Oid::hash_object(ObjectType::Blob, source.as_bytes()).expect("hash");
    CacheKey::new(blob, Grammar::Java, MIN_LINES)
}

type CandidateFields = (u128, u32, u32, u32, usize, usize, usize, usize);

fn candidate_fields(candidates: &[(u128, crate::clones::Candidate)]) -> Vec<CandidateFields> {
    candidates
        .iter()
        .map(|(key, c)| {
            (
                *key,
                c.file_index,
                c.container,
                c.first_statement,
                c.start_line,
                c.end_line,
                c.source_lines,
                c.statement_count,
            )
        })
        .collect()
}

#[test]
fn test_entry_round_trips_every_payload_field() {
    for source in [CLEAN_SOURCE, DAMAGED_SOURCE] {
        let (_dir, cache) = cache();
        let analysis = analysis_of(source);
        let expected_candidates = enumerate_candidates(
            LanguageFamily::Java,
            source,
            &analysis.ir,
            FILE_INDEX,
            MIN_LINES,
        );
        let expected_lines = executable_lines_from_ir(&analysis.ir);
        if source == CLEAN_SOURCE {
            assert!(!expected_candidates.is_empty() && !analysis.findings.is_empty());
        } else {
            assert!(!analysis.ir.damage.is_empty());
        }
        let built = CachedAnalysis::from_analysis(&analysis, source, MIN_LINES);
        let key = key_of(source);
        cache.put(&key, &built).expect("put");

        let read = cache.get(&key).expect("hit");
        assert_eq!(read, built);
        let hydrated = read
            .hydrate(Path::new("A.java"), LanguageFamily::Java)
            .expect("hydrates");
        assert_eq!(hydrated.callables, analysis.callables);
        assert_eq!(hydrated.findings, analysis.findings);
        assert_eq!(hydrated.executable_lines, expected_lines);
        let damage_lines: Vec<(u32, u32)> = hydrated
            .damage
            .iter()
            .map(|d| (d.start_line, d.end_line))
            .collect();
        let expected_damage: Vec<(u32, u32)> = analysis
            .ir
            .damage
            .iter()
            .map(|d| (d.span.start_line, d.span.end_line))
            .collect();
        assert_eq!(damage_lines, expected_damage);
        assert_eq!(
            candidate_fields(&read.clone_candidates(FILE_INDEX)),
            candidate_fields(&expected_candidates)
        );
    }
}

#[test]
fn test_tampered_payload_with_intact_header_is_a_miss() {
    let (_dir, cache) = cache();
    let analysis = analysis_of(CLEAN_SOURCE);
    let built = CachedAnalysis::from_analysis(&analysis, CLEAN_SOURCE, MIN_LINES);
    let key = key_of(CLEAN_SOURCE);
    cache.put(&key, &built).expect("put");
    let candidates = built.clone_candidates(0);
    assert!(!candidates.is_empty());
    let victim = format!("{:032x}", candidates[0].0);
    assert_ne!(victim, FORGED_KEY_HEX);

    let path = cache.entry_path(&key);
    let text = fs::read_to_string(&path).expect("read entry");
    let split = text.find('\n').expect("header line");
    let (header, payload) = text.split_at(split);
    assert!(payload.contains(&victim));
    let forged = payload.replacen(&victim, FORGED_KEY_HEX, 1);
    serde_json::from_str::<serde_json::Value>(&forged).expect("still valid JSON");
    fs::write(&path, format!("{header}{forged}")).expect("write entry");

    assert!(cache.get(&key).is_none());
    cache.put(&key, &built).expect("repair");
    assert_eq!(cache.get(&key), Some(built));
}

const MULTI_LINE_DAMAGE_SOURCE: &str = "class A {
    int ok() {
        return 1;
    }
    @@@ junk
    more junk (
       here
    }
}
";

#[test]
fn test_multi_line_damage_span_round_trips() {
    let (_dir, cache) = cache();
    let analysis = analysis_of(MULTI_LINE_DAMAGE_SOURCE);
    assert!(analysis
        .ir
        .damage
        .iter()
        .any(|damage| damage.span.end_line > damage.span.start_line));
    let key = key_of(MULTI_LINE_DAMAGE_SOURCE);
    cache
        .put(
            &key,
            &CachedAnalysis::from_analysis(&analysis, MULTI_LINE_DAMAGE_SOURCE, MIN_LINES),
        )
        .expect("put");

    let hydrated = cache
        .get(&key)
        .expect("hit")
        .hydrate(Path::new("A.java"), LanguageFamily::Java)
        .expect("hydrates");
    let read: Vec<_> = hydrated
        .damage
        .iter()
        .map(|damage| (damage.kind, damage.start_line, damage.end_line))
        .collect();
    let expected: Vec<_> = analysis
        .ir
        .damage
        .iter()
        .map(|damage| (damage.kind, damage.span.start_line, damage.span.end_line))
        .collect();
    assert_eq!(read, expected);
}

const CANDIDATE_RECORD_LEN: usize = 7;

#[test]
fn test_clone_candidate_is_stored_as_an_array() {
    let (_dir, cache) = cache();
    let analysis = analysis_of(CLEAN_SOURCE);
    let built = CachedAnalysis::from_analysis(&analysis, CLEAN_SOURCE, MIN_LINES);
    let key = key_of(CLEAN_SOURCE);
    cache.put(&key, &built).expect("put");
    let candidates = built.clone_candidates(0);
    assert!(!candidates.is_empty());

    let text = fs::read_to_string(cache.entry_path(&key)).expect("read entry");
    let split = text.find('\n').expect("header line");
    let payload: serde_json::Value = serde_json::from_str(&text[split + 1..]).expect("payload");
    let record = payload["analyzed"]["clone_candidates"][0]
        .as_array()
        .expect("candidate is a JSON array");
    assert_eq!(record.len(), CANDIDATE_RECORD_LEN);
    assert_eq!(record[0], format!("{:032x}", candidates[0].0));
}

const UNKNOWN_RULE_ID: &str = "X999";

#[test]
fn test_unknown_rule_id_is_a_miss() {
    let (_dir, cache) = cache();
    let analysis = analysis_of(CLEAN_SOURCE);
    let built = CachedAnalysis::from_analysis(&analysis, CLEAN_SOURCE, MIN_LINES);
    let key = key_of(CLEAN_SOURCE);
    cache.put(&key, &built).expect("put");
    assert!(!analysis.findings.is_empty());

    let path = cache.entry_path(&key);
    let text = fs::read_to_string(&path).expect("read entry");
    let split = text.find('\n').expect("header line");
    let mut header: serde_json::Value = serde_json::from_str(&text[..split]).expect("header");
    let mut payload: serde_json::Value = serde_json::from_str(&text[split + 1..]).expect("payload");
    payload["analyzed"]["findings"][0]["rule_id"] = serde_json::json!(UNKNOWN_RULE_ID);
    let forged = serde_json::to_vec(&payload).expect("encode payload");
    header["payload_digest"] =
        serde_json::json!(super::digest_hex(super::PAYLOAD_FAMILY_PREFIX, &[&forged]));
    let mut bytes = serde_json::to_vec(&header).expect("encode header");
    bytes.push(b'\n');
    bytes.extend_from_slice(&forged);
    fs::write(&path, bytes).expect("write entry");

    assert!(cache.get(&key).is_none());
    cache.put(&key, &built).expect("repair");
    assert_eq!(cache.get(&key), Some(built));
}
