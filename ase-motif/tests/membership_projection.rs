//! Synthetic membership fixture: dispatch-only membership, generation-pinned
//! CallSig sequences with branch boundaries, per-call evidence and
//! deterministic Jaccard families.
//!
//! The fixture contains only synthetic tool names, sessions and identities.
//! It builds a real address, postings database, manifest and pointer, then
//! exercises the public library API against that pinned generation.

use ase_core::{
    ALLOWLIST_VERSION, CALLSIG_VERSION, NONE, PROJECTION_VERSION, SHINGLE_VERSION,
    TOKENIZER_VERSION,
};
use ase_index::{decode_corpus, sha256_hex, write_address, FsRawStore, Index};
use ase_motif::{
    families, membership, project, reopen_call, FamilyConfig, Sequence, TaskSequences,
};
use ase_postings::{
    publication::{self, ArtifactRef, FileWatermark, Manifest, Pointer},
    shingles_by_token, write_postings, Publication, WriterInput,
};
use ase_task::Status;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const D1: &str = "11111111-1111-4111-8111-111111111111";
const D2: &str = "22222222-2222-4222-8222-222222222222";
const D3: &str = "33333333-3333-4333-8333-333333333333";
const D4_DUP: &str = "44444444-4444-4444-8444-444444444444";
const D5_COMP_A: &str = "55555555-5555-4555-8555-555555555555";
const D6_COMP_B: &str = "66666666-6666-4666-8666-666666666666";
const D7_MISSING: &str = "77777777-7777-4777-8777-777777777777";
const D8_WRONG: &str = "88888888-8888-4888-8888-888888888888";

const PARENT_SESSION: &str = "motif-membership-parent";
const A1_SESSION: &str = "motif-membership-child-a1";
const A2_SESSION: &str = "motif-membership-child-a2";
const A3_SESSION: &str = "motif-membership-child-a3";

struct Fixture {
    root: PathBuf,
    published: Publication,
    index: Arc<Index>,
    store: FsRawStore,
}

fn fixture_root(name: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("workspace root")
                .join("target")
                .join("tmp")
        });
    base.join("motif-membership-motif").join(name)
}

fn session_line(id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session",
            "version": 3,
            "id": id,
            "timestamp": "2026-11-01T00:00:00.000Z",
            "cwd": "C:\\SYNTHETIC\\motif-membership",
            "title": "synthetic membership fixture"
        })
    )
}

fn parent_line(dispatch_id: &str, project: &str, task: &str, attempt: &str, spawn: u32) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": "2026-11-01T00:00:00.500Z",
            "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": dispatch_id,
                "parent_session_id": PARENT_SESSION,
                "parent_tool_call_id": format!("call-{spawn}"),
                "spawn_index": spawn,
                "project_id": project,
                "task_id": task,
                "attempt_id": attempt
            }
        })
    )
}

fn child_line(dispatch_id: &str, session_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": "2026-11-01T00:00:00.750Z",
            "customType": "task_dispatch_child_v1",
            "data": {
                "dispatch_id": dispatch_id,
                "session_id": session_id
            }
        })
    )
}

/// One synthetic assistant turn with a prose block followed by tool calls.
/// The text block keeps selectors `[1..]` for the calls, so a multi-call event
/// also checks within-event ordering.
fn turn_line(id: &str, parent: Option<&str>, tools: &[&str]) -> String {
    let mut content = vec![json!({ "type": "text", "text": format!("synthetic turn {id}") })];
    for tool in tools {
        content.push(json!({
            "type": "toolCall",
            "id": format!("{id}-{tool}"),
            "name": tool,
            "args": { "path": format!("synthetic/{tool}.txt"), "flag": true }
        }));
    }
    let mut value = json!({
        "type": "message",
        "id": id,
        "parentId": serde_json::Value::Null,
        "timestamp": "2026-11-01T00:00:01.000Z",
        "message": { "role": "assistant", "content": content }
    });
    if let Some(parent) = parent {
        value["parentId"] = json!(parent);
    }
    format!("{value}\n")
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).expect("fixture dir");
    }
    fs::write(&path, content).expect("fixture file");
}

fn expected_versions() -> BTreeMap<String, String> {
    [
        ("projection", PROJECTION_VERSION),
        ("callsig", CALLSIG_VERSION),
        ("allowlist", ALLOWLIST_VERSION),
        ("tokenizer", TOKENIZER_VERSION),
        ("shingle", SHINGLE_VERSION),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn build_fixture(name: &str) -> Fixture {
    let root = fixture_root(name);
    let _ = fs::remove_dir_all(&root);
    let omp = root.join("omp");
    let codex = root.join("codex");
    fs::create_dir_all(&omp).expect("omp root");
    fs::create_dir_all(&codex).expect("codex root");
    let index_path = root.join("ase.index");

    // Parent journal: valid joins for task 1 (a1, a2, a3) plus one rejected
    // group for each failure mode the membership rule must refuse.
    let mut parent = session_line(PARENT_SESSION);
    parent.push_str(&parent_line(D1, "MotifProj", "1", "a1", 0));
    parent.push_str(&parent_line(D2, "MotifProj", "1", "a2", 1));
    parent.push_str(&parent_line(D3, "MotifProj", "1", "a3", 2));
    parent.push_str(&parent_line(D4_DUP, "MotifProj", "2", "a1", 3));
    parent.push_str(&parent_line(D4_DUP, "MotifProj", "2", "a1", 4));
    parent.push_str(&parent_line(D5_COMP_A, "MotifProj", "3", "a1", 5));
    parent.push_str(&parent_line(D6_COMP_B, "MotifProj", "3", "b1", 6));
    parent.push_str(&parent_line(D7_MISSING, "MotifProj", "4", "a1", 7));
    parent.push_str(&parent_line(D8_WRONG, "MotifProj", "5", "a1", 8));
    write_file(&omp, "motif-membership/parent.jsonl", &parent);

    // Attempt a1: a five-call chain (ta..te), a fork sibling on the same parent
    // (tf, tg) and an independent root (th, ti).
    let mut a1 = session_line(A1_SESSION);
    a1.push_str(&child_line(D1, A1_SESSION));
    a1.push_str(&turn_line("a1-e1", None, &["ta", "tb", "tc"]));
    a1.push_str(&turn_line("a1-e2", Some("a1-e1"), &["td"]));
    a1.push_str(&turn_line("a1-e3", Some("a1-e2"), &["te"]));
    a1.push_str(&turn_line("a1-e4", Some("a1-e2"), &["tf"]));
    a1.push_str(&turn_line("a1-e5", Some("a1-e4"), &["tg"]));
    a1.push_str(&turn_line("a1-e6", None, &["th", "ti"]));
    write_file(&omp, "motif-membership/child-a1.jsonl", &a1);

    // Attempt a2 shares the first four calls and diverges at the fifth.
    let mut a2 = session_line(A2_SESSION);
    a2.push_str(&child_line(D2, A2_SESSION));
    a2.push_str(&turn_line("a2-e1", None, &["ta", "tb", "tc"]));
    a2.push_str(&turn_line("a2-e2", Some("a2-e1"), &["td"]));
    a2.push_str(&turn_line("a2-e3", Some("a2-e2"), &["tf"]));
    write_file(&omp, "motif-membership/child-a2.jsonl", &a2);

    // Attempt a3 repeats a1's chain exactly, plus an unrelated three-call root.
    let mut a3 = session_line(A3_SESSION);
    a3.push_str(&child_line(D3, A3_SESSION));
    a3.push_str(&turn_line("a3-e1", None, &["ta", "tb", "tc"]));
    a3.push_str(&turn_line("a3-e2", Some("a3-e1"), &["td"]));
    a3.push_str(&turn_line("a3-e3", Some("a3-e2"), &["te"]));
    a3.push_str(&turn_line("a3-e4", None, &["tx", "ty", "tz"]));
    write_file(&omp, "motif-membership/child-a3.jsonl", &a3);

    let mut dup = session_line("motif-membership-child-dup");
    dup.push_str(&child_line(D4_DUP, "motif-membership-child-dup"));
    dup.push_str(&turn_line("dup-e1", None, &["ta", "tb", "tc"]));
    write_file(&omp, "motif-membership/child-dup.jsonl", &dup);

    let mut competing = session_line("motif-membership-child-competing");
    competing.push_str(&child_line(D5_COMP_A, "motif-membership-child-competing"));
    competing.push_str(&child_line(D6_COMP_B, "motif-membership-child-competing"));
    competing.push_str(&turn_line("comp-e1", None, &["ta", "tb", "tc"]));
    write_file(&omp, "motif-membership/child-competing.jsonl", &competing);

    let mut wrong = session_line("motif-membership-child-wrong");
    wrong.push_str(&child_line(D8_WRONG, "motif-membership-ghost"));
    wrong.push_str(&turn_line("wrong-e1", None, &["ta", "tb", "tc"]));
    write_file(&omp, "motif-membership/child-wrong.jsonl", &wrong);

    // A journal whose name could parse to the token but which carries no
    // dispatch record. It must never join membership or contribute sequences.
    let mut untracked = session_line("motif-membership-untracked");
    untracked.push_str(&turn_line("unt-e1", None, &["ta", "tb", "tc"]));
    untracked.push_str(&turn_line("unt-e2", Some("unt-e1"), &["td"]));
    untracked.push_str(&turn_line("unt-e3", Some("unt-e2"), &["te"]));
    untracked.push_str(&turn_line("unt-e4", None, &["tx", "ty", "tz"]));
    write_file(&omp, "motif-membership/Proj88Task1Eta1.jsonl", &untracked);

    let corpus = decode_corpus(&omp, &codex).expect("decode");
    let generation = "motif-membership-fixture-generation";
    let gen_dir = root.join("generations").join(generation);
    fs::create_dir_all(&gen_dir).expect("generation dir");
    let address_path = gen_dir.join("address.bin");
    write_address(&corpus, &address_path).expect("address");
    let (address_bytes, address_sha256) =
        publication::sha256_file(&address_path).expect("address digest");

    let versions = expected_versions();
    let files: Vec<FileWatermark> = corpus
        .files
        .iter()
        .enumerate()
        .map(|(id, file)| FileWatermark {
            id: id as u32,
            provider: file.provider.label().to_string(),
            encoding: file.encoding.label().to_string(),
            size: file.size,
            mtime_secs: file.mtime_secs,
            mtime_nanos: file.mtime_nanos,
            digest: corpus.file_digests[id].clone(),
            events: corpus.decoded[id].events.len() as u32,
            event_start: corpus.bases[id],
            event_end: corpus.bases[id] + corpus.decoded[id].events.len() as u32,
        })
        .collect();
    let base_counts: BTreeMap<String, u64> = [
        ("files", corpus.files.len() as u64),
        ("events", corpus.total_events as u64),
        ("sessions", corpus.stats.sessions as u64),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let core = json!({
        "schema": publication::SCHEMA,
        "versions": versions.clone(),
        "counts": base_counts.clone(),
        "files": files.clone(),
    });
    let core_digest = sha256_hex(core.to_string().as_bytes());

    let postings_path = gen_dir.join("postings.sqlite");
    let stats = write_postings(
        WriterInput {
            corpus: &corpus,
            generation,
            core_digest: &core_digest,
            address_bytes,
            address_sha256: &address_sha256,
        },
        &postings_path,
    )
    .expect("postings");
    let (postings_bytes, postings_sha256) =
        publication::sha256_file(&postings_path).expect("postings digest");

    let mut counts = base_counts.clone();
    for (key, value) in [
        ("blocks", stats.blocks),
        ("prose_blocks", stats.prose_blocks),
        ("call_blocks", stats.call_blocks),
        ("fts_rows", stats.fts_rows),
        ("shingles", stats.shingles),
        ("shingle_abstentions", stats.abstentions),
    ] {
        counts.insert(key.to_string(), value);
    }
    let manifest = Manifest {
        magic: publication::MANIFEST_MAGIC.to_string(),
        schema: publication::SCHEMA,
        generation: generation.to_string(),
        core_digest: core_digest.clone(),
        versions: versions.clone(),
        counts: counts.clone(),
        files,
        address_bytes,
        address_sha256: address_sha256.clone(),
        postings_bytes,
        postings_sha256: postings_sha256.clone(),
    };
    let manifest_path = gen_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&serde_json::to_value(&manifest).expect("manifest value"))
            .expect("manifest bytes"),
    )
    .expect("manifest write");
    let (manifest_bytes, manifest_sha256) =
        publication::sha256_file(&manifest_path).expect("manifest digest");

    let ptr = |name: &str, bytes: u64, sha256: &str| ArtifactRef {
        path: format!("generations/{generation}/{name}"),
        bytes,
        sha256: sha256.to_string(),
    };
    let pointer = Pointer {
        magic: publication::POINTER_MAGIC.to_string(),
        schema: publication::SCHEMA,
        status: "ready".to_string(),
        reason: String::new(),
        generation: generation.to_string(),
        address: ptr("address.bin", address_bytes, &address_sha256),
        postings: ptr("postings.sqlite", postings_bytes, &postings_sha256),
        manifest: ptr("manifest.json", manifest_bytes, &manifest_sha256),
        full_footprint_bytes: address_bytes + postings_bytes + manifest_bytes,
        versions: versions.clone(),
        counts,
    };
    publication::write_pointer(&index_path, &pointer).expect("pointer");
    let published = publication::open(&index_path)
        .expect("open publication")
        .expect("published generation");

    let index = Arc::new(Index::open(&published.address_path).expect("pinned address"));
    let store = FsRawStore::new(index.clone());
    Fixture {
        root,
        published,
        index,
        store,
    }
}

fn signatures(sequence: &Sequence, fixture: &Fixture) -> Vec<String> {
    sequence
        .calls
        .iter()
        .map(|call| {
            reopen_call(
                &fixture.published,
                &fixture.index,
                &fixture.store,
                call,
            )
            .expect("projected call reopens")
        })
        .collect()
}

fn tool_of(signature: &str) -> &str {
    signature.split('|').next().unwrap_or("")
}

fn projected_task(fixture: &Fixture, token: &str) -> TaskSequences {
    let member = membership(&fixture.index, &fixture.store, token).expect("membership");
    project(&fixture.published, &fixture.index, &member).expect("projection")
}

#[test]
fn dispatch_membership_projects_bounded_sequences_and_families() {
    let fixture = build_fixture("sequences");

    // ---- membership comes only from valid dispatch joins -------------------
    let member = membership(&fixture.index, &fixture.store, "1").expect("membership");
    assert_eq!(member.status, Status::Assigned);
    assert_eq!(member.task_key.as_deref(), Some("MotifProj|1"));
    let attempts: Vec<&str> = member.attempts.iter().map(|a| a.attempt.as_str()).collect();
    assert_eq!(attempts, ["a1", "a2", "a3"]);
    let sessions: Vec<&str> = member
        .attempts
        .iter()
        .map(|a| a.child_session_id.as_str())
        .collect();
    assert_eq!(sessions, [A1_SESSION, A2_SESSION, A3_SESSION]);
    assert_eq!(member.dispatch.parents, 9);
    assert_eq!(member.dispatch.children, 7);
    assert_eq!(member.dispatch.joined, 3);

    // Rejected membership never yields an attempt.
    for token in ["2", "3", "4", "5"] {
        let rejected = membership(&fixture.index, &fixture.store, token).expect("membership");
        assert_ne!(rejected.status, Status::Assigned, "token {token}");
        assert!(rejected.attempts.is_empty(), "token {token}");
        assert!(
            !rejected.dispatch.issues_for_token.is_empty(),
            "token {token} must disclose its rejection"
        );
    }
    let duplicate = membership(&fixture.index, &fixture.store, "2").expect("membership");
    assert!(duplicate
        .dispatch
        .issues_for_token
        .iter()
        .any(|issue| issue.kind == "duplicate_parent"));
    let competing = membership(&fixture.index, &fixture.store, "3").expect("membership");
    assert_eq!(competing.dispatch.claims_for_token, 2);
    assert!(competing
        .dispatch
        .issues_for_token
        .iter()
        .any(|issue| issue.kind == "competing_claims"));
    let missing = membership(&fixture.index, &fixture.store, "4").expect("membership");
    assert!(missing
        .dispatch
        .issues_for_token
        .iter()
        .any(|issue| issue.kind == "missing_child"));
    let mismatched = membership(&fixture.index, &fixture.store, "5").expect("membership");
    assert!(mismatched
        .dispatch
        .issues_for_token
        .iter()
        .any(|issue| issue.kind == "child_session_mismatch"));

    let untracked_file = (0..fixture.index.file_count)
        .find(|id| {
            fixture
                .index
                .file(*id)
                .expect("file")
                .path()
                .ends_with("Proj88Task1Eta1.jsonl")
        })
        .expect("untracked journal is indexed");
    assert!(member
        .attempts
        .iter()
        .all(|attempt| attempt.child_file_id != untracked_file));

    // ---- sequences stay inside one branch and one session -----------------
    let task = projected_task(&fixture, "1");
    assert_eq!(task.attempts.len(), 3);
    assert_eq!(task.attempts[0].sequences.len(), 3);
    assert_eq!(task.attempts[1].sequences.len(), 1);
    assert_eq!(task.attempts[2].sequences.len(), 2);
    assert_eq!(task.call_count(), 22);
    assert_eq!(task.attempts[0].call_count(), 9);

    let a1 = &task.attempts[0];
    let chain = &a1.sequences[0];
    assert_eq!(chain.calls.len(), 5);
    assert_eq!(chain.boundary_parent, NONE);
    let chain_tools: Vec<String> = signatures(chain, &fixture)
        .iter()
        .map(|sig| tool_of(sig).to_string())
        .collect();
    assert_eq!(chain_tools, ["ta", "tb", "tc", "td", "te"]);

    let fork = &a1.sequences[1];
    assert_eq!(fork.calls.len(), 2);
    // The fork starts at e4, whose parent is e2 (the fourth call in the chain).
    assert_eq!(fork.boundary_parent, chain.calls[3].event);
    assert_eq!(
        signatures(fork, &fixture)
            .iter()
            .map(|sig| tool_of(sig).to_string())
            .collect::<Vec<_>>(),
        ["tf", "tg"]
    );
    assert!(fork
        .calls
        .iter()
        .all(|call| !chain.calls.iter().any(|other| other.event == call.event)));

    let root = &a1.sequences[2];
    assert_eq!(root.calls.len(), 2);
    assert_eq!(root.boundary_parent, NONE);
    assert_eq!(
        signatures(root, &fixture)
            .iter()
            .map(|sig| tool_of(sig).to_string())
            .collect::<Vec<_>>(),
        ["th", "ti"]
    );
    for sequence in &a1.sequences {
        assert_eq!(sequence.file, a1.membership.child_file_id);
        assert!(sequence.calls.iter().all(|call| call.file == sequence.file));
        assert!(sequence
            .calls
            .iter()
            .all(|call| call.generation == fixture.published.generation));
    }

    // ---- projected shingles match the published writer rows --------------
    for k in 3..=5u32 {
        let tokens = chain.shingle_tokens(k, k);
        assert_eq!(tokens.len(), (chain.calls.len() as u32 - k + 1) as usize);
        for (offset, token) in tokens.iter().enumerate() {
            let first_event = chain.calls[offset].event;
            let last_event = chain.calls[offset + (k - 1) as usize].event;
            let rows = shingles_by_token(&fixture.published, token).expect("shingle rows");
            assert!(
                rows.iter().any(|row| row.k == k
                    && row.file == chain.file
                    && row.event == first_event
                    && row.end_event == last_event),
                "no published k={k} shingle row for the projected window at offset {offset}"
            );
        }
    }
    assert_eq!(chain.shingle_tokens(3, 5).len(), 6);

    // ---- families are deterministic and respect overlap ------------------
    let flat = task.flat_sequences();
    let report = families(&flat, &FamilyConfig::default()).expect("families");
    assert_eq!(report.rule_name, ase_motif::RULE_NAME);
    assert_eq!(report.rule_version, ase_motif::RULE_VERSION);
    assert_eq!(report.sequences, 6);
    assert_eq!(report.eligible_sequences, 4);
    assert_eq!(report.empty_sequences, vec![1, 2]);
    assert_eq!(report.pairs_compared, 6);
    let same = report.family_of(0).expect("chain family");
    assert_eq!(same.members, vec![0, 4]);
    assert_eq!(report.family_of(4), Some(same));
    assert_eq!(report.of_size(1).len(), 2);
    assert_eq!(report.singletons(), vec![3, 5]);
    assert!(report.family_of(0) != report.family_of(3));
    assert!(report.family_of(1).is_none());

    let repeated = families(&task.flat_sequences(), &FamilyConfig::default()).expect("families");
    assert_eq!(report.families, repeated.families);
    assert_eq!(report.pairs_compared, repeated.pairs_compared);

    let token_sets: Vec<std::collections::BTreeSet<String>> = flat
        .iter()
        .map(|sequence| sequence.shingle_tokens(3, 5).into_iter().collect())
        .collect();
    let overlap = ase_motif::family::jaccard(&token_sets[0], &token_sets[4]);
    assert!((overlap - 1.0).abs() < f64::EPSILON);
    let partial = ase_motif::family::jaccard(&token_sets[0], &token_sets[3]);
    assert!((partial - 1.0 / 3.0).abs() < 1e-9);

    // No membership means no sequence, even for a name that looks tracked.
    for token in ["2", "3", "4", "5"] {
        let empty = projected_task(&fixture, token);
        assert!(empty.attempts.is_empty(), "token {token}");
        assert_eq!(empty.call_count(), 0);
    }

    let _ = fs::remove_dir_all(&fixture.root);
}

#[test]
fn reopen_and_pinning_refuse_stale_evidence() {
    let fixture = build_fixture("reopen");
    let task = projected_task(&fixture, "1");
    let call = task.attempts[0].sequences[0].calls[0].clone();

    let sig = reopen_call(&fixture.published, &fixture.index, &fixture.store, &call)
        .expect("evidence reopens");
    assert_eq!(sig, "ta|flag=B,path=P");

    let mut stale_generation = call.clone();
    stale_generation.generation = "another-generation".to_string();
    assert!(reopen_call(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &stale_generation
    )
    .is_err());

    let mut moved_span = call.clone();
    moved_span.span.off += 1;
    assert!(reopen_call(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &moved_span
    )
    .is_err());

    let mut wrong_selector = call.clone();
    wrong_selector.selector = "block-v1|message.content[99]|toolCall".to_string();
    assert!(reopen_call(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &wrong_selector
    )
    .is_err());

    let mut tampered_digest = call.clone();
    let last = tampered_digest.block_hash.pop().expect("hash");
    tampered_digest.block_hash.push(if last == '0' { '1' } else { '0' });
    assert!(reopen_call(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &tampered_digest
    )
    .is_err());

    // Projection refuses an address or a version set that drifted from the
    // pinned generation.
    let member = membership(&fixture.index, &fixture.store, "1").expect("membership");
    let mut wrong_count = fixture.published.clone();
    wrong_count
        .manifest
        .counts
        .insert("events".to_string(), 0);
    assert!(project(&wrong_count, &fixture.index, &member).is_err());

    let mut wrong_version = fixture.published.clone();
    wrong_version
        .pointer
        .versions
        .insert("callsig".to_string(), "callsig-v0".to_string());
    assert!(project(&wrong_version, &fixture.index, &member).is_err());

    let _ = fs::remove_dir_all(&fixture.root);
}
