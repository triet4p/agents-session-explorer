# P0a — Source + Decoder, address-only index, `show` / `resolve --explain`

- Task/attempt/agent: `P0a` / `A1` / `P0aFoundation`
- Role: silver implementation worker (light route, explicitly selected by the user)
- Scope: new P0a Rust implementation in this repository. `docs/INITIALIZATION.md` was read and preserved unchanged.
- Outcome: implemented and verified. All three published P0a gates measured and passing on the live corpus at the stated snapshot.

## 1. What was built

| Crate | Responsibility |
|---|---|
| `ase-core` | Span/file/event/session types, `Source`, `Decoder`, `RawStore` seams. No parsing, I/O, SQLite, model or network dependency. |
| `ase-ingest` | Recursive source discovery; OMP v3 JSONL decoder; Codex JSONL/zstd decoder; timestamp and FNV-1a helpers. |
| `ase-index` | Content-free binary address index writer/reader; per-file raw store (seek for JSONL, decoded-file cache for zstd). |
| `ase-cli` | `index`, `show`, `resolve`, `verify`, `bench-show`, `fixture`. Render only. |

Changed/added files: `Cargo.toml`, `README.md`, `CHANGELOG.md`, `.gitignore`, `.agents/memory/decisions.md`, `.agents/memory/lessons-learned.md`, the four crate trees above, `artifacts/P0a/summary.md`. No existing file was modified except adding the new root files; the initialization document is untouched.

Excluded by assignment: postings/FTS, shingle/motif, turn/episode, task resolver layers, judge, TUI, query-path model.

## 2. Observable contracts

- `SpanRef = (file_id, off, len)`. JSONL: raw file bytes. `.jsonl.zst`: decompressed-stream bytes (`coordinate` field in every result).
- Every non-empty line becomes one event. Unparseable **or** schema-drift lines are retained as `Unknown`/`<unparsed>`; no entry is dropped. `valid_json` is determined independently of typed parsing.
- Index stores only addresses, verbatim source labels (`source_type`, `tag`), branch edges (`parentId`), tool-call links (`toolCallId` / Codex `call_id`), session ids/parent references, freshness metadata and a non-cryptographic FNV-1a checksum. No decoded text, postings, motif or model output.
- `show` is metadata-only by default; `--content` explicitly reveals raw bytes. Results include encoding, coordinate system, ordinal, kind/role, parent/link, freshness and checksum verification; `--explain` adds the lookup steps.
- `resolve session:` / `resolve span:` are exhaustive over the indexed snapshot and expose only verbatim source relations with byte evidence; unresolved parent ids are reported, never inferred.
- `resolve task:<TOKEN>` is **partial and candidate-only**: a literal path-token hit is `candidate_path_match`, not task identity and not an assignment. P0c owns naming grammar, `parentSession`, `task_id` prose, structural continuation and semantic boundaries; P0a performs none of them.
- Stale files are reported via `freshness.status` and still served; stale is never an error.

## 3. Verification

### 3.1 Synthetic behavior checks (no real transcript data)

Commands (release binary, repository root):

```bash
ase fixture --out .tmp/fixtures
ase index --omp-root .tmp/fixtures/omp --codex-root .tmp/fixtures/codex --index .tmp/fixture.index --json
ase show session:syn-session-omp-1 --index .tmp/fixture.index --json --explain
ase show span:1:0:141 --index .tmp/fixture.index --json --explain --content
ase resolve session:syn-session-omp-1 --index .tmp/fixture.index --explain
ase resolve task:syn --index .tmp/fixture.index --json --explain
ase verify --all --index .tmp/fixture.index --json
```

Observed: 3 files (1 OMP JSONL, 1 Codex JSONL, 1 Codex `.jsonl.zst`), 22 events, 3 sessions, 0 unresolved call links, 0 unresolved parent refs, 1 deliberately malformed line preserved, 3 unknown-type events. `show` returned byte provenance, `coordinate=raw_file_bytes` for OMP and `coordinate=decompressed_stream_bytes` for zstd, `freshness=fresh`, and checksum match `true` for both. `resolve session:` returned an unresolved parent id, 5 `parentId` branch edges and 2 tool-call links, each with 1–2 byte-span evidence entries. `resolve task:syn` returned 3 `candidate_path_match` relations with `status=partial` and the P0c ownership note. `verify --all`: 22/22 spans ok, coverage 1.0, 0 failures.

Freshness probe: after `touch .tmp/fixtures/omp/synthetic.jsonl`, `show` reported `freshness.status=stale`; after regenerating the fixture and index it reported `fresh`.

### 3.2 Published P0a gates on the live corpus

Environment: Windows 10.0.26200 x64, 8 cores, rustc 1.91.1, release profile (`cargo build --release`), volume F:. No page-cache drop was performed. Index run 1 followed earlier structural corpus sampling (partially warm); run 2 immediately followed run 1 (warm). Benchmark and verification ran on the resulting index.

Exact corpus denominator at measurement (same snapshot as the final index):

| Source | Files | Bytes | Indexed events |
|---|---:|---:|---:|
| OMP JSONL | 3,873 | 6,896,241,246 | 1,328,052 |
| Codex (519 `.zst` + 9 `.jsonl`) | 528 | 580,590,763 | 493,072 |
| Total | 4,401 | 7,476,832,009 | 1,821,124 |

Index artifact size: 139,726,643 bytes. Decode summary at that snapshot: 0 invalid-JSON lines, 257,589 unknown-type events retained, 104,525 unresolved tool-call links, 0 unresolved parent refs, 48,361 multi-tool-call entries, 4,401 sessions.

#### Gate 1 — index ≤ 60 s

```bash
TIMEFORMAT='real_seconds=%R'; time ./target/release/ase.exe index \
  --omp-root "C:/Users/admin/.omp/agent/sessions" \
  --codex-root "C:/Users/admin/.codex/sessions" \
  --index .tmp/real.index --json
```

| Run | Internal `duration_ms` | Wall seconds | Result |
|---|---:|---:|---|
| 1 (partially warm) | 39,665 | 40.19 | PASS |
| 2 (warm) | 14,095 | 14.58 | PASS |

Both runs indexed OMP **and** Codex; the published gate targets ~6.17 GB OMP alone, while the measured OMP denominator was 6,896,241,246 bytes. Run 1 produced 1,821,081 events and run 2 produced 1,821,124 — the live append-only corpus grew between runs, so exact counts are quoted per run.

#### Gate 2 — `show` p99 < 20 ms

```bash
./target/release/ase.exe bench-show --n 2000 --seed 1 --index .tmp/real.index --json
```

Current implementation: exact pool **1,328,052 OMP events** (all OMP events in an index of 1,821,124 events), deterministic LCG seed 1, 2,000 samples. Each sample performs coordinate lookup + raw span seek/read + checksum, in-process (process startup excluded). Observed: p50 39.3 µs, p90 58.0 µs, **p99 109.8 µs**, max 172,276.2 µs (single outlier), 10,325,163 bytes read. PASS.

Pre-fix evidence (kept for transparency): the first implementation read the entire JSONL file per span and measured p50 3.388 ms, p90 11.627 ms, **p99 34.719 ms**, max 66.837 ms on the same index/pool/command — a credible challenge to the gate. The cause was my I/O path, not the target; `FsRawStore::span` was changed to seek directly to the span. The gate was not revised. Main was notified before the fix.

#### Gate 3 — 100% span verification

```bash
TIMEFORMAT='real_seconds=%R'; time ./target/release/ase.exe verify --all --index .tmp/real.index --json
```

Observed: 1,821,124 / 1,821,124 spans ok, 0 failed, coverage 1.0, 9,329,559,133 bytes re-read, 33,536 ms internal (33.688 s wall). Verification re-reads each exact span, checks its length and FNV-1a digest, and checks that raw JSON validity matches the recorded `valid_json` bit. PASS.

### 3.3 Measured vs unmeasured

Measured: all three published P0a gates; fixture behavior for both providers including zstd; stale/fresh detection.

Unmeasured by the published gates: Codex `show` latency (zstd requires whole-file decompression per uncached file), process-startup-inclusive CLI latency, TUI, and all P0b+ operations. No gate was revised. Index time and show latency are sensitive to page-cache state; conditions and exact denominators are recorded above.

## 4. Findings for the downstream researcher

Contracts to consume: spans are source-coordinate addresses, not text; zstd spans are stream-relative; `show` evidence includes checksum and freshness; `resolve` relations are verbatim source metadata with byte evidence. Use `verify --all` (or per-bundle checks in P4) before exporting any span.

Tradeoffs: one event per source line keeps ordinals and lookup simple but collapses multiple `toolCall` blocks in one assistant entry onto one event (48,361 such entries were counted; only the first call id is linked). Cross-file `toolCallId` matches are not resolved (104,525 unresolved links) because linking is per-file; parent ids that are not local events are preserved as external ids. The index is ~140 MB rather than the doc's ~48 MB because it stores 72-byte records with source labels and link fields; it remains content-free. FNV-1a is a consistency checksum, not content identity or a version key. `task:` results are deliberately candidates only.

Candidates not implemented (belong to later phases): postings/FTS and shingle index (P0b); task grammar clustering, `task_id` prose scan, structural continuation and semantic boundary (P0c); motif/alignment/null model (P0d); turn/episode and branch leaf-chase (P1); judge/assertions/export (P3/P4).

Mismatches between initialization and implementation/measurement:

1. Corpus drift: §3 quotes 3,533 OMP files / 6.171 GB / 1,199,311 entries; live measurement found 3,873 OMP files / 6,896,241,246 bytes / 1,328,052 OMP events (plus 528 Codex files). Codex file composition (519 zst + 9 jsonl) still matches.
2. Codex schema drift: §3.2 lists `session_meta|event_msg|response_item|item_completed|token_count` as top-level types; live files also contain `turn_context`, `token_usage_record`, `inter_agent_communication_metadata`, `world_state` and `compacted`, and `item_completed`/`token_count` frequently appear as payload types inside `response_item`. The decoder maps them to `Unknown` with verbatim tags rather than dropping them.
3. Index size estimate: §7.2's ~48 MB address table assumed a leaner record; preserving call links, source labels and exact spans yields ~140 MB for 1.82M events.
4. §9 lists `ase show ... [--turn a:b]`; P0a exposes `--ordinal A:B` instead because turns are P1. No turn semantics are claimed.

## 5. Housekeeping

- Access gaps: none. All 7 regular files under `C:/Users/admin/.agents/rules/` were read successfully; `implement-atomic-task`, `log-decision`, `log-lesson` and the evidence-review-cycle reference were read.
- Privacy: artifacts contain counts, commands, coordinates and dimensions only. No real transcript excerpt, session UUID, title, personal data or secret was printed into or retained in artifacts; behavioral content cases use synthetic fixtures.
- Repository has no `.git` directory and no commit or push was authorized or performed.
- No advisor consultation was needed or requested.
