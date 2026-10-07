//! The path-independent payload of one cache entry. The types here are the
//! on-disk format: the IR types they mirror are not serialized directly, so
//! a field added elsewhere never changes a persisted entry silently.

use std::path::Path;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::analysis::{AnalyzedCallable, AnalyzedFinding, FileAnalysis, UnanalyzableReason};
use crate::clones::enumerate_candidates;
use crate::identity::{CallableIdentity, OwnerDigest};
use crate::ir::{CallableKind, DamageKind};
use crate::metrics;
use crate::model::{Callable, LanguageFamily, RuleFinding, RuleId};
use crate::rules::{self, ALL_RULE_IDS};

const HEX_WIDTH: usize = 32;
const HEX_RADIX: u32 = 16;
/// The clone candidates of a payload are numbered against this file index;
/// callers rebind them through `clone_candidates`.
const STORED_FILE_INDEX: u32 = 0;

macro_rules! mirror_enum {
    ($mirror:ident <=> $real:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $mirror {
            $($variant),+
        }

        impl $mirror {
            fn from_ir(real: $real) -> Self {
                match real {
                    $($real::$variant => Self::$variant),+
                }
            }

            fn to_ir(self) -> $real {
                match self {
                    $(Self::$variant => $real::$variant),+
                }
            }
        }
    };
}

mirror_enum!(CachedCallableKind <=> CallableKind {
    JavaMethod,
    JavaConstructor,
    JavaCompactConstructor,
    JavaStaticInitializer,
    JavaLambda,
    JsFunctionDeclaration,
    JsGeneratorFunctionDeclaration,
    JsFunctionExpression,
    JsArrowFunction,
    JsMethodDefinition,
});

mirror_enum!(CachedDamageKind <=> DamageKind {
    TsUsingParameterName,
    JsxUnterminatedEntity,
    Unclassified,
});

fn parse_hex(text: &str) -> Option<u128> {
    if text.len() != HEX_WIDTH || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(text, HEX_RADIX).ok()
}

mod hex_u128 {
    use super::*;

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:032x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_hex(&text).ok_or_else(|| D::Error::custom("not a 32-digit hex value"))
    }
}

mod owner_digest_hex {
    use super::*;

    pub fn serialize<S: Serializer>(value: &OwnerDigest, serializer: S) -> Result<S::Ok, S::Error> {
        hex_u128::serialize(&value.to_bits(), serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OwnerDigest, D::Error> {
        hex_u128::deserialize(deserializer).map(OwnerDigest::from_bits)
    }
}

mod rule_id_name {
    use super::*;

    pub fn serialize<S: Serializer>(value: &RuleId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RuleId, D::Error> {
        let text = String::deserialize(deserializer)?;
        ALL_RULE_IDS
            .iter()
            .find(|id| **id == text)
            .copied()
            .ok_or_else(|| D::Error::custom("unknown rule id"))
    }
}

/// One entry's payload: a full analysis, or the reason a blob cannot be
/// analyzed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedAnalysis {
    Analyzed(Box<AnalyzedPayload>),
    Unanalyzable(CachedReason),
}

/// The only two unanalyzable outcomes decided from a blob's content; the
/// others are decided before any read and never reach the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedReason {
    InvalidEncoding,
    ParserUnavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzedPayload {
    pub damage: Vec<CachedDamage>,
    pub callables: Vec<CachedCallable>,
    pub findings: Vec<CachedFinding>,
    pub executable_lines: Vec<usize>,
    /// Lines the lowering left out of the measurement. No serde default: an
    /// entry written before this field existed is a miss, never a zero.
    pub unanalyzed_lines: usize,
    pub clone_candidates: Vec<CachedCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDamage {
    pub kind: CachedDamageKind,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedIdentity {
    #[serde(with = "owner_digest_hex")]
    pub owner_digest: OwnerDigest,
    pub kind: CachedCallableKind,
    pub name: String,
    pub signature: Vec<String>,
}

/// A callable's metrics without `mass`, which is recomputed on read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCallable {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub cc: u32,
    pub sloc: usize,
    pub identity: CachedIdentity,
    pub body_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedFinding {
    #[serde(with = "rule_id_name")]
    pub rule_id: RuleId,
    pub start_line: usize,
    pub end_line: usize,
    pub flagged_lines: Vec<usize>,
    pub syntax_digest: String,
    pub enclosing_callable: Option<usize>,
}

/// A clone candidate without its `file_index`, stored as a JSON array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "CandidateRecord", from = "CandidateRecord")]
pub struct CachedCandidate {
    #[serde(with = "hex_u128")]
    pub key: u128,
    pub container: u32,
    pub first_statement: u32,
    pub start_line: usize,
    pub end_line: usize,
    pub source_lines: usize,
    pub statement_count: usize,
}

/// The on-disk array form of a `CachedCandidate`, in field order.
#[derive(Serialize, Deserialize)]
struct CandidateRecord(
    #[serde(with = "hex_u128")] u128,
    u32,
    u32,
    usize,
    usize,
    usize,
    usize,
);

impl From<CachedCandidate> for CandidateRecord {
    fn from(candidate: CachedCandidate) -> Self {
        Self(
            candidate.key,
            candidate.container,
            candidate.first_statement,
            candidate.start_line,
            candidate.end_line,
            candidate.source_lines,
            candidate.statement_count,
        )
    }
}

impl From<CandidateRecord> for CachedCandidate {
    fn from(record: CandidateRecord) -> Self {
        Self {
            key: record.0,
            container: record.1,
            first_statement: record.2,
            start_line: record.3,
            end_line: record.4,
            source_lines: record.5,
            statement_count: record.6,
        }
    }
}

/// A parse-damage span as its kind and line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HydratedDamage {
    pub kind: DamageKind,
    pub start_line: u32,
    pub end_line: u32,
}

/// A hit rebuilt for one path.
#[derive(Debug, Clone, PartialEq)]
pub struct Hydrated {
    pub callables: Vec<AnalyzedCallable>,
    pub findings: Vec<AnalyzedFinding>,
    pub damage: Vec<HydratedDamage>,
    pub executable_lines: Vec<usize>,
    pub unanalyzed_lines: usize,
}

impl CachedAnalysis {
    /// Records the unanalyzed-line count of an analyzed payload; an
    /// unanalyzable payload is returned unchanged.
    pub fn with_unanalyzed_lines(mut self, unanalyzed_lines: usize) -> Self {
        if let CachedAnalysis::Analyzed(payload) = &mut self {
            payload.unanalyzed_lines = unanalyzed_lines;
        }
        self
    }

    /// Builds the payload for `analysis`, whose bytes were `source`, under
    /// the effective `min_clone_lines`.
    pub fn from_analysis(analysis: &FileAnalysis, source: &str, min_clone_lines: u32) -> Self {
        let damage = analysis
            .ir
            .damage
            .iter()
            .map(|damage| CachedDamage {
                kind: CachedDamageKind::from_ir(damage.kind),
                start_line: damage.span.start_line,
                end_line: damage.span.end_line,
            })
            .collect();
        let callables = analysis
            .callables
            .iter()
            .map(|callable| CachedCallable {
                name: callable.metrics.name.clone(),
                start_line: callable.metrics.start_line,
                end_line: callable.metrics.end_line,
                cc: callable.metrics.cc,
                sloc: callable.metrics.sloc,
                identity: CachedIdentity {
                    owner_digest: callable.identity.owner_digest,
                    kind: CachedCallableKind::from_ir(callable.identity.kind),
                    name: callable.identity.name.clone(),
                    signature: callable.identity.signature.clone(),
                },
                body_fingerprint: callable.body_fingerprint.clone(),
            })
            .collect();
        let findings = analysis
            .findings
            .iter()
            .map(|analyzed| CachedFinding {
                rule_id: analyzed.finding.rule_id,
                start_line: analyzed.finding.start_line,
                end_line: analyzed.finding.end_line,
                flagged_lines: analyzed.finding.flagged_lines.clone(),
                syntax_digest: analyzed.syntax_digest.clone(),
                enclosing_callable: analyzed.enclosing_callable,
            })
            .collect();
        let clone_candidates = enumerate_candidates(
            analysis.language,
            source,
            &analysis.ir,
            STORED_FILE_INDEX,
            min_clone_lines,
        )
        .into_iter()
        .map(|(key, candidate)| CachedCandidate {
            key,
            container: candidate.container,
            first_statement: candidate.first_statement,
            start_line: candidate.start_line,
            end_line: candidate.end_line,
            source_lines: candidate.source_lines,
            statement_count: candidate.statement_count,
        })
        .collect();
        CachedAnalysis::Analyzed(Box::new(AnalyzedPayload {
            damage,
            callables,
            findings,
            executable_lines: rules::executable_lines_from_ir(&analysis.ir),
            unanalyzed_lines: 0,
            clone_candidates,
        }))
    }

    /// The payload for an unanalyzable blob, or `None` for a reason decided
    /// before any read.
    pub fn unanalyzable(reason: UnanalyzableReason) -> Option<Self> {
        match reason {
            UnanalyzableReason::InvalidEncoding => {
                Some(CachedAnalysis::Unanalyzable(CachedReason::InvalidEncoding))
            }
            UnanalyzableReason::ParserUnavailable => Some(CachedAnalysis::Unanalyzable(
                CachedReason::ParserUnavailable,
            )),
            UnanalyzableReason::NonUtf8Path
            | UnanalyzableReason::UnsupportedExtension
            | UnanalyzableReason::TooLarge => None,
        }
    }

    pub fn unanalyzable_reason(&self) -> Option<UnanalyzableReason> {
        match self {
            CachedAnalysis::Analyzed(_) => None,
            CachedAnalysis::Unanalyzable(CachedReason::InvalidEncoding) => {
                Some(UnanalyzableReason::InvalidEncoding)
            }
            CachedAnalysis::Unanalyzable(CachedReason::ParserUnavailable) => {
                Some(UnanalyzableReason::ParserUnavailable)
            }
        }
    }

    /// Rebuilds the callables and findings for `path` and `language`, or the
    /// reason the blob is unanalyzable.
    pub fn hydrate(
        &self,
        path: &Path,
        language: LanguageFamily,
    ) -> Result<Hydrated, UnanalyzableReason> {
        let payload = match self {
            CachedAnalysis::Analyzed(payload) => payload,
            CachedAnalysis::Unanalyzable(CachedReason::InvalidEncoding) => {
                return Err(UnanalyzableReason::InvalidEncoding)
            }
            CachedAnalysis::Unanalyzable(CachedReason::ParserUnavailable) => {
                return Err(UnanalyzableReason::ParserUnavailable)
            }
        };
        let callables = payload
            .callables
            .iter()
            .map(|callable| AnalyzedCallable {
                metrics: Callable {
                    relative_path: path.to_path_buf(),
                    language,
                    name: callable.name.clone(),
                    start_line: callable.start_line,
                    end_line: callable.end_line,
                    cc: callable.cc,
                    sloc: callable.sloc,
                    mass: metrics::mass(callable.cc, callable.sloc),
                },
                identity: CallableIdentity {
                    owner_digest: callable.identity.owner_digest,
                    kind: callable.identity.kind.to_ir(),
                    name: callable.identity.name.clone(),
                    signature: callable.identity.signature.clone(),
                },
                body_fingerprint: callable.body_fingerprint.clone(),
            })
            .collect();
        let findings = payload
            .findings
            .iter()
            .map(|finding| AnalyzedFinding {
                finding: RuleFinding {
                    relative_path: path.to_path_buf(),
                    language,
                    rule_id: finding.rule_id,
                    start_line: finding.start_line,
                    end_line: finding.end_line,
                    flagged_lines: finding.flagged_lines.clone(),
                },
                syntax_digest: finding.syntax_digest.clone(),
                enclosing_callable: finding.enclosing_callable,
            })
            .collect();
        Ok(Hydrated {
            callables,
            findings,
            damage: payload
                .damage
                .iter()
                .map(|damage| HydratedDamage {
                    kind: damage.kind.to_ir(),
                    start_line: damage.start_line,
                    end_line: damage.end_line,
                })
                .collect(),
            executable_lines: payload.executable_lines.clone(),
            unanalyzed_lines: payload.unanalyzed_lines,
        })
    }

    /// The stored clone candidates bound to `file_index`; empty for an
    /// unanalyzable payload.
    #[cfg(test)]
    pub(crate) fn clone_candidates(
        &self,
        file_index: u32,
    ) -> Vec<(u128, crate::clones::Candidate)> {
        match self {
            CachedAnalysis::Analyzed(payload) => payload
                .clone_candidates
                .iter()
                .map(|stored| {
                    (
                        stored.key,
                        crate::clones::Candidate {
                            file_index,
                            container: stored.container,
                            first_statement: stored.first_statement,
                            start_line: stored.start_line,
                            end_line: stored.end_line,
                            source_lines: stored.source_lines,
                            statement_count: stored.statement_count,
                        },
                    )
                })
                .collect(),
            CachedAnalysis::Unanalyzable(_) => Vec::new(),
        }
    }
}
