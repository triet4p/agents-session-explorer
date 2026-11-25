//! P4 bundle export per `artifacts/sprint-7/ASE-S7-T1/contract-a1.md` §3 and
//! `docs/sprint-plans/sprint-7.md:17`.
//!
//! The exporter runs the pinned motif query locally, selects representatives
//! deterministically, redacts result text with `ase-p4-redact-v1`, writes only
//! path-free shareable files, and keeps the alias-to-source map outside the
//! bundle. Nothing is published until the freshly written staging bundle has
//! been reopened and verified by `p4_verify::verify`. No model, network,
//! verdict, catalog or gate measurement is involved.

use crate::p4_verify::{
    cap_signature, canonical, derive_signature, derive_tool, occurrence_identity_hex,
    redaction_lines, render_excerpt, render_evidence_view, render_questions_view,
    result_for_call, result_status_of, row_index_of, row_spans, selector_of, value_digest, verify,
    BUNDLE_VERSION, EVIDENCE_RENDER, MOTIF_SCHEMA, QUESTIONS_RENDER, REDACTION_POLICY,
    REPRESENTATIVE_POLICY, SELECTOR_VERSION,
};
use crate::{motif_target, require_publication, MotifTarget};
use ase_core::{err, RawStore, Result};
use ase_ingest::util::fnv64;
use ase_index::{sha256_hex, FsRawStore, Index};
use ase_motif::{align, AlignConfig, Motif, MotifConfig, MotifReport, Occurrence};
use ase_postings::Publication;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_REPRESENTATIVES: usize = 5;

const MAX_BUNDLE_BYTES: usize = 32_768;
const MAX_CALL_BYTES: usize = 512;
const MAX_OCCURRENCE_BYTES: usize = 4_096;
const MAX_RESULT_LINES: usize = 3;
const QUESTION_TEXTS: [(&str, &str); 3] = [
    ("Q1", "Is this sequence a fixed ritual or a variable procedure?"),
    ("Q2", "Which calls are essential and which are incidental?"),
    (
        "Q3",
        "Should this repetition be codified as a skill or template under the supplied catalog?",
    ),
];
const VALUE_FLAGS: [&str; 7] = [
    "--out",
    "--local-map",
    "--index",
    "--omp-root",
    "--codex-root",
    "--address",
    "--addresses",
];

struct PreparedOccurrence {
    occurrence_id: String,
    order_key: (u64, u64, u32),
    call_count: u64,
    divergence: u32,
    failure: bool,
    result_status: String,
    source_alias: String,
    evidence: Vec<Value>,
    identity: String,
    private: Vec<String>,
}

struct Assembled {
    bundle: Value,
    motif: Value,
    files: Vec<(String, Vec<u8>)>,
    total_bytes: usize,
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opts(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            if let Some(value) = args.get(i + 1) {
                out.push(value.clone());
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn positional_target(args: &[String]) -> Result<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if VALUE_FLAGS.contains(&arg.as_str()) {
            i += 2;
            continue;
        }
        if arg.starts_with("--") {
            i += 1;
            continue;
        }
        return Ok(arg.clone());
    }
    Err(err("export needs one motif:<id> positional argument"))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        normalize(&std::env::current_dir().unwrap_or_default().join(path))
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_string_pretty(value).map_err(|e| err(format!("JSON render: {e}")))?;
    fs::write(path, body).map_err(|e| err(format!("write failed: {e}")))
}

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| err(format!("read failed: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| err(format!("JSON parse: {e}")))
}

fn digest_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| err(format!("read failed: {e}")))?;
    Ok(sha256_hex(&bytes))
}

fn pinned_config() -> MotifConfig {
    let mut config = MotifConfig::default();
    config.family.k_min = 3;
    config.family.k_max = 5;
    config.family.tau = 0.5;
    config.max_gap = 2;
    config.band = 16;
    config.max_substitutions = 2;
    config.null_permutations = 64;
    config.null_seed = 11_400_714_819_323_198_485;
    config
}

fn query_config(config: &MotifConfig) -> Value {
    json!({
        "family": {
            "k_min": config.family.k_min,
            "k_max": config.family.k_max,
            "tau": config.family.tau
        },
        "max_gap": config.max_gap,
        "band": config.band,
        "max_substitutions": config.max_substitutions,
        "min_tasks": config.min_tasks,
        "null_permutations": config.null_permutations,
        "null_seed": config.null_seed.to_string()
    })
}

fn collect_addresses(args: &[String]) -> Result<Vec<MotifTarget>> {
    let mut raw = opts(args, "--address");
    for path in opts(args, "--addresses") {
        let value = read_json(Path::new(&path))?;
        let list = value
            .get("addresses")
            .and_then(Value::as_array)
            .or_else(|| value.as_array())
            .ok_or_else(|| err("--addresses file needs an addresses array"))?;
        for row in list {
            let address = row
                .as_str()
                .ok_or_else(|| err("--addresses entries must be strings"))?;
            raw.push(address.to_string());
        }
    }
    raw.sort();
    raw.dedup();
    if raw.is_empty() {
        return Err(err("export needs --address PROJECT|TASK or --addresses FILE"));
    }
    let mut targets = Vec::with_capacity(raw.len());
    for address in raw {
        let target = motif_target(&address)?;
        if target.project.is_none() {
            return Err(err(format!(
                "export requires PROJECT|TASK addresses; got {address}"
            )));
        }
        targets.push(target);
    }
    Ok(targets)
}

fn occurrence_value(occurrence: &Occurrence) -> Value {
    json!({
        "task": occurrence.task,
        "attempt": occurrence.attempt,
        "dispatch_id": occurrence.dispatch_id,
        "text_hash": occurrence.text_hash,
        "calls": occurrence.calls.iter().map(|call| json!({
            "file": call.evidence.span.file,
            "event": call.evidence.event,
            "offset": call.evidence.span.off,
            "length": call.evidence.span.len,
            "span": format!("span:{}:{}:{}", call.evidence.span.file, call.evidence.span.off, call.evidence.span.len),
            "selector": call.evidence.selector,
            "block_hash": call.evidence.block_hash,
        })).collect::<Vec<Value>>(),
    })
}

fn occurrence_id(value: &Value) -> Result<String> {
    let calls = value
        .get("calls")
        .and_then(Value::as_array)
        .ok_or_else(|| err("occurrence has no calls"))?;
    let mut keys = Vec::with_capacity(calls.len());
    for call in calls {
        let span = call.get("span").and_then(Value::as_str).unwrap_or("");
        let event = call
            .get("event")
            .and_then(Value::as_u64)
            .ok_or_else(|| err("occurrence call event missing"))?;
        let parts: Vec<&str> = span.split(':').collect();
        if parts.len() != 4 || parts[0] != "span" {
            return Err(err("occurrence call span malformed"));
        }
        keys.push(json!({
            "file": parts[1].parse::<u64>().map_err(|_| err("occurrence span file malformed"))?,
            "event": event,
            "offset": parts[2].parse::<u64>().map_err(|_| err("occurrence span offset malformed"))?,
            "length": parts[3].parse::<u64>().map_err(|_| err("occurrence span length malformed"))?,
            "selector": call.get("selector").and_then(Value::as_str).unwrap_or(""),
            "block_hash": call.get("block_hash").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    let identity = json!({
        "task": value.get("task").and_then(Value::as_str).unwrap_or(""),
        "attempt": value.get("attempt").and_then(Value::as_str).unwrap_or(""),
        "dispatch_id": value.get("dispatch_id").and_then(Value::as_str).unwrap_or(""),
        "text_hash": value.get("text_hash").and_then(Value::as_str).unwrap_or(""),
        "calls": keys,
    });
    Ok(format!("occ-{}", &value_digest(&identity)[..16]))
}

fn source_alias(file_id: u64, sha: &str) -> String {
    let digest = value_digest(&json!({ "file_id": file_id, "sha256": sha }));
    format!("src-{}", &digest[..16])
}

fn parse_span(label: &str) -> Result<(u32, u64, u32)> {
    let parts: Vec<&str> = label.split(':').collect();
    if parts.len() != 4 || parts[0] != "span" {
        return Err(err("call span label malformed"));
    }
    Ok((
        parts[1].parse().map_err(|_| err("span file malformed"))?,
        parts[2].parse().map_err(|_| err("span offset malformed"))?,
        parts[3].parse().map_err(|_| err("span length malformed"))?,
    ))
}

fn legacy_selector(selector: &str) -> Option<(usize, String)> {
    let parts: Vec<&str> = selector.split('|').collect();
    if parts.len() != 3 || parts[0] != SELECTOR_VERSION || parts[2] != "toolCall" {
        return None;
    }
    let content = parts[1]
        .strip_prefix("message.content[")?
        .strip_suffix(']')?
        .parse::<usize>()
        .ok()?;
    Some((content, parts[2].to_string()))
}

fn parsed_rows(bytes: &[u8]) -> Vec<Option<Value>> {
    row_spans(bytes)
        .iter()
        .map(|(off, len)| serde_json::from_slice(&bytes[*off..*off + *len]).ok())
        .collect()
}

fn error_nonempty(value: Option<&Value>) -> bool {
    match value {
        None => false,
        Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Number(_)) | Some(Value::Bool(true)) => true,
    }
}

fn result_failure(row: &Value, block: Option<&Value>) -> bool {
    let message = row.get("message").unwrap_or(&Value::Null);
    message.get("isError").and_then(Value::as_bool) == Some(true)
        || error_nonempty(message.get("error"))
        || block
            .map(|block| {
                block.get("isError").and_then(Value::as_bool) == Some(true)
                    || error_nonempty(block.get("error"))
                    || block.get("status").and_then(Value::as_str) == Some("error")
            })
            .unwrap_or(false)
}

fn build_evidence(
    occurrence: &Occurrence,
    occurrence_id: &str,
    idx: &Index,
    store: &FsRawStore,
    published: &Publication,
    bytes_by_file: &mut HashMap<u64, Arc<[u8]>>,
) -> Result<(Vec<Value>, bool, String, Vec<String>)> {
    let mut evidence = Vec::with_capacity(occurrence.calls.len());
    let mut any_failure = false;
    let mut statuses = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut private = Vec::new();
    private.push(occurrence.task.clone());
    private.push(occurrence.attempt.clone());
    private.push(occurrence.dispatch_id.clone());

    for (ordinal, call) in occurrence.calls.iter().enumerate() {
        if call.evidence.generation != published.generation {
            return Err(err("export call generation differs from the active publication"));
        }
        let (file_id, off, len) = parse_span(&format!("span:{}:{}:{}", call.evidence.span.file, call.evidence.span.off, call.evidence.span.len))?;
        if file_id != call.evidence.file {
            return Err(err("call span file differs from its evidence file"));
        }
        files.insert(file_id);
        let bytes = match bytes_by_file.get(&(file_id as u64)) {
            Some(bytes) => Arc::clone(bytes),
            None => {
                let bytes = store.bytes(file_id)?;
                let meta = published
                    .manifest
                    .files
                    .iter()
                    .find(|row| row.id == file_id)
                    .ok_or_else(|| err("call file absent from publication manifest"))?;
                if sha256_hex(&bytes) != meta.digest {
                    return Err(err("call file digest differs from the frozen publication"));
                }
                bytes_by_file.insert(file_id as u64, Arc::clone(&bytes));
                bytes
            }
        };
        if off as usize > bytes.len() || len as usize > bytes.len() - off as usize {
            return Err(err("call span outside the reopened source"));
        }
        let event = idx.event(call.evidence.event)?;
        let span = event.span();
        if span != call.evidence.span || fnv64(&bytes[off as usize..off as usize + len as usize]) != event.hash() {
            return Err(err("call span or checksum differs from the pinned address"));
        }
        let spans = row_spans(&bytes);
        let row = row_index_of(off as usize, &bytes);
        if spans.get(row).copied() != Some((off as usize, len as usize)) {
            return Err(err("call offset is not a pinned row boundary"));
        }
        let rows = parsed_rows(&bytes);
        let (content, kind) = legacy_selector(&call.evidence.selector)
            .ok_or_else(|| err("call selector is not the pinned toolCall form"))?;
        let row_value = rows
            .get(row)
            .and_then(Option::as_ref)
            .ok_or_else(|| err("call row is not JSON"))?;
        let block = row_value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.get(content))
            .ok_or_else(|| err("call selector content index absent"))?
            .clone();
        if block.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
            return Err(err("call selector block type differs"));
        }
        let selector = selector_of(&bytes, row, content, &kind);
        let tool = derive_tool(&block);
        let full_signature = derive_signature(&block);
        let signature = cap_signature(&full_signature);
        let block_sha256 = format!("sha256:{}", sha256_hex(&canonical(&block)));
        let call_id = block
            .get("toolCallId")
            .or_else(|| block.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| err("reopened call has no tool call id"))?;

        let found = result_for_call(&rows, row, call_id);
        let (result, status, result_text, failure) = match found {
            None => (json!({ "status": "absent" }), "absent", String::new(), false),
            Some((result_row, result_content, result_block)) => {
                let status = result_status_of(&result_block);
                let result_kind = result_block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("toolResult");
                let result_selector = selector_of(&bytes, result_row, result_content, result_kind);
                let result_sha256 = format!("sha256:{}", sha256_hex(&canonical(&result_block)));
                let text = if status == "present" {
                    result_block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                };
                let failure = result_failure(&rows[result_row].as_ref().cloned().unwrap_or(Value::Null), Some(&result_block));
                (
                    json!({
                        "status": status,
                        "selector": result_selector,
                        "block_sha256": result_sha256,
                    }),
                    status,
                    text,
                    failure,
                )
            }
        };
        statuses.insert(status.to_string());
        any_failure |= failure;

        let mut lines = redaction_lines(&result_text);
        let mut excerpt = render_excerpt(ordinal as u64, &tool, &signature, status, &lines);
        while excerpt.len() > MAX_CALL_BYTES && lines.len() > 1 {
            lines.pop();
            excerpt = render_excerpt(ordinal as u64, &tool, &signature, status, &lines);
        }
        if excerpt.len() > MAX_CALL_BYTES {
            return Err(err("redacted call exceeds the 512-byte citation limit"));
        }
        let evidence_id = {
            let digest = value_digest(&json!({
                "occurrence_id": occurrence_id,
                "call_ordinal": ordinal,
                "selector": selector,
                "block_sha256": block_sha256,
            }));
            format!("ev-{}", &digest[..16])
        };
        let excerpt_sha256 = format!("sha256:{}", sha256_hex(excerpt.as_bytes()));
        evidence.push(json!({
            "evidence_id": evidence_id,
            "call_ordinal": ordinal,
            "call": {
                "file_id": file_id,
                "selector": selector,
                "block_sha256": block_sha256,
                "tool": tool,
                "signature": signature,
            },
            "result": result,
            "result_lines": lines.len(),
            "excerpt": excerpt,
            "excerpt_sha256": excerpt_sha256,
        }));
    }

    if evidence.is_empty() {
        return Err(err("occurrence has no reopened call evidence"));
    }
    if files.len() != 1 {
        return Err(err("one occurrence must use one private source alias"));
    }
    let result_status = if any_failure {
        "failure".to_string()
    } else if statuses.len() == 1 {
        statuses.iter().next().cloned().unwrap_or_else(|| "absent".to_string())
    } else {
        "mixed".to_string()
    };
    Ok((evidence, any_failure, result_status, private))
}

fn divergence(motif: &Motif, signatures: &[String], band: u32) -> u32 {
    align(&motif.signatures, signatures, &AlignConfig { band })
        .map(|a| a.changes())
        .unwrap_or(0)
}

fn prepare_occurrences(
    motif: &Motif,
    config: &MotifConfig,
    idx: &Index,
    store: &FsRawStore,
    published: &Publication,
    bundle: &Value,
    motif_skeleton: &Value,
) -> Result<Vec<PreparedOccurrence>> {
    let mut bytes_by_file = HashMap::new();
    let mut out = Vec::new();
    for occurrence in &motif.occurrences {
        if occurrence.duplicate_of.is_some() {
            continue;
        }
        let value = occurrence_value(occurrence);
        let id = occurrence_id(&value)?;
        let (evidence, failure, result_status, mut private) = build_evidence(
            occurrence,
            &id,
            idx,
            store,
            published,
            &mut bytes_by_file,
        )?;
        let first = occurrence
            .calls
            .first()
            .ok_or_else(|| err("occurrence has no first call"))?;
        let order_key = (
            first.evidence.span.file as u64,
            first.evidence.span.off,
            first.evidence.event,
        );
        let mut reopened = Vec::with_capacity(occurrence.calls.len());
        for call in &occurrence.calls {
            reopened.push(call.signature.clone());
            private.push(call.evidence.selector.clone());
        }
        let mut occurrence_record = json!({
            "occurrence_id": id,
            "source_alias": "",
            "evidence": evidence,
        });
        let alias = {
            let file_id = first.evidence.span.file as u64;
            let meta = published
                .manifest
                .files
                .iter()
                .find(|row| row.id as u64 == file_id)
                .ok_or_else(|| err("occurrence file absent from publication manifest"))?;
            source_alias(file_id, &format!("sha256:{}", meta.digest))
        };
        occurrence_record["source_alias"] = json!(alias);
        let identity = occurrence_identity_hex(bundle, motif_skeleton, &occurrence_record)
            .ok_or_else(|| err("input identity could not be recomputed"))?;
        out.push(PreparedOccurrence {
            occurrence_id: id,
            order_key,
            call_count: occurrence.calls.len() as u64,
            divergence: divergence(motif, &reopened, config.band),
            failure,
            result_status,
            source_alias: alias,
            evidence: occurrence_record
                .get("evidence")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            identity,
            private,
        });
    }
    Ok(out)
}

fn build_query_digest(report: &MotifReport, config: &MotifConfig) -> Result<String> {
    let mut motifs = Vec::with_capacity(report.motifs.len());
    for motif in &report.motifs {
        let mut occurrences = Vec::new();
        for occurrence in &motif.occurrences {
            if occurrence.duplicate_of.is_none() {
                occurrences.push(occurrence_id(&occurrence_value(occurrence))?);
            }
        }
        occurrences.sort();
        let mut relaxed = BTreeMap::new();
        for (gap, count) in motif.support_relaxed.iter().enumerate() {
            relaxed.insert(gap.to_string(), *count);
        }
        motifs.push(json!({
            "family_id": motif.family_id,
            "motif_id": motif.id,
            "occurrences": occurrences,
            "signatures": motif.signatures,
            "support": { "exact": motif.support_exact, "relaxed": relaxed },
        }));
    }
    motifs.sort_by(|a, b| a["motif_id"].as_str().cmp(&b["motif_id"].as_str()));
    Ok(value_digest(&json!({
        "config": query_config(config),
        "dedupe_rule": { "name": ase_motif::TEXT_HASH_RULE_NAME, "version": ase_motif::TEXT_HASH_RULE_VERSION },
        "motifs": motifs,
        "rule": { "name": report.rule_name, "version": report.rule_version },
    })))
}

fn build_bundle_base(
    published: &Publication,
    report: &MotifReport,
    config: &MotifConfig,
    binary_sha256: &str,
) -> Result<Value> {
    let core = &published.manifest.core_digest;
    if core.len() < 16 {
        return Err(err("publication core digest is too short"));
    }
    let snapshot_id = format!("snap-{}", &core[..16]);
    let mut files = published.manifest.files.clone();
    files.sort_by_key(|row| row.id);
    let file_rows: Vec<Value> = files
        .iter()
        .map(|row| {
            json!({
                "file_id": row.id,
                "sha256": format!("sha256:{}", row.digest),
                "size": row.size,
            })
        })
        .collect();
    let total_bytes: u64 = files.iter().map(|row| row.size).sum();
    let query_result_digest = build_query_digest(report, config)?;
    Ok(json!({
        "schema": BUNDLE_VERSION,
        "bundle_version": BUNDLE_VERSION,
        "created_by": "ase-export",
        "snapshot": {
            "core_digest": core,
            "file_count": files.len(),
            "files": file_rows,
            "provider": "omp",
            "snapshot_id": snapshot_id,
            "total_bytes": total_bytes,
        },
        "query": {
            "config": query_config(config),
            "dedupe_rule": {
                "name": ase_motif::TEXT_HASH_RULE_NAME,
                "version": ase_motif::TEXT_HASH_RULE_VERSION,
            },
            "query_result_digest": query_result_digest,
            "rule": { "name": report.rule_name, "version": report.rule_version },
        },
        "engine": {
            "binary_sha256": format!("sha256:{binary_sha256}"),
            "evidence_render": EVIDENCE_RENDER,
            "name": "ase",
            "questions_render": QUESTIONS_RENDER,
            "redaction_policy": REDACTION_POLICY,
            "rule_versions": {
                "callsig": "callsig-v1",
                "motif_alignment": ase_motif::MOTIF_RULE_VERSION,
                "motif_dedupe": ase_motif::TEXT_HASH_RULE_VERSION,
                "projection": SELECTOR_VERSION,
            },
        },
        "redaction": {
            "allowed_fields": [
                "tool name",
                "callsig-v1 argument signature",
                "first N result text lines after redaction",
            ],
            "forbidden": [
                "real paths",
                "alias-to-source mapping",
                "raw tool output",
                "thinking or system text",
                "verdicts",
                "session identifiers",
            ],
            "markers": [
                "«url»", "«path»", "«email»", "«secret»", "«token»", "«absent»", "«nontext»", "«empty»",
            ],
            "policy": REDACTION_POLICY,
        },
        "citation_limits": {
            "max_bundle_bytes": MAX_BUNDLE_BYTES,
            "max_excerpt_bytes_per_call": MAX_CALL_BYTES,
            "max_excerpt_bytes_per_occurrence": MAX_OCCURRENCE_BYTES,
            "max_occurrences_per_motif": 5,
            "max_result_line_bytes": 160,
            "max_result_lines": MAX_RESULT_LINES,
        },
        "representative_policy": REPRESENTATIVE_POLICY,
    }))
}

fn build_motif_skeleton(motif: &Motif, reference: &str) -> Value {
    let mut relaxed = BTreeMap::new();
    for (gap, count) in motif.support_relaxed.iter().enumerate() {
        relaxed.insert(gap.to_string(), *count);
    }
    json!({
        "schema": MOTIF_SCHEMA,
        "motif_id": motif.id,
        "family_id": motif.family_id,
        "signatures": motif.signatures,
        "support": { "exact": motif.support_exact, "relaxed": relaxed },
        "denominator": {
            "tasks_total": motif.denominator.tasks_total,
            "tasks_in_family": motif.denominator.tasks_in_family,
            "tasks_with_occurrence": motif.denominator.tasks_with_occurrence,
            "sequences_in_family": motif.denominator.sequences_in_family,
            "sequences_with_occurrence": motif.denominator.sequences_with_occurrence,
            "sequences_deduplicated": motif.denominator.sequences_deduplicated,
            "excluded_count": motif.denominator.excluded.len(),
        },
        "reference": { "occurrence_id": reference },
        "representative_policy": REPRESENTATIVE_POLICY,
        "questions": QUESTION_TEXTS.iter().map(|(id, text)| json!({
            "question_id": id,
            "text": text,
        })).collect::<Vec<Value>>(),
        "representatives": [],
        "occurrences_total": 0,
        "representatives_selected": 0,
        "representatives_shortfall": false,
    })
}

fn slot_rank(label: &str) -> u64 {
    match label {
        "earliest" => 0,
        "latest" => 1,
        "most_divergent" => 2,
        "failure" => 3,
        other => other
            .strip_prefix("fill:")
            .and_then(|n| n.parse::<u64>().ok())
            .map(|n| 4 + n)
            .unwrap_or(1000),
    }
}

fn select_representatives(prepared: &[PreparedOccurrence]) -> Vec<(usize, String)> {
    let mut assigned: HashMap<usize, String> = HashMap::new();
    let mut by_key: Vec<usize> = (0..prepared.len()).collect();
    by_key.sort_by_key(|&i| prepared[i].order_key);
    if let Some(&first) = by_key.first() {
        assigned.insert(first, "earliest".to_string());
    }
    if let Some(&last) = by_key.last() {
        assigned.entry(last).or_insert_with(|| "latest".to_string());
    }
    let mut by_divergence: Vec<usize> = (0..prepared.len()).collect();
    by_divergence.sort_by(|&a, &b| {
        prepared[b]
            .divergence
            .cmp(&prepared[a].divergence)
            .then(prepared[b].call_count.cmp(&prepared[a].call_count))
            .then(prepared[a].occurrence_id.cmp(&prepared[b].occurrence_id))
    });
    if let Some(index) = by_divergence.iter().find(|&&i| !assigned.contains_key(&i)).copied() {
        assigned.insert(index, "most_divergent".to_string());
    }
    if let Some(index) = by_key
        .iter()
        .find(|&&i| prepared[i].failure && !assigned.contains_key(&i))
        .copied()
    {
        assigned.insert(index, "failure".to_string());
    }
    let mut fill = 0u64;
    for &index in &by_key {
        if assigned.contains_key(&index) {
            continue;
        }
        fill += 1;
        assigned.insert(index, format!("fill:{fill}"));
    }
    let mut out: Vec<(usize, String)> = assigned.into_iter().collect();
    out.sort_by_key(|(index, label)| (slot_rank(label), prepared[*index].occurrence_id.clone()));
    out
}

fn removable_position(selected: &[(usize, String)]) -> Option<usize> {
    selected
        .iter()
        .enumerate()
        .filter(|(_, entry)| !matches!(entry.1.as_str(), "earliest" | "latest"))
        .map(|(position, entry)| (slot_rank(&entry.1), position))
        .max_by_key(|(rank, position)| (*rank, *position))
        .map(|(_, position)| position)
}

fn cap_selection(selected: &mut Vec<(usize, String)>) {
    while selected.len() > MAX_REPRESENTATIVES {
        let Some(position) = removable_position(selected) else {
            break;
        };
        selected.remove(position);
    }
}

fn finalize_bundle(mut bundle: Value) -> Value {
    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundle_id");
    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundle_digest");
    let id = format!("bun-{}", &value_digest(&bundle)[..16]);
    bundle["bundle_id"] = json!(id);
    let digest = format!("sha256:{}", value_digest(&bundle));
    bundle["bundle_digest"] = json!(digest);
    bundle
}

fn assemble(
    stage_bundle: &Path,
    bundle_base: &Value,
    motif_skeleton: &Value,
    prepared: &[PreparedOccurrence],
    selected: &[(usize, String)],
    occurrences_total: usize,
) -> Result<Assembled> {
    let mut representatives = Vec::with_capacity(selected.len());

    for &(index, ref label) in selected {
        let row = &prepared[index];
        let mut excerpt_bytes = 0usize;
        for evidence in &row.evidence {
            excerpt_bytes += evidence
                .get("excerpt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .len();
        }
        if excerpt_bytes > MAX_OCCURRENCE_BYTES {
            return Err(err("occurrence excerpt exceeds the 4096-byte citation limit"));
        }

        representatives.push(json!({
            "occurrence_id": row.occurrence_id,
            "selection": label,
            "source_alias": row.source_alias,
            "call_count": row.call_count,
            "result_status": row.result_status,
            "input_identity": row.identity,
            "evidence": row.evidence,
        }));
    }
    let mut motif = motif_skeleton.clone();
    motif["representatives"] = json!(representatives);
    motif["occurrences_total"] = json!(occurrences_total);
    motif["representatives_selected"] = json!(selected.len());
    motif["representatives_shortfall"] = json!(selected.len() < occurrences_total);

    let motif_rel = format!("motifs/{}/motif.json", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let evidence_rel = format!("motifs/{}/evidence.md", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let questions_rel = format!("motifs/{}/questions.md", motif_skeleton["motif_id"].as_str().unwrap_or(""));
    let motif_body = serde_json::to_string_pretty(&motif).map_err(|e| err(format!("JSON render: {e}")))?;
    let evidence_body = render_evidence_view(&motif);
    let questions_body = render_questions_view(&motif);
    let hash = |bytes: &[u8]| format!("sha256:{}", sha256_hex(bytes));
    let mut bundle = bundle_base.clone();
    bundle["motifs"] = json!([{
        "evidence_path": evidence_rel,
        "evidence_sha256": hash(evidence_body.as_bytes()),
        "motif_id": motif_skeleton["motif_id"],
        "motif_path": motif_rel,
        "motif_sha256": hash(motif_body.as_bytes()),
        "questions_path": questions_rel,
        "questions_sha256": hash(questions_body.as_bytes()),
    }]);
    let bundle = finalize_bundle(bundle);
    let bundle_body = serde_json::to_string_pretty(&bundle).map_err(|e| err(format!("JSON render: {e}")))?;
    let files = vec![
        ("bundle.json".to_string(), bundle_body.into_bytes()),
        (motif_rel, motif_body.into_bytes()),
        (evidence_rel, evidence_body.into_bytes()),
        (questions_rel, questions_body.into_bytes()),
    ];
    let total_bytes: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
    for (rel, bytes) in &files {
        let path = stage_bundle.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| err(format!("create directory: {e}")))?;
        }
        fs::write(path, bytes).map_err(|e| err(format!("write failed: {e}")))?;
    }

    Ok(Assembled {
        bundle,
        motif,
        files,
        total_bytes,
    })
}

fn leak_patterns() -> Vec<Regex> {
    let raw = [
        r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s"'<>\x60]+"#,
        r#"[A-Za-z]:[\\/][^\s"'<>\x60|]+"#,
        r#"\\\\[^\s"'<>\x60|]+"#,
        r#"/(?:home|Users|root|tmp|var|etc|opt|mnt|workspace|data)/[^\s"'<>\x60|]+"#,
        r#"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"#,
        r#"(?:sk-|ghp_|github_pat_|AKIA|ASIA|xox[abp]-)[A-Za-z0-9_\-]{8,}"#,
        r#"[A-Za-z0-9+/=_-]{48,}"#,
    ];
    raw.iter()
        .map(|source| Regex::new(source).expect("fixed leak pattern"))
        .collect()
}

fn scan_no_leaks(assembled: &Assembled, private_strings: &BTreeSet<String>) -> Result<()> {
    for (rel, bytes) in &assembled.files {
        let text = String::from_utf8_lossy(bytes);
        for private in private_strings {
            if private.len() >= 8 && text.contains(private.as_str()) {
                return Err(err(format!("private tracking identity would leak in {rel}")));
            }
        }
    }
    let patterns = leak_patterns();
    let excerpts: Vec<&str> = assembled.motif["representatives"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row["evidence"].as_array().into_iter().flatten())
        .filter_map(|evidence| evidence["excerpt"].as_str())
        .collect();
    for excerpt in excerpts {
        for pattern in &patterns {
            if pattern.is_match(excerpt) {
                return Err(err("unredacted result text would leak in an excerpt"));
            }
        }
    }
    Ok(())
}

fn build_local_map(
    published: &Publication,
    idx: &Index,
    aliases: &HashMap<u64, String>,
) -> Result<Value> {
    let core = &published.manifest.core_digest;
    let snapshot_id = format!("snap-{}", &core[..16.min(core.len())]);
    let mut files = published.manifest.files.clone();
    files.sort_by_key(|row| row.id);
    let file_rows: Vec<Value> = files
        .iter()
        .map(|row| {
            json!({
                "file_id": row.id,
                "sha256": format!("sha256:{}", row.digest),
                "size": row.size,
            })
        })
        .collect();
    let mut alias_rows = BTreeMap::new();
    for row in &files {
        let alias = aliases
            .get(&(row.id as u64))
            .ok_or_else(|| err("snapshot file has no private alias"))?;
        let source_path = idx.file(row.id)?.path().to_string();
        let path = absolute(Path::new(&source_path)).to_string_lossy().replace('\\', "/");
        alias_rows.insert(alias.clone(), json!({
            "file_id": row.id,
            "path": path,
            "sha256": format!("sha256:{}", row.digest),
        }));
    }
    Ok(json!({
        "schema": "ase-p4-local-sources-v1",
        "snapshot": {
            "core_digest": core,
            "files": file_rows,
            "snapshot_id": snapshot_id,
        },
        "aliases": alias_rows,
    }))
}

pub fn run(args: &[String]) -> Result<()> {
    let target = positional_target(args)?;
    let motif_id = target
        .strip_prefix("motif:")
        .ok_or_else(|| err("export target must be motif:<id>"))?;
    if motif_id.is_empty() || motif_id.contains('/') || motif_id.contains('\\') {
        return Err(err("invalid motif id"));
    }
    let out = absolute(&PathBuf::from(
        opt(args, "--out").ok_or_else(|| err("export needs --out DIR"))?,
    ));
    let local_map = absolute(&PathBuf::from(
        opt(args, "--local-map").ok_or_else(|| err("export needs --local-map FILE"))?,
    ));
    if local_map.starts_with(&out) {
        return Err(err("private local map must live outside the bundle directory"));
    }
    if out.exists() {
        let entries = fs::read_dir(&out).map_err(|e| err(format!("read output directory: {e}")))?;
        if entries.count() != 0 {
            return Err(err("export output directory is not empty"));
        }
    }
    let targets = collect_addresses(args)?;
    let (idx, store, published) = require_publication(args)?;
    let config = pinned_config();
    let mut sessions = BTreeSet::new();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in &targets {
        let member =
            ase_motif::membership_scoped(&idx, &store, &target.token, target.project.as_deref())?;
        for attempt in &member.attempts {
            sessions.insert(attempt.child_session_id.clone());
        }
        tasks.push(ase_motif::project(&published, &idx, &member)?);
    }
    let report = ase_motif::detect(&published, &idx, &store, &tasks, &config)?;
    let motif = report
        .motifs
        .iter()
        .find(|m| m.id == motif_id)
        .ok_or_else(|| err("motif id is not present in the pinned query result"))?;

    let exe = std::env::current_exe().map_err(|e| err(format!("current executable: {e}")))?;
    let binary_sha256 = digest_file(&exe)?;
    let bundle_base = build_bundle_base(&published, &report, &config, &binary_sha256)?;
    let reference_occurrence = motif
        .occurrences
        .iter()
        .find(|occ| {
            occ.task == motif.reference_task
                && occ.attempt == motif.reference_attempt
                && occ.sequence == motif.reference_sequence
        })
        .ok_or_else(|| err("motif reference occurrence is not reopenable"))?;
    let reference = occurrence_id(&occurrence_value(reference_occurrence))?;
    let motif_skeleton = build_motif_skeleton(motif, &reference);
    let prepared = prepare_occurrences(
        motif,
        &config,
        &idx,
        &store,
        &published,
        &bundle_base,
        &motif_skeleton,
    )?;
    if prepared.is_empty() {
        return Err(err("motif has no eligible non-duplicate occurrences"));
    }

    let stage = absolute(Path::new(".tmp/ase-s7-t3/export-stage"));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| err(format!("clear staging directory: {e}")))?;
    }
    let stage_bundle = stage.join("bundle");
    fs::create_dir_all(&stage_bundle).map_err(|e| err(format!("create staging directory: {e}")))?;

    let mut selected = select_representatives(&prepared);
    cap_selection(&mut selected);
    let mut assembled = assemble(
        &stage_bundle,
        &bundle_base,
        &motif_skeleton,
        &prepared,
        &selected,
        prepared.len(),
    )?;
    while assembled.total_bytes > MAX_BUNDLE_BYTES {
        let Some(position) = removable_position(&selected) else {
            return Err(err("bundle exceeds 32768 bytes with only earliest/latest left"));
        };
        selected.remove(position);
        assembled = assemble(
            &stage_bundle,
            &bundle_base,
            &motif_skeleton,
            &prepared,
            &selected,
            prepared.len(),
        )?;
    }

    let mut private_strings = BTreeSet::new();
    private_strings.insert(published.generation.clone());
    for session in &sessions {
        private_strings.insert(session.clone());
    }
    for motif_row in &report.motifs {
        for occurrence in &motif_row.occurrences {
            private_strings.insert(occurrence.task.clone());
            private_strings.insert(occurrence.attempt.clone());
            private_strings.insert(occurrence.dispatch_id.clone());
            for call in &occurrence.calls {
                private_strings.insert(call.evidence.selector.clone());
            }
        }
    }
    for row in &prepared {
        for value in &row.private {
            private_strings.insert(value.clone());
        }
    }
    for file in 0..idx.file_count {
        let path = absolute(Path::new(idx.file(file)?.path()))
            .to_string_lossy()
            .replace('\\', "/");
        private_strings.insert(path);
    }
    scan_no_leaks(&assembled, &private_strings)?;

    let mut aliases = HashMap::new();
    for file in &published.manifest.files {
        aliases.insert(
            file.id as u64,
            source_alias(file.id as u64, &format!("sha256:{}", file.digest)),
        );
    }
    let map = build_local_map(&published, &idx, &aliases)?;
    let stage_map = stage.join("local-map.json");
    write_json(&stage_map, &map)?;

    let report_value = verify(&stage_bundle, Some(&stage_map), None);
    if report_value["status"].as_str() != Some("verified") {
        let codes: Vec<String> = report_value["failures"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|failure| failure["code"].as_str().map(str::to_string))
            .collect();
        return Err(err(format!(
            "prepublication verification refused the bundle: {}",
            codes.join(",")
        )));
    }
    if !report_value["watermarks"].as_array().map_or(false, Vec::is_empty) {
        return Err(err("prepublication verification found an append watermark"));
    }

    if let Some(parent) = local_map.parent() {
        fs::create_dir_all(parent).map_err(|e| err(format!("create local map directory: {e}")))?;
    }
    write_json(&local_map, &map)?;
    fs::create_dir_all(&out).map_err(|e| err(format!("create output directory: {e}")))?;
    let mut published_paths = Vec::with_capacity(assembled.files.len());
    for (rel, bytes) in &assembled.files {
        let path = out.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| err(format!("create output directory: {e}")))?;
        }
        fs::write(&path, bytes).map_err(|e| err(format!("write failed: {e}")))?;
        published_paths.push(rel.clone());
    }

    let summary = json!({
        "command": "export",
        "status": "exported",
        "scope": "P4 bundle export and prepublication reopen; no gate measurement",
        "motif_id": motif_id,
        "snapshot_id": assembled.bundle["snapshot"]["snapshot_id"],
        "core_digest": assembled.bundle["snapshot"]["core_digest"],
        "bundle_id": assembled.bundle["bundle_id"],
        "bundle_digest": assembled.bundle["bundle_digest"],
        "bundle_dir": out.to_string_lossy().replace('\\', "/"),
        "local_map": local_map.to_string_lossy().replace('\\', "/"),
        "files": published_paths,
        "occurrences_total": prepared.len(),
        "representatives_selected": selected.len(),
        "representatives_shortfall": selected.len() < prepared.len(),
        "evidence_total": report_value["evidence_total"],
        "evidence_verified": report_value["evidence_verified"],
        "verification": {
            "status": report_value["status"],
            "failures": report_value["failures"],
            "watermarks": report_value["watermarks"],
        },
        "note": "Only observations and questions are exported; verdicts, catalog entries, raw output, real paths and alias mappings stay out of the bundle.",
    });
    if has(args, "--json") {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    } else {
        println!(
            "p4 export: {}; bundle {}; representatives {}/{}; evidence {}/{}; verification {}",
            motif_id,
            assembled.bundle["bundle_id"].as_str().unwrap_or(""),
            selected.len(),
            prepared.len(),
            report_value["evidence_verified"].as_u64().unwrap_or(0),
            report_value["evidence_total"].as_u64().unwrap_or(0),
            report_value["status"].as_str().unwrap_or("unverified"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence(index: u64, failure: bool) -> PreparedOccurrence {
        PreparedOccurrence {
            occurrence_id: format!("occ-{index:016x}"),
            order_key: (0, index, index as u32),
            call_count: 3,
            divergence: 0,
            failure,
            result_status: "absent".to_string(),
            source_alias: "src-test".to_string(),
            evidence: Vec::new(),
            identity: "0".repeat(64),
            private: Vec::new(),
        }
    }

    fn labels(selected: &[(usize, String)]) -> Vec<String> {
        selected.iter().map(|(_, label)| label.clone()).collect()
    }

    #[test]
    fn selection_is_bounded_to_five_before_rendering() {
        let prepared: Vec<PreparedOccurrence> = (0..7).map(|i| occurrence(i, false)).collect();
        let mut selected = select_representatives(&prepared);
        assert_eq!(selected.len(), 7);
        cap_selection(&mut selected);
        assert_eq!(selected.len(), MAX_REPRESENTATIVES);
        let labels = labels(&selected);
        for required in ["earliest", "latest", "most_divergent", "fill:1", "fill:2"] {
            assert!(labels.iter().any(|label| label == required), "missing {required}: {labels:?}");
        }
        assert!(!labels.iter().any(|label| label == "fill:3" || label == "fill:4"));
    }

    #[test]
    fn singleton_keeps_earliest_precedence() {
        let prepared = vec![occurrence(0, false)];
        let mut selected = select_representatives(&prepared);
        cap_selection(&mut selected);
        assert_eq!(labels(&selected), vec!["earliest".to_string()]);
    }

    #[test]
    fn failure_slot_is_replaced_before_earliest_or_latest() {
        let prepared: Vec<PreparedOccurrence> = (0..6)
            .map(|i| occurrence(i, i == 3))
            .collect();
        let mut selected = select_representatives(&prepared);
        cap_selection(&mut selected);
        let labels = labels(&selected);
        assert_eq!(labels.len(), 5);
        assert!(labels.iter().any(|label| label == "failure"), "{labels:?}");
        assert!(labels.iter().any(|label| label == "earliest"), "{labels:?}");
        assert!(labels.iter().any(|label| label == "latest"), "{labels:?}");
    }
}
