//! Generation-pinned CallSig sequence projection with per-call evidence.
//!
//! A sequence is rebuilt from the published `blocks` rows of one attempt's
//! child journal, using the same continuation rule the postings writer used:
//! the next call-bearing event continues the current sequence only when its
//! resolved parent is the previous call-bearing event of the same file and
//! local session. Forks, roots, unresolved parents and session changes start a
//! new sequence, so no sequence can cross a branch or session boundary.
//!
//! Every call keeps `(generation, event, selector, span, block_hash)` so it can
//! be reopened from raw bytes and re-derived through the versioned selector.

use ase_core::{err, FileId, ProjectionKind, RawStore, Result, SpanRef, NONE};
use ase_index::Index;
use ase_postings::publication::{self, Publication};

use crate::membership::{AttemptMembership, TaskMembership};

/// One reopenable call inside a projected sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallEvidence {
    pub generation: String,
    pub event: u32,
    pub file: FileId,
    pub span: SpanRef,
    pub selector: String,
    pub block_hash: String,
    pub session_local: u32,
}

/// One linear call sequence. It never spans a fork or a session boundary.
#[derive(Clone, Debug)]
pub struct Sequence {
    /// Index of the owning attempt inside `TaskSequences::attempts`.
    pub attempt_index: usize,
    /// Deterministic position inside the attempt's sequence list.
    pub index: usize,
    pub file: FileId,
    pub session_local: u32,
    /// Parent event that started this sequence; `NONE` for a root sequence.
    pub boundary_parent: u32,
    pub calls: Vec<CallEvidence>,
}

impl Sequence {
    /// `shingle-k3k5-v1` compatible windows: the constituent call block hashes
    /// joined by `-`. Empty sequences produce no tokens.
    pub fn shingle_tokens(&self, k_min: u32, k_max: u32) -> Vec<String> {
        let hashes: Vec<&str> = self
            .calls
            .iter()
            .map(|call| call.block_hash.as_str())
            .collect();
        let mut tokens = Vec::new();
        if k_min == 0 || k_max < k_min {
            return tokens;
        }
        for k in k_min..=k_max {
            let k = k as usize;
            if hashes.len() < k {
                break;
            }
            for window in hashes.windows(k) {
                tokens.push(window.join("-"));
            }
        }
        tokens
    }
}

/// All sequences projected for one admitted attempt.
#[derive(Clone, Debug)]
pub struct AttemptSequences {
    pub membership: AttemptMembership,
    pub sequences: Vec<Sequence>,
}

impl AttemptSequences {
    pub fn call_count(&self) -> usize {
        self.sequences.iter().map(|sequence| sequence.calls.len()).sum()
    }
}

/// Sequences for one task token, grouped by admitted attempt.
#[derive(Clone, Debug)]
pub struct TaskSequences {
    pub token: String,
    pub task_key: Option<String>,
    pub attempts: Vec<AttemptSequences>,
}

impl TaskSequences {
    pub fn call_count(&self) -> usize {
        self.attempts.iter().map(AttemptSequences::call_count).sum()
    }

    /// Document order: attempt by attempt, sequence by sequence.
    pub fn flat_sequences(&self) -> Vec<&Sequence> {
        self.attempts
            .iter()
            .flat_map(|attempt| attempt.sequences.iter())
            .collect()
    }
}

fn hex8(hash: &[u8; 8]) -> String {
    let mut out = String::with_capacity(16);
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Refuse to project unless the publication and the address are the same
/// pinned generation this layer understands.
fn validate_pinned(published: &Publication, idx: &Index) -> Result<()> {
    if published.pointer.status != "ready" {
        return Err(err("publication pointer is not ready"));
    }
    if published.generation != published.pointer.generation
        || published.generation != published.manifest.generation
    {
        return Err(err("publication generation differs between pointer and manifest"));
    }
    publication::validate_versions(&published.pointer.versions)?;
    let pinned_events = published
        .manifest
        .counts
        .get("events")
        .copied()
        .ok_or_else(|| err("manifest does not declare an event count"))?;
    if pinned_events != idx.event_count as u64 {
        return Err(err("address event count differs from the pinned manifest"));
    }
    if let Some(pinned_files) = published.manifest.counts.get("files") {
        if *pinned_files != idx.file_count as u64 {
            return Err(err("address file count differs from the pinned manifest"));
        }
    }
    Ok(())
}

/// Rebuild sequences for every admitted attempt of a dispatch-only membership.
///
/// A membership with no admitted attempt yields no sequences; a block that
/// disagrees with the pinned address or with the membership session is an
/// error, never a silent exclusion.
pub fn project(
    published: &Publication,
    idx: &Index,
    membership: &TaskMembership,
) -> Result<TaskSequences> {
    validate_pinned(published, idx)?;
    let mut attempts = Vec::with_capacity(membership.attempts.len());
    for (attempt_index, attempt) in membership.attempts.iter().enumerate() {
        attempts.push(project_attempt(
            published,
            idx,
            attempt,
            attempt_index,
        )?);
    }
    Ok(TaskSequences {
        token: membership.token.clone(),
        task_key: membership.task_key.clone(),
        attempts,
    })
}

fn span_label(span: SpanRef) -> String {
    format!("span:{}:{}:{}", span.file, span.off, span.len)
}

fn project_attempt(
    published: &Publication,
    idx: &Index,
    attempt: &AttemptMembership,
    attempt_index: usize,
) -> Result<AttemptSequences> {
    // The membership must still address this exact snapshot: the dispatch
    // child record, its containing session and its file all have to agree.
    let link = idx.event(attempt.child_event)?;
    if link.span().file != attempt.child_file_id || span_label(link.span()) != attempt.child_span {
        return Err(err("dispatch membership no longer matches the pinned address"));
    }
    if idx.session(link.session())?.id() != attempt.child_session_id {
        return Err(err("dispatch membership child session differs from the pinned address"));
    }
    let blocks = ase_postings::call_blocks_by_file(published, attempt.child_file_id)?;
    let mut sequences: Vec<Sequence> = Vec::new();
    let mut current: Vec<CallEvidence> = Vec::new();
    let mut last_call_event = NONE;
    let mut sequence_file = attempt.child_file_id;
    let mut sequence_session = 0u32;
    let mut sequence_parent = NONE;

    // One address event may carry several call blocks. The writer appends all
    // of them to the same sequence and decides continuation once per event, so
    // the projection must group by event before applying the parent rule.
    let mut cursor = 0usize;
    while cursor < blocks.len() {
        let event_index = blocks[cursor].event;
        let mut end = cursor;
        while end < blocks.len() && blocks[end].event == event_index {
            end += 1;
        }
        let event = idx.event(event_index)?;
        let span = event.span();
        let session_index = event.session();
        if session_index == NONE {
            return Err(err(format!(
                "call block for event {event_index} has no indexed session"
            )));
        }
        let session = idx.session(session_index)?;
        if session.id() != attempt.child_session_id || session.file() != attempt.child_file_id {
            return Err(err(format!(
                "call block for event {event_index} belongs to session {} rather than membership child {}",
                session.id(),
                attempt.child_session_id
            )));
        }
        for block in &blocks[cursor..end] {
            if block.span != span {
                return Err(err(format!(
                    "call block for event {event_index} disagrees with the pinned address span"
                )));
            }
            if block.kind != ProjectionKind::Call.label() {
                return Err(err("call projection query returned a non-call block"));
            }
            if block.parent_idx != event.parent() {
                return Err(err(format!(
                    "call block for event {event_index} carries a different parent than the address"
                )));
            }
        }

        let parent = event.parent();
        let session_local = blocks[cursor].session_local;
        let continues = !current.is_empty()
            && parent != NONE
            && parent == last_call_event
            && session_local == sequence_session
            && span.file == sequence_file;
        if !current.is_empty() && !continues {
            sequences.push(Sequence {
                attempt_index,
                index: sequences.len(),
                file: sequence_file,
                session_local: sequence_session,
                boundary_parent: sequence_parent,
                calls: std::mem::take(&mut current),
            });
        }
        if current.is_empty() {
            sequence_file = span.file;
            sequence_session = session_local;
            sequence_parent = parent;
        }
        for block in &blocks[cursor..end] {
            if block.session_local != session_local {
                return Err(err(format!(
                    "call blocks of event {event_index} mix local sessions"
                )));
            }
            current.push(CallEvidence {
                generation: published.generation.clone(),
                event: block.event,
                file: span.file,
                span,
                selector: block.selector.clone(),
                block_hash: block.block_hash.clone(),
                session_local: block.session_local,
            });
        }
        last_call_event = event_index;
        cursor = end;
    }
    if !current.is_empty() {
        sequences.push(Sequence {
            attempt_index,
            index: sequences.len(),
            file: sequence_file,
            session_local: sequence_session,
            boundary_parent: sequence_parent,
            calls: current,
        });
    }

    Ok(AttemptSequences {
        membership: attempt.clone(),
        sequences,
    })
}

/// Re-derive one projected call from raw bytes.
///
/// The evidence must belong to this exact published generation, address the
/// same pinned span, still match the indexed span checksum, and re-derive the
/// same call block through its versioned selector. Any mismatch is an error;
/// a stale digest is never reported as verified.
pub fn reopen_call(
    published: &Publication,
    idx: &Index,
    store: &dyn RawStore,
    evidence: &CallEvidence,
) -> Result<String> {
    if evidence.generation != published.generation {
        return Err(err("call evidence belongs to another generation"));
    }
    if evidence.event >= idx.event_count {
        return Err(err("call evidence event is outside the pinned address"));
    }
    let event = idx.event(evidence.event)?;
    if event.span() != evidence.span || evidence.file != evidence.span.file {
        return Err(err("call evidence span differs from the pinned address"));
    }
    let raw = store.span(evidence.span)?;
    if ase_ingest::util::fnv64(&raw) != event.hash() {
        return Err(err("raw span no longer matches the indexed checksum"));
    }
    let provider = idx.file(evidence.span.file)?.provider();
    let block = ase_ingest::extract_block(provider, &raw, &evidence.selector)?;
    if block.kind != ProjectionKind::Call {
        return Err(err("selector does not address a call block"));
    }
    let call_sig = block
        .call_sig
        .ok_or_else(|| err("selector re-derived no call signature"))?;
    if hex8(&block.block_hash) != evidence.block_hash {
        return Err(err("re-derived call digest differs from the projected evidence"));
    }
    Ok(call_sig)
}

