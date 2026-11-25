//! Synthetic journal fixtures for the generic index and address corpus.
//!
//! All identifiers and prose are synthetic, and files are written only below
//! the caller's output path.

use crate::cli_option::opt;
use ase_core::{err, Result};
use std::path::PathBuf;


const SYNTHETIC_OMP: &str = r#"{"type":"session","version":3,"id":"syn-session-omp-1","timestamp":"2026-01-02T03:04:05.000Z","cwd":"C:\\SYNTHETIC\\proj","title":"synthetic","parentSession":"syn-session-omp-parent"}
{"type":"message","id":"syn-e1","parentId":null,"timestamp":"2026-01-02T03:04:06.000Z","message":{"role":"user","content":[{"type":"text","text":"SYNTHETIC USER TOKEN"}]}}
{"type":"message","id":"syn-e2","parentId":"syn-e1","timestamp":"2026-01-02T03:04:07.000Z","message":{"role":"assistant","content":[{"type":"text","text":"SYNTHETIC ASSISTANT TOKEN"},{"type":"toolCall","id":"syn-call-1","name":"read","args":{"path":"C:\\SYNTHETIC\\file.txt"}}]}}
{"type":"custom","customType":"tool_execution_start","data":{"toolCallId":"syn-call-1","toolName":"read","args":{"path":"C:\\SYNTHETIC\\file.txt"}},"id":"syn-e3","parentId":"syn-e2","timestamp":"2026-01-02T03:04:08.000Z"}
{"type":"message","id":"syn-e4","parentId":"syn-e2","timestamp":"2026-01-02T03:04:09.000Z","message":{"role":"toolResult","toolCallId":"syn-call-1","toolName":"read","content":[{"type":"text","text":"SYNTHETIC TOOL OUTPUT"}]}}
{"type":"model_change","id":"syn-e5","parentId":"syn-e4","model":"synthetic/model","timestamp":"2026-01-02T03:04:10.000Z"}
{"type":"future_entry_type","id":"syn-e6","parentId":"syn-e5","timestamp":"2026-01-02T03:04:11.000Z","note":"SYNTHETIC"}
{this line is deliberately not json}
"#;

const SYNTHETIC_CODEX: &str = r#"{"timestamp":"2026-01-02T03:04:05.000Z","ordinal":0,"type":"session_meta","payload":{"id":"syn-session-codex-1","cwd":"C:\\SYNTHETIC\\proj"}}
{"timestamp":"2026-01-02T03:04:06.000Z","ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"SYNTHETIC USER TOKEN"}}
{"timestamp":"2026-01-02T03:04:07.000Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"text","text":"SYNTHETIC ASSISTANT TOKEN"}]}}
{"timestamp":"2026-01-02T03:04:08.000Z","ordinal":3,"type":"response_item","payload":{"type":"function_call","call_id":"syn-call-c1","name":"read","arguments":"{}"}}
{"timestamp":"2026-01-02T03:04:09.000Z","ordinal":4,"type":"response_item","payload":{"type":"function_call_output","call_id":"syn-call-c1","output":"SYNTHETIC OUTPUT"}}
{"timestamp":"2026-01-02T03:04:10.000Z","ordinal":5,"type":"token_count","payload":{"total_token_usage":{"input_tokens":1}}}
{"timestamp":"2026-01-02T03:04:11.000Z","ordinal":6,"type":"future_type","payload":{"note":"SYNTHETIC"}}
"#;

pub fn run(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    std::fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    std::fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    std::fs::write(omp.join("synthetic.jsonl"), SYNTHETIC_OMP)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let cb = SYNTHETIC_CODEX.as_bytes();
    std::fs::write(codex.join("synthetic.jsonl"), cb)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(cb), 3)
        .map_err(|e| err(format!("zstd fixture: {e}")))?;
    std::fs::write(codex.join("synthetic.jsonl.zst"), compressed)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    println!("wrote synthetic fixtures to {}", out.display());
    Ok(())
}

const ADDRESS_OMP: &str = r#"{"type":"session","version":3,"id":"addr-session-omp","timestamp":"2026-02-03T04:05:06.000Z","cwd":"C:\\SYNTHETIC\\addr","title":"synthetic"}
{"type":"message","id":"addr-e1","parentId":null,"timestamp":"2026-02-03T04:05:07.000Z","message":{"role":"user","content":[{"type":"text","text":"ADDRESS PROSE ALPHA \u00e9\u2603"},{"type":"text","text":"ADDRESS PROSE BETA"}]}}
{"type":"message","id":"addr-e2","parentId":"addr-e1","timestamp":"2026-02-03T04:05:08.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"ADDRESS EXCLUDED THINKING"},{"type":"text","text":"ADDRESS ASSISTANT GAMMA"},{"type":"toolCall","id":"addr-call-a","name":"alphaTool","args":{"path":"synthetic/a.txt","flag":true}},{"type":"toolCall","id":"addr-call-b","name":"betaTool","args":{"path":"synthetic/b.txt","count":2}},{"type":"toolCall","id":"addr-call-c","name":"gammaTool","args":{}}]}}
{"type":"message","id":"addr-e3","parentId":"addr-e1","timestamp":"2026-02-03T04:05:09.000Z","message":{"role":"toolResult","toolCallId":"addr-call-a","content":[{"type":"text","text":"ADDRESS EXCLUDED TOOL OUTPUT"}]}}
{"type":"message","id":"addr-e5","parentId":"addr-e2","timestamp":"2026-02-03T04:05:10.000Z","message":{"role":"assistant","content":[{"type":"text","text":"ADDRESS LINEAR DELTA"},{"type":"toolCall","id":"addr-call-e","name":"epsilonTool","args":{}}]}}
{"type":"message","id":"addr-e6","parentId":"addr-e5","timestamp":"2026-02-03T04:05:11.000Z","message":{"role":"assistant","content":[{"type":"text","text":"ADDRESS LINEAR ETA"},{"type":"toolCall","id":"addr-call-f","name":"zetaTool","args":{}}]}}
{"type":"message","id":"addr-fork-a","parentId":"addr-e2","timestamp":"2026-02-03T04:05:12.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"addr-call-ga","name":"forkATool","args":{}}]}}
{"type":"message","id":"addr-fork-b","parentId":"addr-e2","timestamp":"2026-02-03T04:05:13.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"addr-call-gb","name":"forkBTool","args":{}}]}}
{this line is deliberately not json}
"#;

const ADDRESS_CODEX: &str = r#"{"timestamp":"2026-02-03T04:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"addr-session-codex","cwd":"C:\\SYNTHETIC\\addr"}}
{"timestamp":"2026-02-03T04:05:07.000Z","ordinal":1,"type":"event_msg","payload":{"type":"user_message","message":"ADDRESS CODEX EPSILON \u00e9"}}
{"timestamp":"2026-02-03T04:05:08.000Z","ordinal":2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ADDRESS CODEX ZETA"},{"type":"output_text","text":"ADDRESS CODEX ETA"}]}}
{"timestamp":"2026-02-03T04:05:09.000Z","ordinal":3,"type":"response_item","payload":{"type":"function_call","call_id":"addr-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/x.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T04:05:10.000Z","ordinal":4,"type":"response_item","payload":{"type":"function_call_output","call_id":"addr-call-x","output":"ADDRESS EXCLUDED TOOL OUTPUT"}}
{"timestamp":"2026-02-03T04:05:11.000Z","ordinal":5,"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"ADDRESS EXCLUDED THINKING"}]}}
"#;

const ADDRESS_CODEX_SECOND: &str = r#"{"timestamp":"2026-02-03T05:05:06.000Z","ordinal":0,"type":"session_meta","payload":{"id":"addr-session-codex-2","cwd":"C:\\SYNTHETIC\\addr"}}
{"timestamp":"2026-02-03T05:05:07.000Z","ordinal":1,"type":"response_item","payload":{"type":"function_call","call_id":"addr-call-x","name":"alphaTool","arguments":"{\"path\":\"synthetic/y.txt\",\"flag\":true}"}}
{"timestamp":"2026-02-03T05:05:08.000Z","ordinal":2,"type":"event_msg","payload":{"type":"user_message","message":"ADDRESS OTHER SESSION THETA"}}
"#;

pub fn run_address(args: &[String]) -> Result<()> {
    let out = PathBuf::from(opt(args, "--out").ok_or_else(|| err("fixture-address needs --out DIR"))?);
    let omp = out.join("omp");
    let codex = out.join("codex");
    std::fs::create_dir_all(&omp).map_err(|e| err(format!("create {}: {e}", omp.display())))?;
    std::fs::create_dir_all(&codex).map_err(|e| err(format!("create {}: {e}", codex.display())))?;
    std::fs::write(omp.join("synthetic-addr.jsonl"), ADDRESS_OMP)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let codex_bytes = ADDRESS_CODEX.as_bytes();
    std::fs::write(codex.join("synthetic-addr.jsonl"), codex_bytes)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(codex_bytes), 3)
        .map_err(|e| err(format!("zstd fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-addr.jsonl.zst"), compressed)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    std::fs::write(codex.join("synthetic-addr-second.jsonl"), ADDRESS_CODEX_SECOND)
        .map_err(|e| err(format!("write fixture: {e}")))?;
    println!("wrote synthetic address fixtures to {}", out.display());
    Ok(())
}
