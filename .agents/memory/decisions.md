# Decisions

## [2026-11-20] Use a custom binary address index for P0a instead of SQLite/FTS

**Decision:** P0a stores a compact little-endian binary address table (file, ordinal, source offset, length, kind, role, source type/tag, branch/link indices, checksum, path/session strings) with offset-addressed reads. No SQLite, FTS5 or postings in P0a.

**Alternatives considered:** SQLite with an events table; reusing omp `stats.db` or codex `thread_history_1.sqlite`; a postings-backed store.

**Reason:** The P0a contract is address-only and content-free (INITIALIZATION.md §7.2/§7.4). A flat binary table gives deterministic O(log n) coordinate lookup with no query-time hash map, so process start and `show` stay cheap; external indices are explicitly forbidden as sources of truth (§1 non-goals). Postings are P0b and must not leak into P0a.

**Consequences:** Later phases can project events into SQLite without changing raw provenance; any change to record layout requires an index rebuild (build id is stored in the header). Type/tag strings are stored in a shared blob addressed by offsets, so readers must add the blob base.

## [2026-11-20] Seek per span for JSONL; cache decoded bytes only for zstd

**Decision:** `FsRawStore::span` opens and seeks directly to `(off, len)` for JSONL; for zstd it decodes the file once and slices the decompressed stream from a small LRU. Bulk `verify` uses a whole-file cached path.

**Alternatives considered:** Reading the whole JSONL file for every span; caching every decoded JSONL file; storing compressed offsets for zstd.

**Reason:** A 2,000-sample real-corpus benchmark with whole-file reads measured p99 show 34.719 ms, above the published <20 ms gate. Seek-based reads measured p99 0.110 ms on the same index. zstd offsets are stream-relative (§6.1), so compressed offsets cannot be stable addresses.

**Consequences:** `show` latency is dominated by file open/seek, not parsing; zstd `show` remains a separate, un-gated cost. Verification of 1.8M spans stays efficient because it reuses one decoded file at a time.

## [2026-11-20] Keep `resolve task:` candidate-only in P0a

**Decision:** `ase resolve task:<TOKEN>` reports literal source-path token hits as `candidate_path_match` with status `partial`, explicitly stating that P0c owns task identity.

**Alternatives considered:** Deriving task keys from filename grammar; scanning prose for `task_id=`; inferring continuation edges.

**Reason:** INITIALIZATION.md §8 assigns naming grammar, parentSession, task_id text and structural/semantic resolution to P0c. Implementing them in P0a would create two owners and overstate evidence.

**Consequences:** Researchers can inspect path candidates and their byte evidence without mistaking them for established tasks; P0c must consume these as candidates and add its own versioned assertions.


## [2026-11-20] Keep the P0a binary address and add a separate contentless FTS5 sidecar

**Decision:** P0b publishes an immutable binary address plus a separate SQLite FTS5 `content=''` postings/shingle file, gated by one atomically replaced pointer sidecar with artifact digests, a generation id and version manifest. Search opens only a fully validated generation; missing or mismatched sidecars return `layer_unavailable` while the address stays usable.

**Alternatives considered:** migrating addresses into one SQLite database (transactional consistency but rebuilds and re-measures every published P0a gate); a bespoke postings format (would require own tokenizer/ranker/versioning); allowing search to serve hits from a mismatched pair (rejected as false evidence).

**Reason:** the P0a address format already has measured lookup/verify evidence and no query-time hash map. Rebuilding it into SQLite would discard that evidence and add a migration without a measured benefit here. A single pointer is the one enable switch for search, so an interrupted build cannot silently join layers from different byte snapshots. Contentless FTS5 stores tokens/postings but no prose column, matching the privacy contract; snippets must be reopened from raw.

**Consequences:** two artifacts must be published together (address copied into the generation directory, pointer replaced atomically); startup validates digests; shingle/FTS layers can be rebuilt without changing raw or address semantics. A crash between address replacement and pointer replacement is detected as a digest/shape mismatch, never as a stale hit set. Do not switch to a single SQLite backend without re-running the published P0a address/show/verify gates on that exact backend.

## [2026-11-20] Restrict CallSig shingles to one address event

**Decision:** `shingle-k2k3-v1` emits ordinal shingles only inside one source event/line, keyed by generation, file, session-local id and branch parent; no sequence crosses a line, file, session or branch edge.

**Alternatives considered:** concatenating calls across a session by ordinal; joining by repeated `call_id` across files; deriving structural continuation edges.

**Reason:** turn/boundary and task semantics are unassigned or explicitly later phases (P0c/P1/P0d). Concatenating would silently invent joins across branches or sessions; repeated ids across files are not evidence of a shared call namespace. Per-event sequences preserve multiple calls on one line and give P0a a safe, versioned shingle substrate.

**Consequences:** P0d must define an explicit, validated boundary rule before extending support beyond one line; `support_exact` over these shingles cannot yet represent cross-turn procedures. Each hit carries its own event and selector, so independent identity is preserved.

## [2026-11-20] Pin the address inside the generation; never search a mutable compatibility binary

**Decision:** P0b `index --layer all|postings` copies the address into the generation directory and the pointer references that immutable copy. The `--index` path is only the sidecar anchor and legacy standalone address input; when a valid pointer exists, `show`, `resolve`, `verify`, `search`, `open` and `shingle` all resolve the pinned address through the pointer. An address-only rebuild writes/overwrites the legacy binary and tombstones the pointer, so postings are never served against a replaced address.

**Alternatives considered:** pointing the manifest at the mutable `--index` binary and relying on digest mismatch (rejected: a failed or address-only build could change address consumers while old postings are still active); keeping a compatibility copy at `--index` in addition to the generation address (rejected: duplicated 139.9 MB and a mutable second address).

**Reason:** the address-only rebuild and interrupted-second-build cases must not change what an already-published generation serves. A single atomic pointer swap plus immutable per-generation address gives one trust boundary; consumers pin it for the whole process. Verified on synthetic fixture: a reader started before a second publish kept the old generation, an interrupted build left the old pointer/generation intact, and an address-only rebuild tombstoned search while `show` used the new legacy address.

**Consequences:** superseded generation directories are immutable and retained until an operator removes them; active bytes and retained bytes must be reported separately. Legacy `--index` binaries without a pointer remain readable but have no search layer. No digest or version check was weakened to reduce footprint.

## [2026-11-20] Continue CallSig shingles along explicit parent chains, k=3..5 with constituent evidence

**Decision:** `shingle-k3k5-v1` emits ordinal `k=3..5` windows over call blocks. A sequence may cross source events only when the next event is a direct child (`parentId`/`parent_idx`) of the last sequenced event in the same file and session. Fork siblings, roots, unresolved/out-of-domain parents and session changes start a new sequence and are counted as abstentions. Every row stores the event and selector of each constituent.

**Alternatives considered:** the earlier per-event-only rule (safe but unusable for P0d multi-call windows); ordinal concatenation within a session (would silently join forks); id-based cross-file joins (unsupported namespace).

**Reason:** P0d §10.2 needs k=3..5 windows over call sequences, and a fork edge is an explicit branch boundary. The parent chain is the only available in-snapshot evidence for linear continuation; anything weaker would fabricate a path through a fork. Storing constituents lets a later motif step reopen each call as evidence without storing prose.

**Consequences:** support across-turn procedures only when parent links actually form a chain; ambiguous areas are absent from shingle support and reported as abstentions. P0d must still define family/alignment and null models; this change adds no mining.

## [2026-11-20] Require the hit generation when reopening evidence

**Decision:** `ase open` requires `--generation` copied from the search hit and refuses a mismatch before opening raw bytes.

**Alternatives considered:** trusting event index plus selector alone (reuses identity across publications); inferring generation from the active pointer only (would verify an old hit against new text).

**Reason:** event indices and selectors are generation-local; a saved hit from G1 can point at different text in G2. Requiring the generation token makes the trust boundary explicit and lets the command reject stale hits even when raw is unreadable.

**Consequences:** callers must carry the generation with every hit; no extra text or state is stored in the query layer. This does not weaken publication validation.

## [2026-11-20] Accept P0b on the shared worker without changing the latency target

**Decision:** The project owner accepts the current P0b result despite the measured search p99 exceeding 50 ms.

**Alternatives considered:** Hold P0b acceptance until the p99 target is demonstrated.

**Reason:** The worker is not always idle. The available run is not an isolated latency baseline; the effect of background load was not measured separately.

**Consequences:** Keep the original measurements and the 50 ms target. Do not report a measured pass or treat the miss as blocking P0b acceptance.

## [2026-11-25] Tách CLI theo nhóm lệnh, không đổi hợp đồng

**Decision:** Giữ parser và router hiện tại, chuyển từng nhóm lệnh sang module theo nghiệp vụ, dùng điểm vào `run(args)` đã có và giữ `main.rs` chỉ điều phối.

**Alternatives considered:** Giữ nguyên `main.rs` và tiếp tục thêm lệnh; chuyển đồng loạt sang một framework CLI; chia file theo số dòng.

**Reason:** `main.rs` đang giữ cả vận hành bằng chứng, fixture và điều phối LLM; các nhóm này có vòng đời kiểm thử khác nhau. Di chuyển theo nhóm cho phép so từng bề mặt CLI với baseline mà không đưa thêm biến số từ parser hay schema.

**Consequences:** Refactor không đổi help, exit code, output, schema hay cache identity. Xác minh LLM offline là mặc định; ngân sách provider thật và điều kiện ngoại lệ thuộc [Sprint 8](../../docs/sprint-plans/sprint-8.md).

## [2026-11-27] Apply the MIT license to the current tree

**Decision:** The project owner chose MIT for the current source tree. `LICENSE` names Le Minh Triet as copyright holder with year 2026, and the workspace manifest sets `license = "MIT"` with every member crate inheriting it. The `v0.7.0` tag and its published archive stay unchanged, and `THIRD_PARTY_NOTICES.txt` is not modified.

**Basis:** Explicit project-owner decision recorded 2026-11-27. Holder and year are taken from repository history: all commits are authored by Le Minh Triet in 2026. No other copyright holder is asserted.

**Consequences:** README, CHANGELOG and current status documentation describe the MIT grant for the current tree while retaining the historical no-license facts for `v0.7.0`. Third-party notices continue to apply to bundled components.
