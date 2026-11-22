# Changelog

## [Unreleased]

## [0.3.0] - 2026-11-22

### Added

- Added the `ase-motif` library: generation-pinned CallSig sequences for the child sessions of a validated native dispatch membership, per-call reopenable evidence, and deterministic Jaccard families over `k=3..5` shingle tokens. Membership comes from `dispatch-v1` joins only; name, path and `parentSession` candidates are never membership inputs, and no sequence crosses a fork or session boundary.
- Added `ase motif` (rule `motif-alignment` / `motif-align-v1`) and `ase fixture-motif`: a bounded banded alignment keeps insertions and substitutions separate, support counts distinct tasks so repeated attempts of one task are one observation, `support_relaxed(g)` is monotonic in the gap allowance, and each motif reports its denominator and excluded tasks. A fixed-seed permutation null model reports `observed`/`expected`/`z`/`lift`, and every occurrence carries its generation, event, selector, span and digest so `ase open` can re-verify a single call.

### Changed

- Changed `ase-motif` from a library-only step to a CLI-visible motif layer; `ase motif` renders alignment, task-deduplicated support, denominator/exclusions and the null model, and no path calls a model.
- Changed `ase motif` cohort addressing: `PROJECT|TASK` addresses one locked `(project, task)` sampling key through the same dispatch validator, so a task token shared by two projects stays two independent samples; a bare token remains ambiguous and is never resolved by fallback.
- Changed motif counting to dedupe duplicate sequence text before statistics under rule `motif-sequence-text` / `seq-text-v1`. The `text_hash` is the SHA-256 of a sequence's reopened CallSig strings, deduped only within one resolved task key; duplicate copies are reported in `deduplicated_sequences`, excluded from family/null/sequence-denominator counting, and every occurrence remains reopenable evidence.

### Fixed

- Fixed the P0d-2 handoff gaps from the Sprint 2 differential review: cross-project task-token collisions are addressable without an alternate validator or ambiguous fallback, and the design §10.3 `text_hash` dedupe is implemented before counting instead of being waived.
- Fixed OMP tool-call evidence reopen: object-valued `arguments` now re-derives the same CallSig digest as indexing, so `ase open`, verification and motif calls stay verified; legacy `args` precedence and string arguments are preserved.
### Limitations

- P0d ablation ran 9 `τ/g` cells over 380 manually corroborated legacy task keys (979 attempts; 11,621 distinct task/text units); defaults remain `τ=0.50, g=2` (196 motifs, median support 3, median z 3.21, median lift 3.28, 162/196 motifs with z≥2). The cohort is manually corroborated legacy membership, not native `task_dispatch_v1`, and the 9-cell grid is descriptive over one frozen cohort; no full ablation rerun is claimed in this release.
- Offline motif evaluation does not extend the product resolver; no resolver, membership, digest or index-format change is made by the ablation.
- Object-valued OMP `arguments` reopen parity is fixed; legacy `args` precedence and string arguments are preserved, and the digest algorithm and index format are unchanged.

## [0.2.0] - TBD

> Release date is deferred until publication; this is the `v0.2.0` release candidate.

### Added

- Added a pinned OMP `v19.0.0` dispatch-tracking patch and Windows x64 integration notes under `support/omp/`.
- Added versioned `ase audit naming` (rule `file-name-grammar` / `naming-v1`): metadata-only coverage, clusters, ties and unparsed names from indexed file paths, plus `ase fixture-naming` synthetic fixtures. Attempt numbers stay labels; ambiguous splits abstain and no cross-project or cross-parent merge is made.
- Added versioned `ase audit delegation` (rule `parent-session-delegation` / `delegation-v1`) and `ase fixture-delegation`: indexed `parentSession` declarations become evidence-backed delegation edges only for a unique exact-id target, while missing, ambiguous and self targets stay unlinked without a file-order or path-proximity tie-break. Payload and path observations are kept separate, and a parent never produces a task key or an assignment.
- Added versioned `ase audit markers` (rule `task-id-marker` / `marker-v1`) and `ase fixture-markers`: `task_id=` markers are observed only in allowlisted user/assistant prose blocks of one published generation, reopened from the pinned raw span by versioned selector, and classified as assignment context, mention or ambiguous with cues retained. Multiple markers and repeated references stay separate occurrences; tool output, thinking and call-argument markers are excluded, output is metadata-only by default, and no task key or session-level assignment is produced.
- Added `resolve task:` task identity reconciliation (rule `task-identity-reconciliation` / `task-resolve-v2`) and `ase fixture-resolve`: naming, `parentSession`, `task_id` and native dispatch evidence are merged into `assigned`, `ambiguous` or `unresolved`, with per-candidate support/refute/context evidence labelled observed or derived and pinned to rule versions. Only one parsed project-scoped naming key or one unique consistent dispatch join can assign; markers, mentions, path hits and parents never assign alone, and the same token in different projects is never merged.
- Added native OMP dispatch provenance to `resolve task:` (rule `task-dispatch-provenance` / `dispatch-v1`) and `ase fixture-dispatch`: `task_dispatch_v1` parent records and `task_dispatch_child_v1` links are reopened from their indexed spans, checksum-verified and joined only on one exact runtime `dispatch_id` with matching containing session headers. The result names the executing child session, `project`/`task`/`attempt` and both reopenable spans. Malformed, missing, duplicated, wrong-session, mismatched, name-conflicting or competing claims assign nothing and are reported as rejected claims; untracked journals keep the previous behaviour. No index format change is required.

### Changed

- Changed `ase resolve task:` from literal path candidates to an evidence-backed `assigned` / `ambiguous` / `unresolved` decision; path hits and `parentSession` edges are now reported as context, and an unavailable marker layer is disclosed instead of being treated as no markers.

### Fixed

- Fixed address-index `source_type` and `tag` strings resolving inside the path/session string region instead of the type-string region. `show` no longer prints path fragments as a record type, and native `customType` tags such as `task_dispatch_v1` are readable. The type-string boundary is derived from the type table, so indexes written by older builds remain readable and the layout/version is unchanged.
- Fixed OMP child session headers with a JSON-escaped Windows `parentSession` path: the explicit session id and decoded parent path are now retained, so native dispatch links from those sessions resolve instead of being dropped as path-derived.

### Limitations

- Task identity is supported only for **new OMP sessions recorded with dispatch tracking** (`tracking` on `task` items). Sessions without tracking records remain unresolved; no membership is inferred from file names, prose, parent declarations or a shared numeric suffix.
- The frozen P0c holdout had **zero assignable members**: no membership could be assigned there. Those results are retained and disclosed; the synthetic P0c-10 batch does not replace the holdout.
- No historic-session reconstruction, precision/recall claim, or P0d claim is made in this release.
- The Windows x64 runtime requires the pinned patch plus the public `@oh-my-pi/pi-natives-win32-x64@19.0.0` addon; see `support/omp/README.md`. Other OMP versions are unsupported.
- CLI JSON and index-format compatibility across releases is not guaranteed; rebuild the index with `ase index --layer all` when it predates dispatch records.

## [0.1.0] - 2026-11-22

### Added

- Added layered `ase index --layer address|postings|all` with a validated atomic generation pointer for the derived search layer.
- Added `ase search` over contentless SQLite FTS5 prose postings, with metadata-only hits and exact event/selector mapping back to the pinned address.
- Added `ase open` to re-derive a hit's exact block from raw bytes and refuse to label edited raw as verified evidence.
- Added `ase shingle` for branch-bounded `k=3..5` CallSig shingle metadata with per-constituent evidence, and `ase bench-search` for deterministic lookup and evidence-reopen latency sampling.
- Added `ase fixture-p0b` synthetic fixtures for generation, allowlist, boundary and stale-evidence behavior checks.
- Added a P0a address-only indexer and CLI for OMP JSONL and Codex JSONL/zstd session sources.
- Added `ase show` with byte-span provenance, source-coordinate labels, freshness and optional raw content.
- Added `ase resolve` for source-backed session, branch and tool-call relations with `--explain` evidence.
- Added `ase verify --all` for full span re-verification and `ase bench-show` for latency sampling.
- Added synthetic fixtures for reproducible behavior checks.

### Changed

- Changed `ase open` to require the `--generation` value from the search hit and to reject a stale hit before reading raw source bytes.
- Changed the benchmark probe set to include a guaranteed no-hit term, a multi-token phrase and classified common/rare terms with observed aggregate counts; lookup is reported unrestricted and source filters apply only to reopened evidence.

### Fixed

- Fixed `ase index --help` and `ase index -h` to print usage and exit 0 without reading session sources or writing an index.

### Notes

- [Source-only CLI preview](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.1.0) for P0a/P0b. `resolve task:` returns path candidates only; P0c task identity is not included. No layer calls a model.
- Build with Rust 1.91.1 or newer: `cargo build --release --locked`. On Windows x64, the release commit indexed four synthetic OMP/Codex files (including JSONL.zst), reopened a search hit with both evidence hashes matching, and verified 24/24 spans.
- CLI JSON and index format compatibility across releases is not guaranteed. Rebuild from local sources with `ase index --layer all` if needed; an address-only rebuild disables search until postings are rebuilt.
- Search lookup p99 was **304.29 ms** in-process with a warm cache (3,000 queries, top-k 10), above the **<50 ms** target. See the [P0b report](artifacts/P0b/summary.md) for workload and limitations.
