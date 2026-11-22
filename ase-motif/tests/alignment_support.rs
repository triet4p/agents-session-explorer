//! P0d-2 synthetic fixture: bounded alignment, task-deduplicated support,
//! recorded denominator/exclusions, a deterministic null model and
//! per-occurrence evidence reopening.
//!
//! The fixture is synthetic only: tool names, session ids and dispatch ids are
//! invented, and no real transcript text is used. It builds a real address,
//! postings database, manifest and pointer, then runs the public library API
//! against that pinned generation.

use ase_core::{
    ALLOWLIST_VERSION, CALLSIG_VERSION, PROJECTION_VERSION, SHINGLE_VERSION, TOKENIZER_VERSION,
};
use ase_index::{decode_corpus, sha256_hex, write_address, FsRawStore, Index};
use ase_motif::align::{align, AlignConfig, AlignOp};
use ase_motif::motif::{detect, MotifConfig};
use ase_motif::{membership, project, reopen_call, TaskSequences};
use ase_postings::{
    publication::{self, ArtifactRef, FileWatermark, Manifest, Pointer},
    write_postings, Publication, WriterInput,
};
use ase_task::Status;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const M_A1: &str = "a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const M_A2: &str = "a2a2a2a2-a2a2-4a2a-8a2a-a2a2a2a2a2a2";
const M_B1: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";
const M_C1: &str = "c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1";
const M_E1: &str = "e1e1e1e1-e1e1-4e1e-8e1e-e1e1e1e1e1e1";

const PARENT_SESSION: &str = "p0d2-parent";

struct TaskSpec {
    dispatch_id: &'static str,
    task: &'static str,
    attempt: &'static str,
    session_id: &'static str,
    tools: &'static [&'static str],
}

// Task 1 repeats over two attempts (exact then one inserted call), task 2 is
// gapped-only, task 3 substitutes one column, and task 9 has a valid join but
// no call sequence at all.
const TASKS: &[TaskSpec] = &[
    TaskSpec {
        dispatch_id: M_A1,
        task: "1",
        attempt: "a1",
        session_id: "p0d2-child-1-a1",
        tools: &["t1", "t2", "t3", "t4"],
    },
    TaskSpec {
        dispatch_id: M_A2,
        task: "1",
        attempt: "a2",
        session_id: "p0d2-child-1-a2",
        tools: &["t1", "t2", "x1", "t3", "t4"],
    },
    TaskSpec {
        dispatch_id: M_B1,
        task: "2",
        attempt: "b1",
        session_id: "p0d2-child-2-b1",
        tools: &["t1", "t2", "y1", "t3", "t4"],
    },
    TaskSpec {
        dispatch_id: M_C1,
        task: "3",
        attempt: "c1",
        session_id: "p0d2-child-3-c1",
        tools: &["t1", "t2", "z1", "t4"],
    },
    TaskSpec {
        dispatch_id: M_E1,
        task: "9",
        attempt: "a1",
        session_id: "p0d2-child-9-a1",
        tools: &[],
    },
];

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
    base.join("p0d2-motif").join(name)
}

fn session_line(id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "session",
            "version": 3,
            "id": id,
            "timestamp": "2026-11-02T00:00:00.000Z",
            "cwd": "C:\\SYNTHETIC\\p0d2",
            "title": "synthetic P0d-2 fixture"
        })
    )
}

fn parent_line(spawn: u32, task: &TaskSpec) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": "2026-11-02T00:00:00.500Z",
            "customType": "task_dispatch_v1",
            "data": {
                "dispatch_id": task.dispatch_id,
                "parent_session_id": PARENT_SESSION,
                "parent_tool_call_id": format!("call-motif-{spawn}"),
                "spawn_index": spawn,
                "project_id": "P0D2",
                "task_id": task.task,
                "attempt_id": task.attempt
            }
        })
    )
}

fn child_line(dispatch_id: &str, session_id: &str) -> String {
    format!(
        "{}\n",
        json!({
            "type": "custom",
            "timestamp": "2026-11-02T00:00:00.750Z",
            "customType": "task_dispatch_child_v1",
            "data": { "dispatch_id": dispatch_id, "session_id": session_id }
        })
    )
}

fn turn_line(session_id: &str, ordinal: usize, parent: Option<&str>, tools: &[&str]) -> String {
    let id = format!("{session_id}-e{ordinal}");
    let mut content = vec![json!({
        "type": "text",
        "text": format!("synthetic motif turn {ordinal}")
    })];
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
        "timestamp": "2026-11-02T00:00:01.000Z",
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
    .map(|(key, value)| (key.to_string(), value.to_string()))
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

    let mut parent = session_line(PARENT_SESSION);
    for (spawn, task) in TASKS.iter().enumerate() {
        parent.push_str(&parent_line(spawn as u32, task));
    }
    write_file(&omp, "p0d2/parent.jsonl", &parent);

    for task in TASKS {
        let mut content = session_line(task.session_id);
        content.push_str(&child_line(task.dispatch_id, task.session_id));
        if task.tools.is_empty() {
            content.push_str(&format!(
                "{}\n",
                json!({
                    "type": "message",
                    "id": format!("{}-e1", task.session_id),
                    "parentId": serde_json::Value::Null,
                    "timestamp": "2026-11-02T00:00:01.000Z",
                    "message": {
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "no tool call here" }]
                    }
                })
            ));
        } else {
            let mut parent_id: Option<String> = None;
            for (ordinal, tool) in task.tools.iter().enumerate() {
                content.push_str(&turn_line(
                    task.session_id,
                    ordinal + 1,
                    parent_id.as_deref(),
                    &[*tool],
                ));
                parent_id = Some(format!("{}-e{}", task.session_id, ordinal + 1));
            }
        }
        write_file(
            &omp,
            &format!("p0d2/child-{}-{}.jsonl", task.task, task.attempt),
            &content,
        );
    }

    let corpus = decode_corpus(&omp, &codex).expect("decode");
    let generation = "p0d2-fixture-generation";
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
    .map(|(key, value)| (key.to_string(), value))
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

fn cohort(fixture: &Fixture) -> Vec<TaskSequences> {
    let mut tasks = Vec::new();
    for token in ["1", "2", "3", "9"] {
        let member = membership(&fixture.index, &fixture.store, token).expect("membership");
        assert_eq!(member.status, Status::Assigned, "token {token}");
        tasks.push(project(&fixture.published, &fixture.index, &member).expect("project"));
    }
    // An input unit with no admitted attempt must stay in the denominator.
    tasks.push(TaskSequences {
        token: "404".to_string(),
        task_key: None,
        attempts: Vec::new(),
    });
    tasks
}

fn synthetic_config() -> MotifConfig {
    let mut config = MotifConfig::default();
    // The fixture exists to exercise alignment, not family tuning: the family
    // threshold is a caller parameter, so it is lowered to keep exact, gapped
    // and near-miss sequences in one candidate family.
    config.family.tau = 0.0;
    config.null_permutations = 128;
    config
}

#[test]
fn alignment_keeps_insertion_and_substitution_separate() {
    let signature = |items: &[&str]| items.iter().map(|item| item.to_string()).collect::<Vec<_>>();
    let motif = signature(&["a", "b", "c", "d"]);
    let config = AlignConfig::default();

    let inserted = align(&motif, &signature(&["a", "b", "X", "c", "d"]), &config)
        .expect("alignment with one insertion");
    assert_eq!(inserted.matches, 4);
    assert_eq!(inserted.insertions, 1);
    assert_eq!(inserted.substitutions, 0);
    assert_eq!(inserted.deletions, 0);
    assert!(inserted
        .ops
        .contains(&AlignOp::Insertion { other: 2 }));
    assert_eq!(inserted.changes(), 1);

    let substituted = align(&motif, &signature(&["a", "b", "X", "d"]), &config)
        .expect("alignment with one substitution");
    assert_eq!(substituted.matches, 3);
    assert_eq!(substituted.insertions, 0);
    assert_eq!(substituted.substitutions, 1);
    assert_eq!(substituted.deletions, 0);
    assert_eq!(substituted.substituted_columns(), vec![2]);
    assert!(substituted
        .ops
        .contains(&AlignOp::Substitution { motif: 2, other: 2 }));

    // The band is a hard bound: a longer length difference is an error, not a
    // quietly truncated alignment.
    let narrow = AlignConfig { band: 1 };
    assert!(align(&motif, &signature(&["a", "b"]), &narrow).is_err());
    assert!(align(&motif, &signature(&[]), &config).is_err());
    assert!(align(&motif, &signature(&["a"]), &AlignConfig { band: 0 }).is_err());

    // Same inputs always produce the same operation list.
    let repeated = align(&motif, &signature(&["a", "b", "X", "c", "d"]), &config)
        .expect("repeat alignment");
    assert_eq!(inserted.ops, repeated.ops);
}

#[test]
fn support_dedupes_tasks_and_reopens_every_occurrence() {
    let fixture = build_fixture("support");
    let tasks = cohort(&fixture);
    let config = synthetic_config();
    let report = detect(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &tasks,
        &config,
    )
    .expect("motif report");

    // Denominator: 4 dispatch tasks plus the unmatched input unit.
    assert_eq!(report.tasks, 5);
    assert_eq!(report.sequences, 4);
    assert_eq!(report.families, 1);
    assert_eq!(report.motifs.len(), 1);
    let excluded: BTreeMap<&str, &str> = report
        .excluded_tasks
        .iter()
        .map(|exclusion| (exclusion.task.as_str(), exclusion.reason.label()))
        .collect();
    assert_eq!(excluded.get("P0D2|9"), Some(&"no_call_sequence"));
    assert_eq!(excluded.get("token:404"), Some(&"no_admitted_attempt"));

    let motif = &report.motifs[0];
    let expected: Vec<String> = ["t1", "t2", "t3", "t4"]
        .iter()
        .map(|tool| format!("{tool}|flag=B,path=P"))
        .collect();
    assert_eq!(motif.signatures, expected);
    assert_eq!(motif.reference_task, "P0D2|1");
    assert_eq!(motif.reference_attempt, "a1");
    assert!(motif.id.starts_with("mot-"));

    // Task 1 repeats over two attempts; the task is one observation.
    assert_eq!(motif.occurrences.len(), 3);
    let t1_attempts: Vec<(&str, u32)> = motif
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.task == "P0D2|1")
        .map(|occurrence| (occurrence.attempt.as_str(), occurrence.gap))
        .collect();
    assert_eq!(t1_attempts, [("a1", 0), ("a2", 1)]);
    assert_eq!(motif.support_exact, 1);
    // Gap allowance is monotonic: 1 exact task, then two with one skip each.
    assert_eq!(motif.support_relaxed, vec![1, 2, 2]);
    assert!(motif
        .support_relaxed
        .windows(2)
        .all(|window| window[0] <= window[1]));
    assert_eq!(motif.support_at(1), 2);
    assert_eq!(motif.support_at(9), 2);

    let denominator = &motif.denominator;
    assert_eq!(denominator.tasks_total, 5);
    assert_eq!(denominator.tasks_in_family, 3);
    assert_eq!(denominator.tasks_with_occurrence, 2);
    assert_eq!(denominator.sequences_in_family, 4);
    assert_eq!(denominator.sequences_with_occurrence, 3);
    let motif_excluded: BTreeMap<&str, &str> = denominator
        .excluded
        .iter()
        .map(|exclusion| (exclusion.task.as_str(), exclusion.reason.as_str()))
        .collect();
    assert_eq!(motif_excluded.get("P0D2|9"), Some(&"no_call_sequence"));
    assert_eq!(
        motif_excluded.get("token:404"),
        Some(&"no_admitted_attempt")
    );

    // One substituted column and no missing column: a near miss, not support.
    assert_eq!(motif.columns[2].matches, 3);
    assert_eq!(motif.columns[2].missing, 0);
    assert_eq!(
        motif.columns[2]
            .substitutions
            .get("z1|flag=B,path=P")
            .copied(),
        Some(1)
    );
    assert_eq!(motif.columns[3].matches, 4);
    assert_eq!(motif.columns[3].missing, 0);
    let near: Vec<&ase_motif::motif::Variant> = motif.near_misses().collect();
    assert_eq!(near.len(), 1);
    assert_eq!(near[0].task, "P0D2|3");
    assert_eq!(near[0].substitutions, 1);
    assert_eq!(near[0].insertions, 0);
    assert_eq!(near[0].deletions, 0);
    assert_eq!(near[0].substituted_columns, vec![2]);
    assert!(motif.sequence_exclusions.is_empty());

    // Null model is seeded and reproducible; it must yield lift or z.
    assert_eq!(motif.null.permutations, 128);
    assert_eq!(motif.null.observed, 1);
    assert!(motif.null.z.is_some() || motif.null.lift.is_some());

    let repeated = detect(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &tasks,
        &config,
    )
    .expect("repeat report");
    assert_eq!(repeated.motifs.len(), 1);
    assert_eq!(repeated.motifs[0].id, motif.id);
    assert_eq!(repeated.motifs[0].support_relaxed, motif.support_relaxed);
    assert_eq!(repeated.motifs[0].null.seed, motif.null.seed);
    assert_eq!(repeated.motifs[0].null.expected, motif.null.expected);
    assert_eq!(repeated.motifs[0].null.z, motif.null.z);
    assert_eq!(repeated.motifs[0].null.lift, motif.null.lift);

    // Every occurrence call reopens from raw bytes to the projected signature.
    for occurrence in &motif.occurrences {
        assert_eq!(occurrence.calls.len(), 4);
        for call in &occurrence.calls {
            let reopened = reopen_call(
                &fixture.published,
                &fixture.index,
                &fixture.store,
                &call.evidence,
            )
            .expect("occurrence evidence reopens");
            assert_eq!(reopened, call.signature);
            let mut stale = call.evidence.clone();
            stale.generation = "another-generation".to_string();
            assert!(reopen_call(
                &fixture.published,
                &fixture.index,
                &fixture.store,
                &stale
            )
            .is_err());
        }
    }

    // A wider gap ceiling cannot lower support.
    let mut wider = config;
    wider.max_gap = 4;
    let wider_report = detect(
        &fixture.published,
        &fixture.index,
        &fixture.store,
        &tasks,
        &wider,
    )
    .expect("wider report");
    let wider_motif = &wider_report.motifs[0];
    assert_eq!(&wider_motif.support_relaxed[..3], &motif.support_relaxed[..]);
    assert!(wider_motif
        .support_relaxed
        .windows(2)
        .all(|window| window[0] <= window[1]));

    // Reference boundary stays explicit: no sequence crossed a fork here.
    assert!(motif
        .occurrences
        .iter()
        .all(|occurrence| occurrence.dispatch_id != M_E1));

    let _ = fs::remove_dir_all(&fixture.root);
}
