# P0b — generation-pinned FTS5 postings, CallSig shingles, search and exact-block reopen

- Task/attempt/agent: `P0b` / `A1` / `P0bPostings`
- Role: silver implementation worker (light standalone route, user-selected; no gate/checkpoint/commit assigned)
- Scope: derived layer only. No P0c/P0d+, no motif family/alignment, no model, no second address backend.
- Outcome: implementation and correctness proofs complete; release measurements recorded, including a **published search p99 miss**. No criterion was revised.

## 1. What changed

| File | Change |
|---|---|
| `ase-core/src/lib.rs` | Shared versioned `BlockProjection` / `ProjectionKind` vocabulary and version constants; `FileDecode.projections`. |
| `ase-ingest/src/projection.rs` (new) | Allowlist per-block projection, `callsig-v1` normalization, digest, and selector-based re-derivation from raw. |
| `ase-ingest/src/{omp,codex}.rs` | Emit projections from the same bytes used for addresses; fill session/branch identity after assembly. |
| `ase-index/src/builder.rs`, `src/lib.rs` | One `decode_corpus` pass shared by `write_address` and postings; per-file strong digests. |
| `ase-postings/` (new crate) | Contentless FTS5 + `shingle-k3k5-v1` writer, publication manifest/pointer validation, read-only search/open/shingle access. |
| `ase-cli/src/main.rs` | `--layer address|postings|all`, `search`, generation-aware `open`, `shingle`, corrected `bench-search`, `fixture-p0b`; all address consumers resolve the pinned pointer when present. |
| `README.md`, `CHANGELOG.md`, `.agents/memory/decisions.md`, `.agents/memory/lessons-learned.md` | User-facing commands, decisions, and observed quirks. |

No dependency edge from query to a model was added. The query crates are `ase-cli → ase-postings → {ase-index, ase-ingest, ase-core}` plus SQLite/serde/sha2.

## 2. Observable contracts

- One address event per non-blank line is unchanged. A line may emit `0..n` blocks; each block has `(generation, event, versioned selector)` and a short digest. Two prose blocks on one line are two independent hits.
- FTS5 is `content=''`, `tokenize='unicode61 remove_diacritics 0'`. Only user/assistant prose enters FTS. Thinking, tool output, system and unknown blocks are excluded. No prose column, snippet or stored text exists in the derived DB.
- CallSig `callsig-v1` stores a normalized tool name plus sorted argument keys and value classes; no argument values.
- `shingle-k3k5-v1` emits ordinal `k=3..5` windows. A sequence may cross events only along a same-session explicit parent chain (`parentId` → last sequenced event). Fork siblings, roots, unresolved/out-of-domain parents and session changes start a new sequence; ambiguous parents are counted as abstentions. Each row stores each constituent's `(event, selector)` so P0d can reopen every item.
- Search is metadata-first: event, span, selector, role, bm25 score, encoding, freshness status, generation. Ranking remains FTS5 bm25 ascending with rowid tie-break; top-k is bounded and fixed. Production ranking was not changed.
- `open` requires the `--generation` value from the search hit and rejects a mismatch **before reading raw**. A stale event/selector cannot verify against a different publication, even if raw is unreadable.
- Publication: both `--layer all` and `--layer postings` copy the address into the immutable generation directory and the pointer references that copy, never the mutable `--index` path. When a valid pointer exists, all consumers resolve the pinned address. `--layer address` writes the legacy binary and tombstones the pointer, so postings cannot be served against a replaced address. Missing, tombstoned or mismatched sidecar yields `layer_unavailable` (exit 4), never zero hits.
- `open` reports `verified_against_index_snapshot` only when both span checksum and block digest match; edited raw is `unverified`. `--content` is explicit opt-in from raw.

## 3. Synthetic fixture proofs (no real transcript content)

Fixture generator: `ase fixture-p0b --out <synthetic dir>` writes one OMP JSONL, one Codex JSONL, one Codex `.jsonl.zst` and a second Codex JSONL with a repeated call id in another session. Latest fixture publication: 4 files, 24 events, 12 prose blocks, 10 call blocks, 6 shingles, 0 abstentions.

| Proof | Command shape (index path is synthetic) | Observed result |
|---|---|---|
| Two prose blocks, one line | `ase search <synthetic token A>` / `<synthetic token B>`; `ase open --generation G ...` | Two distinct selectors on the same event; each reopened with `block_digest_match=true`. |
| Unicode / escaped text | `ase search <unicode glyph>`; `ase open ... --content` | Hits on OMP escaped text and both Codex JSONL/zstd copies; escapes decoded identically at index and reopen time. |
| Excluded text | `ase search <synthetic excluded marker>` | Zero FTS hits; thinking and tool output absent from FTS and block tables. |
| Multi-call identity | `ase open --generation G --event 17 --selector ...toolCall` | Three distinct call selectors on one line, each verified from raw. |
| Cross-event shingle chain | `ase shingle --event 17` / `19` / `20` | Windows `k=3` (3), `k=4` (2), `k=5` (1) over 5 calls spanning events 17→19→20; each row lists its constituent event/selector pairs. All three constituents of a sample window reopened as `verified_against_index_snapshot`. |
| Fork and session boundary | `ase shingle --event 21` / `22`; repeated call id in another session/file | Two fork-sibling events produced zero shingles; no window joined forks, files or sessions. |
| Stale generation hit | Save G1 hit, edit synthetic prose, publish G2, `open --generation G1 ...` | `generation_mismatch`, no content field; still rejected when the raw file was temporarily unavailable. The same event/selector with `--generation G2` verified the new block. |
| Changed raw | Edit one synthetic prose token after build; `open --generation G ...`; `search <indexed token>` | `status=unverified`, span hash and block digest false, `freshness=stale`; search still returned the old hit with stale status. |
| Missing / mismatched sidecar | Move sidecar aside; separately replace its generation field | `layer_unavailable` with `sidecar_missing` / `generation_mismatch`; exit 4; `show` still read the address. |
| Interrupted pre-publication | `ase index --layer all ... --fail-before-publish --json` | Exit 2; pointer generation unchanged; owned staged files and the unreferenced generation directory were removed; no `.stage-*` files left. |
| Concurrent old reader | Start `bench-search` on G1, publish G2 during the run | Reader reported G1; the swap did not mutate a pinned generation. |
| Address-only → postings-only → address-only | `index --layer address`, `postings`, `address` with a G1 reader active | Reader stayed on G1; the postings pointer referenced `generations/.../address.bin`, not `--index`; final sidecar was tombstoned and `show` used the new legacy binary. Compatibility binary retained: 2,645 bytes. No staged files left; generation-directory count unchanged across a failed build. |
| Layer selection | Default `address`, then `--layer postings`, then `search` | Tombstoned sidecar after address-only; postings-ready publication and one hit after postings-only. |
| P0a compatibility | `ase fixture`; default-layer `index`; `show`; `resolve`; `verify --all`; `bench-show` | 3 files / 22 events / 3 sessions / 1 malformed / 3 unknown types; `resolve` returned 5 branch edges, 2 call links, 1 unresolved parent with byte evidence; `verify` 22/22. Counts match the published P0a fixture evidence. |

A latent P0a display issue was observed but not changed: `show` source type/tag strings can render as path fragments because event records use interner-local offsets for type strings. Counts, spans, hashes, relations and all P0b fields are unaffected; recorded as an unresolved finding, not silently patched.

## 4. Release measurement on the reachable local corpus

Environment: Windows 10.0.26200 x64, 8 cores, release profile, volume F:. Page cache was **not** dropped; all timings are warm-cache. No cold-start, peak-RSS or disk high-water-mark measurement was taken.

Snapshot denominator from the published manifest: **4,408 files / 7,501,530,912 source bytes / 1,824,008 events**. Build: 45,169 ms internal, 46.27 s wall. Blocks 672,426 (prose 135,116, calls 537,310); shingles 110,587; shingle abstentions 79,853.

| Artifact | Exact bytes | Decimal MB |
|---|---:|---:|
| Address (one immutable copy in active generation) | 139,946,142 | 139.95 |
| Postings-only SQLite (FTS + blocks + shingles + metadata) | 316,489,728 | 316.49 |
| Manifest | 1,457,927 | 1.46 |
| Atomic sidecar pointer | 1,334 | 0.0013 |
| Active published footprint (all four) | 457,895,131 | 457.90 |
| All persisted published bytes (one active generation; no retained old generation in this measurement) | 457,895,131 | 457.90 |

Staging before publish was address + postings + manifest = 457,893,797 bytes; generation copies temporarily double that set, so transient staging-plus-published bytes were about 915.79 decimal MB (derived from exact artifact sizes, not a measured high-water mark). Superseded generations are immutable and retained until an operator removes them; test-only generations were deleted before this final measurement. P0b builds do not remove existing user-owned legacy binaries; the final corpus measurement had no legacy binary at `--index` (0 bytes), while the synthetic address-only sequence retained a 2,645-byte compatibility copy.

### Latency (final corrected workload)

Sample policy: `ase bench-search --n 3000 --seed 7 --top-k 10 --json`. Query classes: a common function word, a **multi-token phrase** of common plus generic token, a deliberately low-frequency synthetic token, and a guaranteed-absent synthetic token. Query text is not retained in output. Deterministic LCG seed, 10 hits maximum per query, bm25 + rowid tie-break. Lookup is **unrestricted across sources**; `--source` selects which hit is reopened, it does not filter lookup. Evidence reopen runs every tenth query with an eligible hit and checks both span checksum and block digest. `FsRawStore` caches at most four decoded zstd files; JSONL seeks directly; publication digests and address load once before the loop.

Observed counts over 3,000 queries: query counts — common 958, phrase 470, rare 1,079, absent 493; hit counts — common 9,580, phrase 4,700, **rare 0 (a zero-hit/miss probe, not a low-nonzero probe)**, absent 0; aggregate 14,280. No query string or corpus-derived term is recorded.

| Series | Samples | p50 µs | p99 µs | max µs |
|---|---:|---:|---:|---:|
| Lookup, all sources (unrestricted) | 3,000 | 2,639.1 | 304,286.0 | 645,893.0 |
| Lookup, same workload with JSONL reopen selection | 3,000 | 1,505.1 | 212,827.2 | 458,120.2 |
| Lookup, same workload with zstd reopen selection | 3,000 | 1,148.5 | 117,569.1 | 167,832.4 |
| Evidence reopen, all (151 JSONL + 0 zstd) | 151 | 262.7 | 1,001.3 | 1,840.0 |
| Evidence reopen, JSONL | 151 | 188.3 | 564.5 | 828.2 |
| Evidence reopen, zstd | 151 | 16.1 | 3,600.9 | 10,079.1 |

All 151 reopen samples verified span and block (151/151 for each). The three lookup rows are the same unrestricted FTS workload; only the reopen sampling differs, so their spread reflects warm-cache variability across runs, not a source-filtered lookup.

Startup-inclusive CLI latency (separate process per sample; publication digest validation over ~458 MB plus address load on every start):

| Command | Samples | p50 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|
| `ase search` | 40 | 725.91 | 799.28 | 799.28 |
| `ase open` | 40 | 618.92 | 834.54 | 834.54 |
| `ase shingle --hash` | 30 | 710.61 | 851.59 | 851.59 |

These are warm-cache startup numbers, not the published in-process lookup gate and not cold measurements. Address lookup (`bench-show --n 2000 --seed 1`) on a 1,330,936-event OMP pool: p50 239.6 µs, p99 1,088.3 µs, max 2,133.7 µs.

### Gate interpretation and result (criteria unchanged)

- **Search p99: 304.29 ms — published <50 ms gate MISSED** under the corrected workload. No ranking, corpus, probe set or criterion was changed to recover it. The 2,639.1 µs p50 shows the tail is driven by high-frequency matches, not by every query. Startup-inclusive p99 799.28 ms is reported separately.
- Index ≤500 MB decimal: postings-only **316.49 MB**; active full published footprint **457.90 MB**. Both are numerically below 500 MB, but `docs/SCOPE.md` requires owner agreement on the definition of "index", so no formal gate pass is claimed.
- Superseded observations preserved for transparency: an earlier benchmark using literal placeholder tokens measured lookup p99 8.63 ms with different hit patterns; an earlier shingle schema measured postings 300.54 MB and footprint 441.90 MB; a discarded design kept a duplicate compatibility address and would have persisted 581.72 MB. None of these replaces the final numbers.

## 5. Backend tradeoff, diagnosis and next steps

Chosen: binary P0a address + contentless SQLite FTS5/shingle sidecar + one validated atomic pointer. Alternatives considered without implementation: one SQLite database holding addresses and postings; a bespoke postings format. A single SQLite would share a commit and simplify joins, but would require migrating and re-measuring every published P0a gate on the new backend, and a DB transaction cannot freeze the source files. The sidecar keeps the measured address path intact and permits independent rebuild/invalidation; its cost is startup digest validation and careful single-pointer publication. No second backend was implemented for comparison.

Gate-tail diagnosis (read-only): the common probe's FTS posting list contains 48,624 matching rows; the multi-token phrase matches 632; the rare and absent probes match 0. FTS5 must score every matching row before `ORDER BY rank LIMIT` can select top-k, so bm25 work scales with the common posting list. Options recorded, **none applied to production**: (a) keep relevance semantics and tune scoring/indexing (e.g., block-max/WAND-style skipping or a documented stopword policy) and re-measure on the same workload; (b) rowid ordering would be a different, non-relevance semantics and was not used; (c) `ORDER BY rank LIMIT` is the current bm25 path and was not substituted with a cheaper query. Owner decision needed on workload definition and optimization budget; the gate remains missed.

## 6. Limitations and housekeeping

- Shingle support follows explicit parent chains; fork-sibling, root and ambiguous-parent regions are abstained. Cross-turn continuation, task-level counts, family/alignment and null models remain P0d work.
- Query terms are quoted before FTS MATCH; unicode61 tokenization is the shared rule. Phrase quality across languages was not evaluated.
- Cold page-cache conditions, peak RSS, disk high-water mark and multi-process readers beyond the one fixture test were not measured. The rare probe is a zero-hit miss, not a genuinely low-nonzero probe.
- Recovery limits: owned `.stage-<nonce>-*` files and the unreferenced generation directory are removed on the success and injected-failure paths. An abrupt crash or an unexpected error after staging can leave stale staging files or unreferenced generation directories; there is no automatic GC in P0b and an operator must remove them. User-owned legacy binaries are never removed by P0b.
- All seven regular files under `C:/Users/admin/.agents/rules/` were read. Skills `implement-atomic-task`, `log-decision`, `log-lesson` were read. `sandbox_permissions: require_escalated` from shell rules is not an available field in this toolset; no external-environment command was needed. No Git operation, commit or push was authorized or performed.
- Privacy: this artifact contains counts, commands, version names, structural selectors and dimensions only. No real transcript text, corpus-derived term, real path, session id or secret was printed into or retained in an artifact. Behavioral cases use synthetic fixtures; release evidence is aggregate only.
