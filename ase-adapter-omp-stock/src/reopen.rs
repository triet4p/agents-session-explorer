//! Evidence reopen path for adapter claims.
//!
//! Reopen deliberately re-reads source bytes and re-runs the versioned selector.
//! It never trusts stored prose, a cache, a nearest line or a different
//! generation.

use ase_core::Result;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::extract::extract_task_tokens;
use crate::model::*;

fn selector_token_of(selector: &str) -> Option<&str> {
    selector
        .strip_prefix(SELECTOR_VERSION)
        .and_then(|s| s.strip_prefix("|sidecar.task_token:"))
}

fn check_path_selector(
    view: &dyn StockView,
    ev: &EvidenceRef,
) -> Vec<String> {
    let mut errors = Vec::new();
    let found = view.files().iter().find(|f| path_eq(&f.path, &ev.path));
    match found {
        Some(file) => {
            let normalized = path_key(&file.path);
            let expect = sha256_hex(normalized.as_bytes());
            if ev.block_digest.as_deref() != Some(expect.as_str()) {
                errors.push("block_digest_mismatch:path".to_string());
            }
        }
        None => errors.push("source_missing".to_string()),
    }
    errors
}

fn check_sidecar(view: &dyn StockView, ev: &EvidenceRef) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(token) = selector_token_of(&ev.selector) else {
        return vec!["selector_version_unknown:sidecar".to_string()];
    };
    let bytes = match view.sidecar_bytes(&ev.path) {
        Ok(Some(b)) => b,
        Ok(None) => return vec!["source_missing".to_string()],
        Err(_) => return vec!["source_missing".to_string()],
    };
    if let Some(manifest) = &ev.manifest_digest {
        if *manifest != sha256_hex(&bytes) {
            errors.push("digest_mismatch:sidecar_manifest".to_string());
        }
    }
    let start = ev.span.off as usize;
    let end = start.saturating_add(ev.span.len as usize);
    if end > bytes.len() {
        errors.push("span_out_of_bounds".to_string());
        return errors;
    }
    let line = &bytes[start..end];
    if ev.line_digest != sha256_hex(line) {
        errors.push("digest_mismatch:line".to_string());
    }
    let text = String::from_utf8_lossy(line);
    let present = extract_task_tokens(&text).iter().any(|t| t.token == token)
        || crate::extract::extract_task_id_markers(&text)
            .iter()
            .any(|t| t.token == token);
    if !present {
        errors.push("block_digest_mismatch:token".to_string());
    } else if ev.block_digest.as_deref() != Some(sha256_hex(token.as_bytes()).as_str()) {
        errors.push("block_digest_mismatch:token_digest".to_string());
    }
    errors
}

fn check_journal(view: &dyn StockView, ev: &EvidenceRef) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(file) = view.files().iter().find(|f| path_eq(&f.path, &ev.path)) else {
        return vec!["source_missing".to_string()];
    };
    if ev.coordinate != file.encoding.coordinate() {
        errors.push("layer_unavailable:coordinate".to_string());
    }
    let span = ev.span.to_span();
    let event = match view.event_at(span) {
        Ok(Some(e)) => e,
        Ok(None) => {
            errors.push("span_out_of_bounds".to_string());
            return errors;
        }
        Err(_) => {
            errors.push("span_out_of_bounds".to_string());
            return errors;
        }
    };
    if let Some(checksum) = &ev.span_checksum {
        if *checksum != format!("{:016x}", event.hash) {
            errors.push("checksum_mismatch".to_string());
        }
    }
    let bytes = match view.span_bytes(span) {
        Ok(b) => b,
        Err(_) => {
            errors.push("span_out_of_bounds".to_string());
            return errors;
        }
    };
    if bytes.len() != span.len as usize {
        errors.push("span_out_of_bounds".to_string());
    }
    if ev.line_digest != sha256_hex(&bytes) {
        errors.push("digest_mismatch:line".to_string());
    }
    if let Some(manifest) = &ev.manifest_digest {
        if let Some(current) = &file.manifest_digest {
            if manifest != current {
                errors.push("digest_mismatch:manifest".to_string());
            }
        }
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            errors.push("digest_mismatch:json".to_string());
            return errors;
        }
    };
    match derive_block(&value, &ev.selector) {
        Ok(block) => {
            if ev.block_digest.as_deref() != Some(sha256_hex(&block).as_str()) {
                errors.push("block_digest_mismatch".to_string());
            }
        }
        Err(e) => {
            let code = e.0.split(':').next().unwrap_or("selector_version_unknown");
            errors.push(format!("{code}:selector"));
        }
    }
    errors
}

pub fn reopen(
    view: &dyn StockView,
    claims: &ClaimsFile,
    claim_filter: Option<&str>,
) -> Result<ReopenReport> {
    let active_generation = view.generation().to_string();
    let mut errors = Vec::new();
    if claims.generation != active_generation {
        errors.push(ScanError {
            code: "generation_mismatch".to_string(),
            path: None,
            detail: format!(
                "claims generation {} is not the active generation {}",
                claims.generation, active_generation
            ),
        });
        let claims_out = claims
            .claims
            .iter()
            .filter(|c| claim_filter.map(|f| c.claim_id == f).unwrap_or(true))
            .map(|c| ReopenedClaim {
                claim_id: c.claim_id.clone(),
                layer: c.layer.clone(),
                original_state: c.state.clone(),
                assessed_state: STATE_UNAVAILABLE.to_string(),
                evidence: Vec::new(),
            })
            .collect::<Vec<_>>();
        return Ok(ReopenReport {
            adapter: ADAPTER_NAME.to_string(),
            generation: claims.generation.clone(),
            active_generation,
            status: "generation_mismatch".to_string(),
            claims: claims_out,
            counts: BTreeMap::from([("unavailable".to_string(), 1)]),
            errors,
        });
    }

    let mut out = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut verified_claims = 0usize;
    let mut stale_claims = 0usize;
    for claim in claims
        .claims
        .iter()
        .filter(|c| claim_filter.map(|f| c.claim_id == f).unwrap_or(true))
    {
        let mut checks = Vec::new();
        let mut any_error = false;
        for (i, ev) in claim.evidence.iter().enumerate() {
            let errors = if ev.selector.starts_with(&format!("{SELECTOR_VERSION}|path.")) {
                check_path_selector(view, ev)
            } else if ev.selector.starts_with(&format!("{SELECTOR_VERSION}|sidecar.")) {
                check_sidecar(view, ev)
            } else {
                check_journal(view, ev)
            };
            let ok = errors.is_empty();
            if !ok {
                any_error = true;
            }
            checks.push(EvidenceCheck {
                index: i,
                path: ev.path.clone(),
                selector: ev.selector.clone(),
                status: if ok { "verified".to_string() } else { "stale".to_string() },
                errors,
            });
        }
        let assessed = if any_error {
            stale_claims += 1;
            "stale".to_string()
        } else {
            verified_claims += 1;
            claim.state.clone()
        };
        *counts.entry(assessed.clone()).or_default() += 1;
        out.push(ReopenedClaim {
            claim_id: claim.claim_id.clone(),
            layer: claim.layer.clone(),
            original_state: claim.state.clone(),
            assessed_state: assessed,
            evidence: checks,
        });
    }
    let status = if out.is_empty() {
        "no_claims"
    } else if stale_claims > 0 {
        "stale"
    } else {
        "verified"
    };
    counts.insert("claims_assessed".to_string(), out.len());
    counts.insert("claims_verified".to_string(), verified_claims);
    counts.insert("claims_stale".to_string(), stale_claims);
    Ok(ReopenReport {
        adapter: ADAPTER_NAME.to_string(),
        generation: claims.generation.clone(),
        active_generation,
        status: status.to_string(),
        claims: out,
        counts,
        errors,
    })
}
