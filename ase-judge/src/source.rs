//! Reopenable OMP journal projection for the P3 decision unit.
//!
//! The leaf and active-path rule come from `ase-work`'s locked
//! `turn-episode-v1` projection; this module only adds the human/no-prose
//! decision anchors on that path and re-reads them for validation. After a
//! journal is loaded, every decision is addressed by two exact source spans
//! plus their line digests, and the rendered input digest is re-derived from
//! those bytes before an assertion may be stored.

use ase_core::{err, Result};
use ase_work::{project_bytes, ProjectOptions, JournalProjection};
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::assertion::{
    render_decision, sha256_hex, Anchor, Assertion, Evidence,
};

pub trait EvidenceVerifier {
    /// Re-reads the assertion's two spans and confirms that its evidence,
    /// unit, decision index and rendered input all match current source bytes.
    fn verify_source(&self, assertion: &Assertion) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct HumanMessage {
    pub anchor: Anchor,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Journal {
    pub rel_path: String,
    pub session_id: String,
    pub sha256: String,
    pub size: u64,
    pub leaf_entry_id: Option<String>,
    /// Human user messages on the active leaf path, in path order.
    pub human: Vec<HumanMessage>,
    /// Every `role = user` message on the active path, including
    /// parent-agent-attributed ones. Reported for cross-checks only.
    pub all_role_user: usize,
    /// The product projection's user-message count for this file across all
    /// branches. The active-path count must never exceed it.
    pub projection_user_all_branches: u32,
    pub duplicate_entry_ids: usize,
}

#[derive(Deserialize)]
struct RawRecord {
    #[serde(rename = "type")]
    ty: Option<String>,
    id: Option<String>,
    #[serde(rename = "parentId")]
    _parent_id: Option<String>,
    message: Option<Value>,
}

#[derive(Deserialize)]
struct RawMessage {
    role: Option<String>,
    content: Option<Value>,
    attribution: Option<String>,
}

struct LineRecord {
    ordinal: u32,
    off: u64,
    len: u32,
    line_sha256: String,
    entry_id: Option<String>,
    source_type: String,
    role: Option<String>,
    attribution: Option<String>,
    text: String,
}

fn decode_texts(content: &Value) -> Vec<String> {
    let mut texts = Vec::new();
    match content {
        Value::String(text) => texts.push(text.clone()),
        Value::Array(blocks) => {
            for block in blocks {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        texts.push(text.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    texts
}

/// Parses non-blank JSONL records with the same ordinal/offset convention as
/// the cohort pin: `ordinal` counts non-blank lines from zero, `off` is the
/// byte start of the line and `len` excludes the trailing LF/CR.
fn parse_lines(bytes: &[u8]) -> Vec<LineRecord> {
    let mut records = Vec::new();
    let mut ordinal = 0u32;
    let mut off = 0usize;
    for segment in bytes.split(|b| *b == b'\n') {
        if segment.is_empty() && off >= bytes.len() {
            break;
        }
        let raw_len = segment.len();
        let mut len = raw_len;
        if len > 0 && segment[len - 1] == b'\r' {
            len -= 1;
        }
        let line = &segment[..len];
        let span_off = off as u64;
        off += raw_len + 1;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let digest = sha256_hex(line);
        let current = ordinal;
        ordinal += 1;
        let Ok(record) = serde_json::from_slice::<RawRecord>(line) else {
            continue;
        };
        let message = record
            .message
            .as_ref()
            .and_then(|value| serde_json::from_value::<RawMessage>(value.clone()).ok());
        let (role, attribution, text) = match message {
            Some(message) => {
                let text = message
                    .content
                    .as_ref()
                    .map(decode_texts)
                    .unwrap_or_default()
                    .join("\n");
                (message.role, message.attribution, text)
            }
            None => (None, None, String::new()),
        };
        records.push(LineRecord {
            ordinal: current,
            off: span_off,
            len: len as u32,
            line_sha256: digest,
            entry_id: record.id.filter(|id| !id.is_empty()),
            source_type: record.ty.unwrap_or_default(),
            role,
            attribution,
            text,
        });
    }
    records
}

fn ensure_inside(root: &Path, rel_path: &str) -> Result<PathBuf> {
    let candidate = root.join(rel_path);
    let real_root = fs::canonicalize(root)
        .map_err(|e| err(format!("cannot resolve corpus root {}: {e}", root.display())))?;
    let real = fs::canonicalize(&candidate)
        .map_err(|e| err(format!("cannot resolve journal {rel_path}: {e}")))?;
    if !real.starts_with(&real_root) {
        return Err(err(format!("journal escapes the corpus root: {rel_path}")));
    }
    Ok(real)
}

impl Journal {
    pub fn load(root: &Path, rel_path: &str) -> Result<Self> {
        let path = ensure_inside(root, rel_path)?;
        let bytes = fs::read(&path)
            .map_err(|e| err(format!("read journal {rel_path}: {e}")))?;
        let sha256 = sha256_hex(&bytes);
        let projection: JournalProjection = project_bytes(
            &ProjectOptions {
                file_id: 0,
                path: rel_path,
                source_id: rel_path,
                generation: "ase-s6-pin",
                sha256: &sha256,
                event_base: 0,
                explicit_leaf: None,
                window: None,
            },
            &bytes,
        )?;
        let session_id = projection
            .session_id
            .clone()
            .ok_or_else(|| err(format!("journal {rel_path} has no session id")))?;
        let records = parse_lines(&bytes);
        let mut by_id: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, record) in records.iter().enumerate() {
            if let Some(id) = &record.entry_id {
                by_id.entry(id.clone()).or_default().push(index);
            }
        }
        let duplicate_entry_ids = by_id.values().filter(|v| v.len() > 1).count();
        let active_ids: HashSet<&str> = projection
            .path_nodes
            .iter()
            .map(|node| node.entry_id.as_str())
            .collect();
        let mut active_indices: Vec<usize> = Vec::new();
        for id in &active_ids {
            let indices = by_id
                .get(*id)
                .ok_or_else(|| err(format!("active path id {id} not found in {rel_path}")))?;
            if indices.len() != 1 {
                return Err(err(format!(
                    "active path id {id} is duplicated in {rel_path}; refusing an arbitrary choice"
                )));
            }
            active_indices.push(indices[0]);
        }
        active_indices.sort_by_key(|index| records[*index].ordinal);
        // Keep path order exactly as the projection walked it.
        let path_order: HashMap<&str, usize> = projection
            .path_nodes
            .iter()
            .enumerate()
            .map(|(position, node)| (node.entry_id.as_str(), position))
            .collect();
        active_indices.sort_by_key(|index| {
            records[*index]
                .entry_id
                .as_deref()
                .and_then(|id| path_order.get(id).copied())
                .unwrap_or(usize::MAX)
        });

        let mut human = Vec::new();
        let mut all_role_user = 0usize;
        for index in active_indices {
            let record = &records[index];
            if record.source_type != "message" || record.role.as_deref() != Some("user") {
                continue;
            }
            all_role_user += 1;
            let human_attributed = matches!(record.attribution.as_deref(), None | Some("user"));
            if !human_attributed {
                continue;
            }
            let Some(entry_id) = &record.entry_id else {
                continue;
            };
            human.push(HumanMessage {
                anchor: Anchor {
                    ordinal: record.ordinal,
                    entry_id: entry_id.clone(),
                    off: record.off,
                    len: record.len,
                    line_sha256: record.line_sha256.clone(),
                },
                text: record.text.clone(),
            });
        }
        let leaf_entry_id = projection.active_leaf.entry_id.clone();
        Ok(Journal {
            rel_path: rel_path.to_string(),
            session_id,
            sha256,
            size: bytes.len() as u64,
            leaf_entry_id,
            human,
            all_role_user,
            projection_user_all_branches: projection.counts.user_messages_primary,
            duplicate_entry_ids,
        })
    }

    pub fn decision_count(&self) -> usize {
        self.human.len().saturating_sub(1)
    }

    pub fn decision(&self, index: u32) -> Result<(Evidence, String)> {
        let index = index as usize;
        let prev = self
            .human
            .get(index)
            .ok_or_else(|| err(format!("decision {index} has no earlier message")))?;
        let curr = self
            .human
            .get(index + 1)
            .ok_or_else(|| err(format!("decision {index} has no later message")))?;
        let evidence = Evidence {
            prev: prev.anchor.clone(),
            curr: curr.anchor.clone(),
        };
        let render = render_decision(&prev.text, &curr.text);
        Ok((evidence, render))
    }
}

/// Corpus-backed verifier and journal cache. One journal is loaded and
/// projected at most once per process; insert validation and the evaluator
/// therefore share the same reopenable projection.
pub struct CorpusSource {
    root: PathBuf,
    paths: RefCell<HashMap<String, String>>,
    journals: RefCell<HashMap<String, Rc<Journal>>>,
}

impl CorpusSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            paths: RefCell::new(HashMap::new()),
            journals: RefCell::new(HashMap::new()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn register(&self, unit_id: &str, rel_path: &str) {
        self.paths
            .borrow_mut()
            .insert(unit_id.to_string(), rel_path.to_string());
    }

    pub fn human_text(&self, unit_id: &str, index: u32) -> Result<String> {
        let journal = self.journal(unit_id)?;
        journal
            .human
            .get(index as usize)
            .map(|message| message.text.clone())
            .ok_or_else(|| err(format!("unit {unit_id} has no human message {index}")))
    }

    pub fn journal(&self, unit_id: &str) -> Result<Rc<Journal>> {
        if let Some(journal) = self.journals.borrow().get(unit_id) {
            return Ok(journal.clone());
        }
        let rel_path = self
            .paths
            .borrow()
            .get(unit_id)
            .cloned()
            .ok_or_else(|| err(format!("no journal path registered for unit {unit_id}")))?;
        let journal = Rc::new(Journal::load(&self.root, &rel_path)?);
        self.journals
            .borrow_mut()
            .insert(unit_id.to_string(), journal.clone());
        Ok(journal)
    }
}

impl EvidenceVerifier for CorpusSource {
    fn verify_source(&self, assertion: &Assertion) -> Result<()> {
        let journal = self.journal(&assertion.unit_id)?;
        let (evidence, render) = journal.decision(assertion.decision_index)?;
        if evidence != assertion.evidence {
            return Err(err(format!(
                "unit {} decision {} evidence does not match the projected source pair",
                assertion.unit_id, assertion.decision_index
            )));
        }
        let render_sha256 = sha256_hex(render.as_bytes());
        if render_sha256 != assertion.render_sha256 {
            return Err(err(format!(
                "unit {} decision {} rendered input digest does not match source text",
                assertion.unit_id, assertion.decision_index
            )));
        }
        Ok(())
    }
}
