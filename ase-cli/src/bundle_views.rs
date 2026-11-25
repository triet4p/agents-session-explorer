//! Deterministic markdown views inside an exported bundle.
//!
//! `evidence.md` and `questions.md` are rendered only from the motif record,
//! so the exporter can hash the rendered bytes and the verifier can re-render
//! the same record and compare byte for byte.

use serde_json::Value;

fn synthetic_line(value: &Value) -> &'static str {
    if value.get("synthetic").and_then(Value::as_bool) == Some(true) {
        "synthetic: true\n\n"
    } else {
        ""
    }
}

pub(crate) fn render_evidence_view(motif: &Value) -> String {
    let mut out = format!("# Evidence — {}\n\n", motif.get("motif_id").and_then(Value::as_str).unwrap_or(""));
    out.push_str(synthetic_line(motif));
    let reps = motif.get("representatives").and_then(Value::as_array).cloned().unwrap_or_default();
    for rep in reps {
        out.push_str(&format!(
            "## {} — selection={}\n",
            rep.get("occurrence_id").and_then(Value::as_str).unwrap_or(""),
            rep.get("selection").and_then(Value::as_str).unwrap_or(""),
        ));
        out.push_str(&format!("source_alias: {}\n", rep.get("source_alias").and_then(Value::as_str).unwrap_or("")));
        out.push_str(&format!("input_identity: sha256:{}\n\n", rep.get("input_identity").and_then(Value::as_str).unwrap_or("")));
        let rows = rep.get("evidence").and_then(Value::as_array).cloned().unwrap_or_default();
        for row in rows {
            let result = row.get("result").unwrap_or(&Value::Null);
            out.push_str(&format!("### call {}\n", row.get("call_ordinal").and_then(Value::as_u64).unwrap_or(0)));
            out.push_str(&format!("selector: {}\n", row.get("call").and_then(|c| c.get("selector")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("block_sha256: {}\n", row.get("call").and_then(|c| c.get("block_sha256")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("tool: {}\n", row.get("call").and_then(|c| c.get("tool")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("signature: {}\n", row.get("call").and_then(|c| c.get("signature")).and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_status: {}\n", result.get("status").and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_selector: {}\n", result.get("selector").and_then(Value::as_str).unwrap_or("")));
            out.push_str(&format!("result_block_sha256: {}\n", result.get("block_sha256").and_then(Value::as_str).unwrap_or("")));
            out.push_str("excerpt:\n");
            for line in row.get("excerpt").and_then(Value::as_str).unwrap_or("").split('\n') {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }
    out
}

pub(crate) fn render_questions_view(motif: &Value) -> String {
    let mut out = format!("# Questions — {}\n\n", motif.get("motif_id").and_then(Value::as_str).unwrap_or(""));
    out.push_str(synthetic_line(motif));
    if let Some(questions) = motif.get("questions").and_then(Value::as_array) {
        for q in questions {
            out.push_str(&format!(
                "{}. {}\n",
                q.get("question_id").and_then(Value::as_str).unwrap_or(""),
                q.get("text").and_then(Value::as_str).unwrap_or(""),
            ));
        }
    }
    out
}
