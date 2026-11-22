//! Regression: OMP tool-call evidence must re-derive the writer's CallSig
//! digest for object-valued arguments, legacy `args`, string arguments and
//! empty argument fields. All tool names and paths here are synthetic.

use ase_core::{ProjectionKind, Provider, Role, SpanRef};
use ase_ingest::projection::{extract_block, project_omp_content};
use serde_json::value::RawValue;

const CONTENT: &str = r#"[
  {"type":"toolCall","id":"fix-c1","name":"Read.File","arguments":{"path":"C:/SYNTHETIC/report.txt","flag":true,"count":2,"nested":{"k":1},"items":[1,2]}},
  {"type":"toolCall","id":"fix-c2","name":"legacyTool","args":{"legacy":true}},
  {"type":"toolCall","id":"fix-c3","name":"precedenceTool","args":{"legacy":true},"arguments":"{\"modern\":2}"},
  {"type":"toolCall","id":"fix-c4","name":"stringTool","arguments":"{\"modern\":2}"},
  {"type":"toolCall","id":"fix-c5","name":"emptyTool"}
]"#;

#[test]
fn omp_tool_call_reopen_rederives_writer_digest_for_object_arguments() {
    let line = format!(
        r#"{{"type":"message","id":"fix-e1","timestamp":"2026-11-22T00:00:00.000Z","message":{{"role":"assistant","content":{CONTENT}}}}}"#
    );
    let content: &RawValue = serde_json::from_str(CONTENT).expect("valid raw content");
    let span = SpanRef {
        file: 9,
        off: 0,
        len: line.len() as u32,
    };
    let blocks = project_omp_content(4, span, content, Role::Assistant);
    assert_eq!(blocks.len(), 5, "each toolCall block is projected once");

    let expected = [
        "read.file|count=N,flag=B,items=A,nested=O,path=P",
        "legacytool|legacy=B",
        "precedencetool|legacy=B",
        "stringtool|arg=S",
        "emptytool|arg=Z",
    ];
    for (block, want) in blocks.iter().zip(expected) {
        assert_eq!(block.kind, ProjectionKind::Call);
        assert_eq!(block.call_sig, want);
        let reopened = extract_block(Provider::Omp, line.as_bytes(), &block.selector)
            .expect("toolCall selector re-derives from raw bytes");
        assert_eq!(reopened.kind, ProjectionKind::Call);
        assert_eq!(reopened.call_sig.as_deref(), Some(want));
        assert_eq!(reopened.block_hash, block.block_hash, "{}", block.selector);
    }

    // A populated argument shape must not collapse to the empty-argument digest.
    assert_ne!(blocks[0].block_hash, blocks[4].block_hash);
}
