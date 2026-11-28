# Changelog

## [Unreleased]

### Fixed

- Fixed `audit naming`, `audit delegation` and work/analytics readers to resolve the logical `--index FILE` through the same publication seam as the other index readers. They now read the pinned generation address when the publication is usable; with an unusable sidecar they use the named path only if it is itself a usable address, while derived-layer commands still refuse instead of reporting an empty result.

## [0.8.0] - 2026-11-28

### Added

- Licensed the current source tree under [MIT](LICENSE) with copyright held by Le Minh Triet; the workspace manifest declares `license = "MIT"` and every crate inherits it. The `v0.7.0` tag and archive are unchanged.
- Added `ase search --text`, a metadata-only hit list with the returned count, full generation, explicit index path and per-hit role, span, freshness, event and selector. It never prints the query, prose, journal paths or generated shell commands; JSON stays the default and `--text` cannot be combined with `--json`, `--open-args` or `--explain`. No hits remains exit `0`, while an unavailable postings layer remains `layer_unavailable` with exit `4`.
- Added `ase open --text`, a metadata-only reopen report with the evidence status, block key, span, encoding and coordinate system, both hash checks and the freshness hint. It never prints prose, call signatures or journal paths; JSON stays the default and `--text` cannot be combined with `--json`, `--content` or `--session-args`. A hit from another generation still refuses with `generation_mismatch` and exit `4` before reading raw, a readable mismatch stays `unverified` with exit `0`, and an unreadable raw source, missing mapping or unusable publication remains an error with exit `2`.
- Published the Windows x64 binary `ase-0.8.0-windows-x64.zip` with `SHA256SUMS.txt`. The archive contains `ase.exe`, `README.md`, the MIT `LICENSE` and `THIRD_PARTY_NOTICES.txt`; it is unsigned and requires the Microsoft Visual C++ 2015–2022 Redistributable (x64) for `VCRUNTIME140.dll`. Default JSON output and existing exit codes are unchanged.

### Notes

- `v0.8.0` was accepted by the project owner on fixture and build-host evidence. The `--text` modes were exercised on synthetic fixtures only; no clean Windows machine, new user profile, restored VM, Windows 10/Server, ARM64, blocked network or real corpus was tested, and SmartScreen/enterprise policy behavior is unmeasured.
- The binary is built from the `v0.8.0` tag with Rust `--remap-path-prefix` and MSVC `/pathmap`; rebuilding does not reproduce the published digest because PE timestamps and PDB identity differ.
- The version is `0.8.0` because `ase search --text` and `ase open --text` add new opt-in output modes; default JSON output and the evidence contract are unchanged.
- The `v0.7.0` tag and archive remain as released and do not contain the project `LICENSE`.

## [0.7.0] - 2026-11-27

### Added

- Published the first Windows x64 binary release, `ase-0.7.0-windows-x64.zip`, with `SHA256SUMS.txt`. The archive contains `ase.exe`, `README.md` and `THIRD_PARTY_NOTICES.txt`; it is unsigned and requires the Microsoft Visual C++ 2015–2022 Redistributable (x64) for `VCRUNTIME140.dll`. The binary builds on the `v0.6.0` behavior; no command, flag, output, exit code or schema changed.
- Added `THIRD_PARTY_NOTICES.txt` at the repository root and inside the archive. It lists exactly the 59 third-party packages resolved by `Cargo.lock`, includes the previously missing `cc 1.11.2`, removes a duplicate `generic-array 0.14.7` row, and marks build-time-only and unused-target crates consistently.

### Changed

- Rewrote `README.md` as a product README: download and verification steps, first local index, build from source, command map, data model, privacy defaults, uninstall and limitations. Sprint history, research narrative and stale Windows integration notes were removed.
- The release binary is built with Rust `--remap-path-prefix` and MSVC `/pathmap` so the image does not embed the build host's source paths. Rebuilding does not reproduce the published digest: PE timestamps and PDB identity differ between runs.
- Release verification instructions use `certutil -hashfile` instead of `Get-FileHash`/`Get-AuthenticodeSignature`, which are unavailable or broken on the tested host, and state the unsigned status without promising SmartScreen acceptance.

### Notes

- `v0.7.0` was accepted by the project owner on fixture and build-host evidence. It was not tested on a clean Windows machine, a new user profile, a restored VM, Windows 10/Server, ARM64 or a blocked network, and SmartScreen/enterprise policy behavior is unmeasured.
- The repository still has no `LICENSE` or `NOTICE` file and no workspace `license` field. No license grant for the ASE source or binary is made by this release; ask the project owner before redistributing.
- The version is `0.7.0` because this is the first binary distribution channel; `0.6.0` remains the last source-only prerelease.

## [0.6.0] - 2026-11-27

### Added

- Added `ase search --open-args`, an opt-in `open_args` field on every hit with the exact `--event`, `--selector`, `--generation` and `--index` values from that hit, plus a `flags` array ready to pass to `ase open`. Nothing is executed, no query, prose or source-journal path is emitted, and default search output, ranking and `open` verification are unchanged.
- Added `ase open --session-args`, an opt-in `session_args` locator for a verified block. It is `ready` only when the session id is declared in its header, the header span still re-reads to that id, and exactly one session in the pinned index declares it; it emits `show`, `resolve` and OMP `work session` flags that target the immutable generation address. Missing or file-name-derived ids, duplicate ids, edited headers and unverified evidence are `refused` with a reason and no runnable flags.

### Changed

- Changed session-id selection in `show`, `resolve`, `work session`, `analytics session` and `adapter omp-stock` to refuse an ambiguous id instead of silently opening one of the matching sessions. This is a behavior change: an id shared by two indexed sessions (for example the same Codex session indexed from both JSONL and zstd) now exits 2 with the match count and asks for an explicit span or file address, while a unique id behaves as before.

### Notes

- `v0.6.0` is a source prerelease from `main`; no binary assets are published.
- Sprint 9 was verified on synthetic fixtures with an independent review; no real-user pilot or real corpus was run.
- Outbound/network behavior was not verified in a blocked-outbound environment. The default navigation path makes no model/network call, but this does not prove the CLI never attempts network access.

## [0.5.1] - 2026-11-27

### Fixed

- Fixed the `v0.5.0` source snapshot, which omitted the final `ase-cli` module declaration ordering: `v0.5.1` publishes the verified integrated tree. Commands, flags, help text, stdout/stderr, exit codes, schemas, serialization and public Rust API are identical between `v0.5.0` and `v0.5.1`.

### Notes

- `v0.5.1` is a source prerelease from `main` and replaces `v0.5.0` as the newest tag; `v0.5.0` remains published and unchanged. Gate and measurement records remain workspace-local evidence and are not release assets.

## [0.5.0] - 2026-11-27

### Added

- Added `ase join skill-gap --store FILE --catalog FILE --bundle DIR --local-map FILE` for verified evidence bundles. It reopens the cited bundle evidence, excludes assertions with stale bundle identity, judged content, provenance or spans, and joins only verified assertions to a user-supplied exact-kind/key/alias catalog. It reports `match`, `no_match`, `ambiguous` and `insufficient_input`, honors catalog completeness, keeps a path-free verdict → occurrence → span trace, treats a catalog miss as no evidence that a skill exists or should be created, and never writes a store, calls a model or measures a gate.
- Added `ase import verdicts FILE --bundle DIR --local-map FILE --store FILE` for evidence bundles. It reopens the bundle and every cited evidence row before writing a separate local `ase-assertions-v1` layer, enforces the contract's identity, stale-bundle, cross-bundle, unverified-evidence and judge checks per record in order, keeps provenance and path-free span references for later catalog joins, ignores identical replays, and inserts all passing rows in one transaction while never storing a rejected row. It never calls a model, reads a catalog or measures a gate.
- Added `ase export motif:<id> --out DIR --local-map FILE` for evidence bundles. It runs the pinned motif query, selects at most five representatives deterministically, redacts result text, writes only path-free observations and questions, and keeps the alias-to-source map outside the bundle. A private staging copy must reopen and verify before anything is published; digest, selector, redaction, citation-cap or planted-leak failures refuse export. It reports no gate result and makes no model or network call.
- Added `ase verify <bundle-dir>` for evidence bundles. It resolves opaque source aliases only through a private local map, re-reads each cited byte span, re-runs the versioned selector, rechecks strong block/result digests, re-applies the pinned redaction policy, and verifies bundle, view and input identities with path-free failure codes. Missing maps, aliases or sources are reported unverified; a changed whole-file digest with valid spans is accepted only with a separate append watermark and no completeness claim. Optional `--verdicts FILE` cross-checks assertions without importing or writing anything. The existing `ase verify --all` span check is unchanged.
- Added `ase-llm` and `ase llm config|key|status`: user-scoped LLM API configuration and secret resolution for the optional Judge API transport. Providers are explicit user-defined aliases; a saved key is scoped to the exact `(provider, normalized origin)` pair, so same-provider/same-origin profiles share one key, distinct providers on one origin stay isolated, and a provider switch or origin change never reuses the earlier key. Five typed APIs are configured independently: OpenAI-compatible `chat-completions` and `responses` (default `medium` effort), Console System One `decision`, and native Gemini `gemini` and `gemini-decision` (no OpenAI effort). One-time hidden stdin enrollment stores the key in a Windows Credential Manager Generic credential (or an explicitly selected workspace-local DPAPI store); `OPENAI_API_KEY` is process-only for the OpenAI-compatible kinds, `GEMINI_API_KEY` is process-only for the native Gemini kinds, the two families never fall back to each other, and neither is ever persisted. Non-secret profiles (`provider`, typed API, full request URL, model, effort) live in a strict version-2 JSON config; `ASE_LLM_PROVIDER`, `OPENAI_API_TYPE`, `OPENAI_API_ENDPOINT`, `OPENAI_API_MODEL` and `OPENAI_API_EFFORT` override it in a documented order. `ase llm status` is a local, redacted readiness report; URL credentials, fragments and credential-shaped query parameters are rejected, an unsupported credential platform fails explicitly, and argv key input is refused without echo.
- Added `ase-judge` and `ase judge run|verify|report|show|fixture|contract-runner`: an offline evaluation over a pinned, reopenable OMP projection. The deterministic `baseline-rubric-v2` baseline, the local SLM boundary stage and the separate local Judge stage keep independent versions, identities and cache namespaces. Every assertion carries two exact source anchors plus a rendered-input digest, is rejected at insert when evidence is missing or does not re-open, and is cached by span/input and version rather than by whole-journal digest. Read commands never call a model. Model access is either a workspace-local stdin/stdout process protocol or a typed API profile resolved by `ase llm`; a missing or failed stage never falls back to the baseline.
- Added typed API backends for `ase judge`: `ase llm` frames and sends the five configured protocols (Chat Completions, Responses, native Gemini generation, Console System One Decision and native Gemini Decisions) with protocol-defined auth (`Authorization: Bearer` or `x-goog-api-key`), exact effort policy, one request per decision, redirect refusal and stable redacted errors. Added `ase llm smoke` for a fixed synthetic wire check, `--slm-profile`/`--judge-profile` for evaluation, and `--allow-remote-session-data` for an explicit, destination-printing opt-in; loopback runs need no opt-in.
- Added bounded failure diagnostics to `ase llm smoke`: a failed call emits an `ase-llm-smoke-diagnostic/v1` JSON report distinguishing incomplete/truncated Responses (with an allowlisted reason), refusal or missing output text, and text that does not map to the rubric. It includes the HTTP status when known and the extracted text's length/JSON delimiter presence, never credentials, endpoint queries, prompt, response, output/refusal text or provider error text. No retry or fallback request is made and the successful report schema is unchanged.

### Changed

- Changed unreleased judge and evidence-bundle schema identifiers to phase-free names: `ase-bundle-v1`, `ase-motif-bundle-v1`, `ase-verdicts-v1`, `ase-assertions-v1`, `ase-join-v1`, the render/redaction/representative policies and the `ase-judge-*` input, cache, assertion, run, report and fixture schemas. Bundle, verdict, assertion, judge-run and fixture files written by earlier unreleased builds are rejected by schema version and must be re-exported or regenerated. The released `ASEP0A01` index magic and `s4t1-turn-v1` compatibility identifier are unchanged and remain isolated as documented legacy constants.
- Renamed `ase fixture-p0b` to `ase fixture-address`; its generated session, file and prose labels are synthetic domain labels rather than phase names.
- Removed compiled default session roots that named the build host's account. `--omp-root`/`--codex-root` still take precedence; defaults now read `ASE_OMP_ROOT`/`ASE_CODEX_ROOT` and otherwise resolve under the current user's home directory.
- Changed model confidence storage to preserve semantics: generation self-reports are stored as `verbal:<level>`, decision probabilities as `probability:<value>`, and `judge show` reports a `confidence_semantics` object. Neither value is translated into the other or thresholded by an uncalibrated cutoff. The SLM and Judge prompt versions are now `slm-boundary-v2`/`judge-adjudicate-v2`; historical `-v1` assertions remain valid evidence and are not reinterpreted.
- Changed the API cache identity to include the typed API, normalized endpoint, provider, model, resolved effort and versioned wire/parse semantics, never a credential. A shared store therefore cannot mix protocols or effort configurations. `none` is a valid explicit effort for the two OpenAI-compatible text protocols and is sent verbatim; there is no automatic conversion or retry.
- Changed `ase llm smoke` to request the rubric's exact JSON object in the fixed synthetic instruction for text-generation APIs (`chat-completions`, `responses`, `gemini`): one object with `label` (`split`/`continue`/`unknown`), verbal `confidence` (`high`/`medium`/`low`) and `abstain_reason` (a short string only for `unknown`, otherwise `null`). Both decision APIs keep their typed-choice instruction and probability contract; no response format is forced and no retry is added.

### Fixed

- Fixed `ase llm config enroll` for newly created profiles. It saves the key for the provider and endpoint origin in the profile being written, not the old config or a runtime environment override.

### Removed

- Removed the Ollama backend, its `--*-ollama-*` options, the loopback HTTP client, model probe and their documentation and tests; no alias, fallback or compatibility shim remains. Historical Ollama evaluation records under `artifacts/` are preserved as evidence and are not a runnable backend.

### Limitations

- `ase llm` stores configuration and credentials and performs one typed request per explicit call; it has no provider catalog, endpoint inference or automatic request. The Windows Credential Manager write/read path is compiled and is the default backend, but verification exercised persistence with an isolated workspace-local DPAPI store (`ASE_LLM_CREDENTIAL_STORE=dpapi-file:<dir>`) plus a missing-entry read probe only; the agent created no real user credential-store entry, and T5 used process environment keys only. Native enrollment remains an owner action.
- No live provider request is part of T5. The release CLI was verified against a synthetic loopback server for all five protocols (auth location, body shape, effort, typed parse, redaction, redirect refusal, cold/warm cache); the Windows WinHTTP HTTPS path shares the same request code but no TLS/provider call was made and no real key was used. A target whose effort capability differs (for example a Chat target that advertises only `none`/`high`) needs an explicit profile value; there is no silent conversion or retry.
- The full-cohort offline SLM and Judge run was unmeasured in the published source text; a workspace-local experiment has since measured the cache/zero-call gate over the 1,400 pinned decisions with a temporary, reverted `redact-v1` pre-send policy and fail-closed allowlist. The unchanged replay made 0 model calls with an identical assertion digest and `verify` reopened 4,200/4,200 anchors. The shipped CLI keeps policy `none` and has no judge redaction option; the 26-decision ablation is descriptive with no quality threshold, so no cohort quality verdict or historical-recap equivalence is claimed. The narrow real-inference verification (two pinned units, five decisions) ran cold and warm on the now-removed loopback Ollama backend; those records remain historical evidence, not a runnable backend or a judge cohort measurement. The contract runner used for cache and protocol verification is a synthetic protocol double behind `--verification-only`, in a separate cache namespace; its labels are never evaluation results. Session recaps are not read, `unknown` is never merged into a positive label, and no judge quality gate is claimed.

### Notes

- `v0.5.0` is a source prerelease from `main` at the release commit; it is not a gate or quality verdict. Gate and measurement records remain workspace-local evidence and are not release assets. Public source commit `7c6000ba8bf9` first published the judge and evidence-bundle implementation with synthetic fixtures under `fixtures/p4/`; `v0.4.0` predates those commands. The current runnable bundle fixture is the `ase-bundle-v1` tree under `fixtures/bundle/`; the phase-named `fixtures/p4/` tree is retained only as historical source and is rejected by the current schema. The CLI, stock-adapter, `ase-task`, `ase-judge` and evidence-bundle module layout was reorganized for maintainability with no change to commands, flags, output, exit codes, schema, serialization or public Rust API.

## [0.4.0] - 2026-11-23

### Added

- Added `ase-work` and `ase work project|session|reopen|compare`: a leaf-aware OMP `Turn`/`Episode` projection under rule `omp-turn-episode` / `turn-episode-v1`, compatible with the locked Sprint-4 reference rule `s4t1-turn-v1`. The active leaf is the graph leaf with the greatest `(entry timestamp, ordinal)`; `--leaf ENTRY_ID` projects an explicit older tip; parallel tool calls stay one turn; primary and user-attributed `custom_message` episode boundaries are reported separately. The projection is task-agnostic, so unresolved and no-task sessions project identically, and it never reads or writes resolver or motif membership.
- Added `ase work reopen` and `ase work compare`: detailed projections carry generation-pinned, selector-versioned evidence for every leaf turn and episode boundary; reopen re-reads the span, checks index checksum and SHA-256 line digest, re-runs the selector and recomputes the whole journal, reporting `stale` on any mismatch. Compare aligns product rows with an independent reference by exact journal SHA-256, reports per-journal differences before aggregation, both cancellation directions, excluded product/reference-only files and the strict aggregate relative-error result.
- Added `ase-adapter-omp-stock`, a separate stock-journal adapter that emits advisory `parent_child`, `repo` and `task` claims without writing resolver assignment, motif membership, the address index or any source journal. `confirmed`, `candidate`, `missing`, `unavailable` and `conflict` stay independent per layer; multi-child calls keep one scoped claim per sibling and multi-task sessions report `multi_task`.
- Added `ase adapter omp-stock` (`--session`, `--file`, `--all`, `--out`, `--reopen`) and `ase fixture-adapter`: claims carry generation-pinned, reopenable evidence with index checksum, strong line digest, versioned selector and block digest. `--reopen` re-reads source bytes, re-runs the selector and marks any mismatch `stale`; it refuses another generation before reading raw. The synthetic fixture covers matching, multi-child, missing, conflict and multi-task shapes with no real transcript content.
- Added `ase work delta` and `ase fixture-delta`: an explicit before/after comparison of two attempts of one project/task confirmed only by native dispatch joins. The report orders attempts by dispatch parent-record time rather than attempt label, shows signed count differences plus turn and episode-boundary changes with reopenable evidence, and rejects ambiguous, cross-project, unrelated-task, unconfirmed and unverifiable pairs before emitting any delta.
- Added `ase analytics` and `ase fixture-analytics` (rule `runtime-analytics` / `runtime-analytics-v1`): model-by-project runtime reports over the P1 leaf/turn projection using nominal stored `usage.cost.total` with no repricing and no `stats.db` dependency. Call duration, time-to-first-token and the union of `[timestamp, completedAt)` clock intervals are reported with explicit coverage; missing cost, timing or model fields are counted, never zero-filled. Active-leaf, all-branch and abandoned turn/cost/duration totals are reconciled against P1 counts, project provenance comes from the journal tree layout with an explicit unknown group, and `ase analytics reopen` re-verifies generation-pinned evidence and stored runtime facts from raw bytes.

### Fixed

- Fixed `ase analytics project --start/--end` root selection: a scoped run now projects every root journal and its subagents instead of excluding journals whose session header predates the window. Pre-window journals contribute their in-window turns with complete branch context, and the report's `period` totals include only turns inside the half-open `message.timestamp` window while `totals` remains the unscoped all-branch reconciliation. `ase work project` keeps its separate header-time selection contract.
- Fixed duration reconciliation at corpus scale: stored durations are accumulated as exact fixed-point nanoseconds, so the leaf/all/abandoned branch balance is exact instead of failing on float rounding of roughly 2e9 ms. Genuine imbalances still produce a nonzero residual; the cost reconciliation gate is unchanged.

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
