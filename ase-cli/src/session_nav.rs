//! Session locators for one verified evidence block.
//!
//! A locator is emitted only from a session whose id is declared in its own
//! header, still re-reads to that id, and is unique in the pinned index. The
//! pinned generation address is the snapshot every emitted command targets;
//! a rebuilt base index or a path-derived id never yields a shortcut.

use crate::show::span_label;
use ase_core::{EventIdx, Provider, RawStore, Result, NONE};
use ase_index::{FsRawStore, Index};
use ase_ingest::util::fnv64;
use ase_postings::Publication;
use serde_json::{json, Value};


/// The session id declared by one header line, parsed from its raw bytes.
pub(crate) fn declared_session_id(provider: Provider, raw: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(raw).ok()?;
    match provider {
        Provider::Omp => {
            if value.get("type")?.as_str()? != "session" {
                return None;
            }
            value.get("id")?.as_str().map(str::to_string)
        }
        Provider::Codex => {
            if value.get("type")?.as_str()? != "session_meta" {
                return None;
            }
            value.get("payload")?.get("id")?.as_str().map(str::to_string)
        }
    }
}

fn refused(reason: &str, detail: &str, extra: Value) -> Value {
    let mut value = json!({
        "status": "refused",
        "reason": reason,
        "detail": detail,
        "span_evidence": "span evidence remains addressable on the pinned snapshot; no session shortcut is emitted"
    });
    if let (Some(object), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in extra {
            object.insert(key.clone(), item.clone());
        }
    }
    value
}

fn snapshot(published: &Publication) -> Value {
    json!({
        "generation": &published.generation,
        "address": published.address_path.to_string_lossy().replace('\\', "/"),
        "address_sha256": &published.pointer.address.sha256
    })
}

/// Session navigation for one `open` result. `verified` is the same decision
/// `open` reports; an unverified block never receives ready-to-run commands.
pub(crate) fn session_args(
    idx: &Index,
    store: &FsRawStore,
    published: &Publication,
    event: EventIdx,
    verified: bool,
) -> Result<Value> {
    if !verified {
        return Ok(refused(
            "evidence_unverified",
            "the block did not match the pinned span and digest; reindex or restore the source before navigating",
            json!({}),
        ));
    }
    let ev = idx.event(event)?;
    let session_idx = ev.session();
    if session_idx == NONE {
        return Ok(refused(
            "session_not_recorded",
            "the pinned event has no indexed session record",
            json!({}),
        ));
    }
    let file = idx.file(ev.span().file)?;
    let provider = file.provider();
    let session = idx.session(session_idx)?;
    if session.path_derived() {
        return Ok(refused(
            "session_id_not_declared",
            "the indexed session id was derived from the source file name; only a header-declared id is a session locator",
            json!({"id_source": "file_name"}),
        ));
    }
    let id = session.id().to_string();
    if id.is_empty() {
        return Ok(refused(
            "session_id_not_declared",
            "the session header does not declare an id",
            json!({}),
        ));
    }
    let header_idx = session.header();
    if header_idx == NONE {
        return Ok(refused(
            "header_missing",
            "the session has no indexed header event to re-check its id",
            json!({"id": &id}),
        ));
    }
    let header = idx.event(header_idx)?;
    if header.session() != session_idx {
        return Ok(refused(
            "header_missing",
            "the session header event belongs to another indexed session",
            json!({"id": &id}),
        ));
    }
    let header_span = header.span();
    let raw = match store.span(header_span) {
        Ok(raw) => raw,
        Err(_) => {
            return Ok(refused(
                "header_unreadable",
                "the session header span cannot be re-read from the pinned source",
                json!({"id": &id, "header_span": span_label(header_span)}),
            ))
        }
    };
    if fnv64(&raw) != header.hash() {
        return Ok(refused(
            "header_edited",
            "the session header bytes no longer match the indexed span; the declared id is no longer verified",
            json!({"id": &id, "header_span": span_label(header_span)}),
        ));
    }
    let declared = declared_session_id(provider, &raw);
    match declared.as_deref() {
        None => {
            return Ok(refused(
                "session_id_not_declared",
                "the header no longer declares a session id",
                json!({"id": &id, "header_span": span_label(header_span)}),
            ))
        }
        Some(declared_id) if declared_id != id => {
            return Ok(refused(
                "header_id_mismatch",
                "the header declares a different id than the indexed session",
                json!({"id": &id, "header_id": declared_id}),
            ))
        }
        Some(_) => {}
    }
    let matches = idx.sessions_with_id(&id);
    if matches.len() > 1 {
        return Ok(refused(
            "duplicate_session_id",
            "more than one indexed session declares this header id; a session:ID shortcut could open the wrong one",
            json!({"id": &id, "matches": matches.len()}),
        ));
    }
    if matches.first() != Some(&session_idx) {
        return Ok(refused(
            "session_id_mismatch",
            "the indexed session id does not resolve to its own recorded session",
            json!({"id": &id, "matches": matches.len()}),
        ));
    }
    let address = published.address_path.to_string_lossy().replace('\\', "/");
    let target = format!("session:{id}");
    let show_flags = json!([target, "--index", &address]);
    let resolve_flags = json!([target, "--index", &address, "--explain"]);
    let work = if provider == Provider::Omp {
        json!({
            "available": true,
            "flags": ["--session", &id, "--index", &address],
            "leaf": {
                "default": "active",
                "explicit_flag": "--leaf",
                "note": "the default projects the active graph leaf; --leaf projects an explicit tip named by the projection"
            }
        })
    } else {
        json!({
            "available": false,
            "reason": "work session projection is OMP-only"
        })
    };
    Ok(json!({
        "status": "ready",
        "event": event,
        "id": &id,
        "provider": provider.label(),
        "unique": true,
        "matches": 1,
        "snapshot": snapshot(published),
        "header": {
            "event": header_idx,
            "span": span_label(header_span),
            "hash_match": true,
            "id_match": true
        },
        "show": {"target": target, "flags": show_flags},
        "resolve": {"target": target, "flags": resolve_flags},
        "work": work
    }))
}

#[cfg(test)]
mod tests {
    use super::declared_session_id;
    use ase_core::Provider;

    #[test]
    fn test_declared_session_id_reads_only_the_documented_header_field() {
        let omp = br#"{"type":"session","id":"omp-header-id","timestamp":"2026-01-02T03:04:05.000Z"}"#;
        let codex = br#"{"type":"session_meta","payload":{"id":"codex-header-id"}}"#;
        let omp_message = br#"{"type":"message","id":"message-id","message":{"role":"user"}}"#;
        let derived = br#"{"type":"session_meta","payload":{}}"#;
        assert_eq!(declared_session_id(Provider::Omp, omp).as_deref(), Some("omp-header-id"));
        assert_eq!(declared_session_id(Provider::Codex, codex).as_deref(), Some("codex-header-id"));
        assert_eq!(declared_session_id(Provider::Omp, omp_message), None);
        assert_eq!(declared_session_id(Provider::Codex, derived), None);
        assert_eq!(declared_session_id(Provider::Omp, b"{not json"), None);
    }
}
