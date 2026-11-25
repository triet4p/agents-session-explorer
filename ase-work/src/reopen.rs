//! Reopen Turn/Episode evidence from exact source bytes.
//!
//! Reopen never trusts a cached projection or a nearest line. It checks the
//! pinned index generation and file digest, re-reads every evidence span,
//! re-runs the versioned selector and recomputes the whole journal projection.
//! Any mismatch is reported as `stale`; it is never silently repaired.

use ase_core::SpanRef;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::model::*;
use crate::project::{project_bytes, ProjectOptions};
use crate::util::{fnv64_hex, sha256_hex};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReopenReport {
    pub status: String,
    pub generation: String,
    pub file_id: u32,
    pub source_id: String,
    pub evidence_checked: u32,
    pub evidence_verified: u32,
    pub projection_recomputed: bool,
    pub runtime_checked: u32,
    pub runtime_verified: u32,
    pub errors: Vec<String>,
}

#[derive(Deserialize)]
struct EvidenceLine<'a> {
    #[serde(rename = "type")]
    ty: Option<&'a str>,
    id: Option<&'a str>,
    attribution: Option<&'a str>,
    message: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct EvidenceMessage<'a> {
    role: Option<&'a str>,
    usage: Option<&'a RawValue>,
}

fn selector_kind(selector: &str) -> Option<&str> {
    selector
        .strip_prefix(EVIDENCE_VERSION)
        .and_then(|s| s.strip_prefix('|'))
}

fn check_evidence(proj: &JournalProjection, bytes: &[u8], ev: &EvidenceRef) -> Vec<String> {
    let mut errors = Vec::new();
    if ev.generation != proj.generation {
        errors.push(format!("generation_mismatch:{}", ev.selector));
    }
    let start = ev.span.off as usize;
    let end = start.saturating_add(ev.span.len as usize);
    if end > bytes.len() {
        errors.push(format!("span_out_of_bounds:{}", ev.selector));
        return errors;
    }
    let line = &bytes[start..end];
    if sha256_hex(line) != ev.line_digest {
        errors.push(format!("digest_mismatch:line:{}", ev.selector));
    }
    if fnv64_hex(line) != ev.index_checksum {
        errors.push(format!("checksum_mismatch:index:{}", ev.selector));
    }
    let Some(kind) = selector_kind(&ev.selector) else {
        errors.push(format!("selector_version_unknown:{}", ev.selector));
        return errors;
    };
    let Ok(record) = serde_json::from_slice::<EvidenceLine<'_>>(line) else {
        errors.push(format!("selector_parse_error:{}", ev.selector));
        return errors;
    };
    let message = record
        .message
        .and_then(|raw| serde_json::from_str::<EvidenceMessage<'_>>(raw.get()).ok());
    let is_user = record.ty == Some("message") && message.as_ref().and_then(|m| m.role) == Some("user");
    let is_turn = record.ty == Some("message")
        && message.as_ref().and_then(|m| m.role) == Some("assistant")
        && message
            .as_ref()
            .and_then(|m| m.usage)
            .is_some_and(|u| u.get().trim_start().starts_with('{'));
    let is_custom_user = record.ty == Some("custom_message") && record.attribution == Some("user");
    let ok = match kind {
        SEL_TURN => is_turn,
        SEL_USER => is_user,
        SEL_CUSTOM_USER => is_custom_user,
        SEL_LEAF => record.id == proj.active_leaf.entry_id.as_deref(),
        _ => false,
    };
    if !ok {
        errors.push(format!("selector_mismatch:{kind}"));
    }
    errors
}

fn key_diffs(expected: &JournalProjection, actual: &JournalProjection) -> Vec<String> {
    let mut out = Vec::new();
    if expected.active_leaf.entry_id != actual.active_leaf.entry_id {
        out.push("leaf_entry_id".to_string());
    }
    if expected.counts.turns_active_leaf != actual.counts.turns_active_leaf {
        out.push("turns_active_leaf".to_string());
    }
    if expected.counts.turns_all_branches != actual.counts.turns_all_branches {
        out.push("turns_all_branches".to_string());
    }
    if expected.counts.episodes_primary_count != actual.counts.episodes_primary_count {
        out.push("episodes_primary_count".to_string());
    }
    if expected.counts.episodes_wide_count != actual.counts.episodes_wide_count {
        out.push("episodes_wide_count".to_string());
    }
    out
}

/// Recomputes one journal projection and verifies every embedded evidence
/// reference. `current_generation` identifies the index snapshot in use.
pub fn reopen_bytes(
    proj: &JournalProjection,
    bytes: &[u8],
    current_generation: &str,
) -> ReopenReport {
    let mut report = ReopenReport {
        status: "verified".to_string(),
        generation: current_generation.to_string(),
        file_id: proj.file_id,
        source_id: proj.source_id.clone(),
        ..ReopenReport::default()
    };
    if proj.generation != current_generation {
        report.errors.push(format!(
            "generation_mismatch:projection={} current={current_generation}",
            proj.generation
        ));
        report.status = "generation_mismatch".to_string();
        return report;
    }
    if sha256_hex(bytes) != proj.sha256 {
        report.errors.push("digest_mismatch:file".to_string());
        report.status = "stale".to_string();
        return report;
    }

    let mut evidence: Vec<&EvidenceRef> = Vec::new();
    if let Some(ev) = &proj.active_leaf.evidence {
        evidence.push(ev);
    }
    for turn in &proj.turns {
        evidence.push(&turn.evidence);
    }
    for episode in proj.episodes_primary.iter().chain(proj.episodes_wide.iter()) {
        if let Some(ev) = &episode.evidence {
            evidence.push(ev);
        }
    }
    report.evidence_checked = evidence.len() as u32;
    for ev in evidence {
        let errors = check_evidence(proj, bytes, ev);
        if errors.is_empty() {
            report.evidence_verified += 1;
        } else {
            report.errors.extend(errors);
        }
    }

    for runtime in &proj.runtime {
        let errors = check_evidence(proj, bytes, &runtime.evidence);
        report.runtime_checked += 1;
        if errors.is_empty() {
            report.runtime_verified += 1;
        } else {
            report.errors.extend(errors);
        }
    }

    let opts = ProjectOptions {
        file_id: proj.file_id,
        path: &proj.path,
        source_id: &proj.source_id,
        generation: &proj.generation,
        sha256: &proj.sha256,
        event_base: proj.event_base,
        explicit_leaf: proj.requested_leaf.as_deref(),
        window: proj.window,
    };
    match project_bytes(&opts, bytes) {
        Ok(recomputed) => {
            report.projection_recomputed = true;
            // Projection equality stays stable for projections written before the
            // additive runtime field existed; runtime facts are checked separately.
            let mut expected = proj.clone();
            expected.runtime.clear();
            let mut recalculated = recomputed.clone();
            recalculated.runtime.clear();
            if expected != recalculated {
                for diff in key_diffs(proj, &recomputed) {
                    report.errors.push(format!("projection_mismatch:{diff}"));
                }
                if report.errors.is_empty() {
                    report.errors.push("projection_mismatch:fields".to_string());
                }
            }
            if proj.runtime != recomputed.runtime {
                report.errors.push(format!(
                    "runtime_mismatch:expected={} actual={}",
                    proj.runtime.len(),
                    recomputed.runtime.len()
                ));
            }
        }
        Err(e) => {
            report.errors.push(format!("projection_error:{e}"));
        }
    }

    if !report.errors.is_empty() {
        report.status = "stale".to_string();
    }
    report
}

/// Convenience for callers that already read one evidence span.
pub fn evidence_span(ev: &EvidenceRef) -> SpanRef {
    SpanRef { file: ev.span.file, off: ev.span.off, len: ev.span.len }
}
