//! Contract tests for the assertion, cache and baseline vocabulary.
//!
//! These protect observable evaluation behavior: evidence-less assertions are refused
//! before any source read, cache identity follows span/input and versions, and
//! the baseline keeps `unknown` as a measured outcome.

use crate::assertion::{
    Anchor, Assertion, Decision, Evidence, Generator, BASELINE_GENERATOR_ID,
    BASELINE_RULE_VERSION, JUDGE_PROMPT_VERSION,
};
use crate::baseline;
use crate::source::EvidenceVerifier;
use crate::store::Store;

fn anchor(ordinal: u32, len: u32, digest: &str) -> Anchor {
    Anchor {
        ordinal,
        entry_id: format!("entry{ordinal}"),
        off: u64::from(ordinal) * 100,
        len,
        line_sha256: digest.to_string(),
    }
}

fn digest(byte: char) -> String {
    std::iter::repeat(byte).take(64).collect()
}

fn assertion(label: Decision) -> Assertion {
    Assertion {
        unit_id: digest('a'),
        decision_index: 0,
        generator: Generator::Slm,
        rule_version: crate::assertion::SLM_PROMPT_VERSION.to_string(),
        generator_id: "local-model#0123456789ab".to_string(),
        evidence: Evidence {
            prev: anchor(1, 20, &digest('b')),
            curr: anchor(2, 20, &digest('c')),
        },
        render_sha256: digest('d'),
        label,
        confidence: Some("medium".to_string()),
        abstain_reason: None,
        context: None,
        verification_only: false,
    }
}

struct PanicVerifier;

impl EvidenceVerifier for PanicVerifier {
    fn verify_source(&self, _assertion: &Assertion) -> ase_core::Result<()> {
        Err(ase_core::err("source was read before evidence validation"))
    }
}

#[test]
fn insert_rejects_missing_evidence_before_reading_source() {
    let mut store = Store::open(":memory:").expect("store");
    let mut broken = assertion(Decision::Continue);
    broken.evidence.prev.len = 0;
    broken.evidence.prev.line_sha256 = String::new();
    let error = store
        .insert(&broken, &PanicVerifier, "run-test", "2026-11-23T00:00:00.000Z")
        .err()
        .expect("evidence-less assertion must be rejected");
    assert!(
        error.0.contains("evidence") || error.0.contains("digest") || error.0.contains("span"),
        "unexpected rejection reason: {}",
        error.0
    );
    assert_eq!(store.count().expect("count"), 0);
}

#[test]
fn insert_rejects_a_source_mismatch_and_stores_nothing() {
    let mut store = Store::open(":memory:").expect("store");
    let error = store
        .insert(
            &assertion(Decision::Continue),
            &PanicVerifier,
            "run-test",
            "2026-11-23T00:00:00.000Z",
        )
        .err()
        .expect("a failing source reopen must reject the assertion");
    assert!(error.0.contains("source was read"));
    assert_eq!(store.count().expect("count"), 0);
}

#[test]
fn cache_key_follows_input_span_and_version_not_whole_file() {
    let base = assertion(Decision::Continue);
    let same = base.clone();
    assert_eq!(base.cache_key(), same.cache_key());

    let mut shifted = base.clone();
    shifted.evidence.curr.off += 1;
    assert_ne!(
        base.cache_key(),
        shifted.cache_key(),
        "a moved span address must invalidate the cached judgement"
    );

    let mut changed = base.clone();
    changed.evidence.curr.line_sha256 = digest('e');
    assert_ne!(
        base.cache_key(),
        changed.cache_key(),
        "a changed decision span must invalidate the cache"
    );

    let mut versioned = base.clone();
    versioned.rule_version = "slm-boundary-v3".to_string();
    assert_ne!(base.cache_key(), versioned.cache_key());

    let mut other_model = base.clone();
    other_model.generator_id = "local-model#ffffffffffff".to_string();
    assert_ne!(base.cache_key(), other_model.cache_key());

    let mut verification = base.clone();
    verification.verification_only = true;
    assert_ne!(
        base.cache_key(),
        verification.cache_key(),
        "verification-only judgements must never enter the evaluation namespace"
    );
}

#[test]
fn judge_input_depends_on_candidate_labels() {
    let mut judge = assertion(Decision::Continue);
    judge.generator = Generator::Judge;
    judge.rule_version = JUDGE_PROMPT_VERSION.to_string();
    judge.context = Some(serde_json::json!({"baseline": "continue", "slm": "split"}));
    let with_split = judge.input_sha256();
    judge.context = Some(serde_json::json!({"baseline": "continue", "slm": "unknown"}));
    assert_ne!(with_split, judge.input_sha256());
}

#[test]
fn baseline_is_deterministic_and_preserves_unknown() {
    let split = baseline::classify("earlier request", "new task: build the report");
    assert_eq!(split.label, Decision::Split);
    assert_eq!(split.confidence, "high");
    assert_eq!(
        split,
        baseline::classify("earlier request", "new task: build the report")
    );

    let ack = baseline::classify("we finished the work", "thanks");
    assert_eq!(ack.label, Decision::Unknown);
    assert_eq!(ack.abstain_reason, Some("acknowledgement_only"));

    let undecided = baseline::classify("some instruction", "a detached observation");
    assert_eq!(undecided.label, Decision::Unknown);
    assert!(undecided.abstain_reason.is_some());
    assert_eq!(BASELINE_GENERATOR_ID, "baseline-rules");
    assert_eq!(BASELINE_RULE_VERSION, "baseline-rubric-v2");
}

#[test]
fn generation_rubric_keeps_verbal_confidence_typed() {
    let (label, confidence, abstain) = crate::rubric::parse_generation(
        "prefix {\"label\":\"continue\",\"confidence\":\"HIGH\",\"abstain_reason\":null} suffix",
    )
    .expect("valid generation answer");
    assert_eq!(label, Decision::Continue);
    assert_eq!(
        confidence,
        Some(crate::rubric::ModelConfidence::Verbal("high".to_string()))
    );
    assert!(abstain.is_none());

    let (label, confidence, abstain) = crate::rubric::parse_generation(
        "{\"label\":\"unknown\",\"confidence\":\"low\",\"abstain_reason\":null}",
    )
    .expect("unknown without a reason");
    assert_eq!(label, Decision::Unknown);
    assert_eq!(
        confidence,
        Some(crate::rubric::ModelConfidence::Verbal("low".to_string()))
    );
    assert_eq!(abstain.as_deref(), Some("model_chose_unknown"));

    assert_eq!(
        crate::rubric::parse_generation("{\"label\":\"maybe\"}").unwrap_err(),
        "generation_invalid_label"
    );
    assert_eq!(
        crate::rubric::parse_generation(
            "{\"label\":\"split\",\"confidence\":\"certain\",\"abstain_reason\":null}"
        )
        .unwrap_err(),
        "generation_invalid_confidence"
    );
    assert_eq!(
        crate::rubric::parse_generation("no object").unwrap_err(),
        "generation_missing_json_object"
    );
}

#[test]
fn decision_rubric_keeps_probability_semantics_separate() {
    let (label, confidence, abstain) =
        crate::rubric::map_choice("unknown", 0.51).expect("typed choice");
    assert_eq!(label, Decision::Unknown);
    assert_eq!(
        confidence,
        crate::rubric::ModelConfidence::Probability(0.51)
    );
    assert_eq!(confidence.semantics(), "probability");
    assert_eq!(confidence.encode(), "probability:0.51");
    assert_eq!(abstain.as_deref(), Some("model_chose_unknown"));

    let (label, confidence, abstain) =
        crate::rubric::map_choice("split", 0.9).expect("typed choice");
    assert_eq!(label, Decision::Split);
    assert_eq!(confidence.encode(), "probability:0.9");
    assert!(abstain.is_none());

    assert_eq!(
        crate::rubric::map_choice("other", 0.9).unwrap_err(),
        "decision_unknown_choice"
    );
    assert_eq!(
        crate::rubric::map_choice("split", 1.5).unwrap_err(),
        "decision_invalid_confidence"
    );
}

#[test]
fn confidence_decoding_preserves_legacy_verbal_values() {
    assert_eq!(
        crate::rubric::decode_confidence("high"),
        ("verbal", serde_json::json!("high"))
    );
    assert_eq!(
        crate::rubric::decode_confidence("verbal:medium"),
        ("verbal", serde_json::json!("medium"))
    );
    assert_eq!(
        crate::rubric::decode_confidence("probability:0.875"),
        ("probability", serde_json::json!(0.875))
    );
}

#[test]
fn api_backends_change_the_cache_namespace_by_protocol_and_identity() {
    let mut assertion = assertion(Decision::Continue);
    assertion.generator_id = "provider:chat-completions:model#aaaaaaaaaaaa".to_string();
    let chat_key = assertion.cache_key();
    assertion.generator_id = "provider:gemini:model#aaaaaaaaaaaa".to_string();
    assert_ne!(
        chat_key,
        assertion.cache_key(),
        "a different typed API must not share a model cache entry"
    );
    assertion.generator_id = "provider:chat-completions:model#bbbbbbbbbbbb".to_string();
    assert_ne!(
        chat_key,
        assertion.cache_key(),
        "a different endpoint/model/effort identity must not share a cache entry"
    );
}
