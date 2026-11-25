# agents-session-explorer — session source index and evidence tools

The address layer is a standalone, address-only source index for OMP/Codex
session files. It answers *where* an observation came from. The derived search
layer adds a generation-pinned, contentless full-text and shingle layer for
allowlisted prose and tool-call signatures. Task resolution reconciles file-name
grammar, `parentSession` delegation and `task_id` prose markers with inspectable
evidence. No query layer calls a model; the `judge` layer is the only model
caller and runs offline only (a workspace-local process or an explicitly
configured typed API profile), never on a query path. A request that carries session text to a
non-loopback endpoint requires an explicit run opt-in; `ase llm smoke` is the
fixed synthetic wire check and sends no session data. `ase llm` stores the
non-secret API profile and the user-scoped API key used by that transport.

**Publication state:** the latest published release tag is
[`v0.4.0`](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.4.0),
a source prerelease from before the judge and evidence-bundle commands. Public
source commit
[`7c6000ba8bf9`](https://github.com/triet4p/agents-session-explorer/commit/7c6000ba8bf964ba2b454a0888ebb7de3b36ebed)
adds the judge and evidence-bundle implementation and runnable synthetic
fixtures under `fixtures/p4/`; it is a source publish, not a release, and its
status text predates later local measurements. Those measurement records are
workspace-local evidence and are not published release assets. This checkout
does not contain the public `fixtures/p4/` tree; the commands below generate
equivalent synthetic inputs under `.tmp/`.

## Design documentation

- [Use cases and requirements](docs/USE_CASE.md) — user workflows and acceptance criteria.
- [Scope](docs/SCOPE.md) — supported behavior, non-goals, and layer boundaries.
- [Static architecture](docs/STATIC_ARCHITECTURE.md) — current components and proposed changes.
- [Data contracts](docs/DATA_CONTRACTS.md) — provenance, source coordinates, and derived data.
- [Derived search verification](artifacts/P0b/summary.md) — fixture proofs, release measurements and limits.
- [Initialization record](docs/INITIALIZATION.md) — original design and corpus observations.

## Build

```bash
cargo build --release --locked
```

Rust 1.91.1 or newer. Produces `target/release/ase` (`ase.exe` on Windows).

## v0.2.0 integration on Windows x64

`resolve task:` needs an OMP runtime that records dispatch tracking; sessions
without it stay unresolved. Build ASE with its locked dependency set:

```bash
cargo build --release --locked
```

**1. Patch the pinned OMP source.** The patch is tied to upstream `v19.0.0`
(`57771ae`); do not apply it to another base.

```bash
git clone --depth 1 --branch v19.0.0 https://github.com/can1357/oh-my-pi.git
cd oh-my-pi
git apply --check ../support/omp/omp-15c418d.patch
git am --keep-cr ../support/omp/omp-15c418d.patch
```

**2. Add the public Windows x64 addon and build the runtime.** Hashes,
provenance and license notes are in `support/omp/README.md`.

```powershell
bun install --frozen-lockfile
npm pack @oh-my-pi/pi-natives-win32-x64@19.0.0
tar -xzf oh-my-pi-pi-natives-win32-x64-19.0.0.tgz
copy package\pi_natives.win32-x64-baseline.node packages\natives\native\
bun run --cwd=packages/coding-agent build
```

**3. Index with explicit roots.** The compiled defaults listed below point at
the build host's account; pass your own roots on any other machine.

```powershell
.\ase.exe index --layer all --omp-root "%USERPROFILE%\.omp\agent\sessions" --codex-root "%USERPROFILE%\.codex\sessions" --index ase.index --json
.\ase.exe resolve task:<TOKEN> --index ase.index --json --explain
.\ase.exe verify --all --index ase.index --json
```

Index rebuild compatibility: dispatch records are read from indexed
custom-record spans, so the address layout is unchanged and an existing
address index stays readable. Rebuild with `--layer all` when the index
predates the dispatch records themselves. An address-only rebuild tombstones
the postings pointer and disables search and marker evidence until postings
are rebuilt with `--layer postings` (or `--layer all`).

Limitations: tracked new OMP sessions only; untracked sessions stay
unresolved. The frozen holdout had zero assignable members (disclosed, not
replaced by synthetic fixtures); no historic or motif-detection claim is made.

## Run

```bash
# Derived layer: one generation, validated atomic pointer
ase index --layer all --index ase.index --json

# Search metadata only; open the exact block from raw on demand
ase search "phrase or terms" --index ase.index --json --explain
ase open --event N --selector S --generation G --index ase.index --json
ase shingle --event N --index ase.index --json

# In-process latency sample (lookup and evidence reopen reported separately)
ase bench-search --n 2000 --seed 7 --index ase.index --json
# Show provenance for a span or a session (metadata only; --content opts in)

ase show span:<FILE>:<OFF>:<LEN> --index ase.index --json --explain
ase show session:<ID> --role assistant --index ase.index --json

# Resolve source-backed relations with byte evidence
ase resolve session:<ID> --index ase.index --explain
ase resolve span:<FILE>:<OFF>:<LEN> --index ase.index --explain
ase resolve task:<TOKEN> --index ase.index --json --explain   # evidence-backed assignment

# Naming grammar: synthetic fixture, metadata-only audit
ase fixture-naming --out .tmp/naming
ase index --omp-root .tmp/naming/omp --codex-root .tmp/naming/codex --index .tmp/naming/ase.index
ase audit naming --index .tmp/naming/ase.index --json --explain

# parentSession delegation: synthetic fixture, metadata-only audit
ase fixture-delegation --out .tmp/delegation
ase index --omp-root .tmp/delegation/omp --codex-root .tmp/delegation/codex --index .tmp/delegation/ase.index
ase audit delegation --index .tmp/delegation/ase.index --json --explain

# task_id markers: synthetic fixture, metadata-only audit
ase fixture-markers --out .tmp/markers
ase index --layer all --omp-root .tmp/markers/omp --codex-root .tmp/markers/codex --index .tmp/markers/ase.index
ase audit markers --index .tmp/markers/ase.index --json --explain
ase open --event N --selector S --generation G --index .tmp/markers/ase.index --content

# Task identity reconciliation: synthetic fixture, metadata-only resolve
ase fixture-resolve --out .tmp/resolve
ase index --layer all --omp-root .tmp/resolve/omp --codex-root .tmp/resolve/codex --index .tmp/resolve/ase.index
ase resolve task:101 --index .tmp/resolve/ase.index --json --explain

# Native dispatch provenance: synthetic producer-shaped fixture
ase fixture-dispatch --out .tmp/dispatch
ase index --layer all --omp-root .tmp/dispatch/omp --codex-root .tmp/dispatch/codex --index .tmp/dispatch/ase.index
ase resolve task:410 --index .tmp/dispatch/ase.index --json --explain
ase resolve task:10410 --index .tmp/dispatch/ase.index --json --explain
ase show span:<FILE>:<OFF>:<LEN> --index .tmp/dispatch/ase.index --json --explain

# Motif alignment and support: synthetic fixture, occurrence reopen
ase fixture-motif --out .tmp/motif
ase index --layer all --omp-root .tmp/motif/omp --codex-root .tmp/motif/codex --index .tmp/motif/ase.index
ase motif 1 2 3 9 --index .tmp/motif/ase.index --tau 0 --json --explain
ase open --event N --selector S --generation G --index .tmp/motif/ase.index --json

# OMP stock adapter: advisory parent-child / repo / task claims (no membership)
ase fixture-adapter --out .tmp/adapter
ase index --layer all --omp-root .tmp/adapter/omp --codex-root .tmp/adapter/codex --index .tmp/adapter/ase.index
ase adapter omp-stock --all --index .tmp/adapter/ase.index --out .tmp/adapter/claims.json --json
ase adapter omp-stock --session <ID> --index ase.index --json
ase adapter omp-stock --reopen .tmp/adapter/claims.json --index .tmp/adapter/ase.index --json

# Turn/Episode projection: active leaf by default, explicit older leaf on request
ase index --omp-root .tmp/work/omp --codex-root .tmp/work/codex --index .tmp/work/ase.index
ase work project --omp-root .tmp/work/omp --start 2026-09-01T00:00:00Z \
  --end 2026-10-01T00:00:00Z --index .tmp/work/ase.index \
  --out .tmp/work/rows.jsonl --json
ase work session --session <ID> --index .tmp/work/ase.index --json
ase work session --session <ID> --leaf <ENTRY_ID> --index .tmp/work/ase.index \
  --out .tmp/work/session.json --json
ase work reopen .tmp/work/session.json --index .tmp/work/ase.index --json

# Compare an independently counted reference, aligned per journal by SHA-256
ase work compare --projection .tmp/work/rows.jsonl \
  --reference .tmp/work/reference.jsonl --json

# Explicit before/after attempt delta for one dispatch-confirmed task
ase fixture-delta --out .tmp/delta
ase index --omp-root .tmp/delta/omp --codex-root .tmp/delta/codex --index .tmp/delta/ase.index
ase work delta --task T701 --project Proj31 --before delta-before --after delta-after \
  --omp-root .tmp/delta/omp --index .tmp/delta/ase.index \
  --out .tmp/delta/delta.json --before-out .tmp/delta/before.json \
  --after-out .tmp/delta/after.json --json
ase work reopen .tmp/delta/before.json --index .tmp/delta/ase.index --json
ase work reopen .tmp/delta/after.json --index .tmp/delta/ase.index --json

# Runtime analytics: nominal stored cost, duration vs clock union, coverage
ase fixture-analytics --out .tmp/analytics
ase index --omp-root .tmp/analytics/omp --codex-root .tmp/analytics/codex \
  --index .tmp/analytics/ase.index
ase analytics project --omp-root .tmp/analytics/omp --index .tmp/analytics/ase.index \
  --start 2026-10-01T00:00:00Z --end 2026-11-01T00:00:00Z \
  --out .tmp/analytics/rows.jsonl --report .tmp/analytics/report.json --json
ase analytics session --session <ID> --index .tmp/analytics/ase.index \
  --out .tmp/analytics/row.json --json
ase analytics reopen .tmp/analytics/rows.jsonl --index .tmp/analytics/ase.index --json

# Re-read raw bytes at every indexed span
ase verify --all --index ase.index --json

# Deterministic in-process show-latency sample over OMP spans
ase bench-show --n 2000 --seed 1 --index ase.index --json
```

Defaults: `--index ase.index`; OMP root from `ASE_OMP_ROOT`, else
`$HOME/.omp/agent/sessions` (`%USERPROFILE%` on Windows); Codex root from
`ASE_CODEX_ROOT`, else `$HOME/.codex/sessions`. An explicit `--omp-root` or
`--codex-root` always wins; no build-host path is compiled in.

`ase verify <bundle-dir>` reopens an evidence bundle from a private local
source map. It is distinct from the span-level `verify --all` above, whose
flags and output are unchanged.

```bash
ase verify fixtures/bundle \
  --local-map fixtures/bundle/local-map.json \
  --verdicts fixtures/bundle/verdicts.json --json
```

The same synthetic tree is documented in `fixtures/bundle/README.md`. It uses
the current `ase-bundle-v1` schema. The older public `fixtures/p4/` tree is not
present in this checkout.

The `--local-map` JSON is never part of the bundle. It carries
`schema: ase-local-sources-v1`, a pinned `snapshot` (`snapshot_id`,
`core_digest`, per-file `sha256`/`size`), and `aliases` mapping each opaque
`src-<16 hex>` to a file id, file digest and local path. Relative paths are
resolved next to the map file. Without a supplied map, an unknown alias, an
absent source or an unreadable source, every affected row is reported
`unverified`; the command never returns `verified` on missing evidence.

Selectors use `block-v1|span:<byte-offset>:<byte-length>|message.content[<index>]|<block-type>`.
The block type is `toolCall` for a call and the type of the reopened result
block (`toolResult` or native OMP `text`). Verification reads those exact bytes
again, re-runs the selector, checks the strong SHA-256 of the reopened block,
resolves the paired result by `toolCallId`/`id`, re-applies `ase-redact-v1`,
and recomputes the excerpt and input identities. The displayed signature is
UTF-8-safe capped at 160 bytes; an optional `result_lines` field records how
many of the first three redacted lines fit the 512-byte call cap so the exact
published excerpt can be regenerated. Bundle and view digests, engine/redaction
versions and the snapshot pin are checked first. The JSON report separates
`evidence_total`/`evidence_verified`/`evidence_failed`/`evidence_unverified`,
`motifs_total`/`motifs_verified`, and optional read-only assertion counts;
`failures[]` carries path-free codes and details.

If the whole source file digest differs but every reopened span still checks
out, rows are accepted as `verified_span_after_append` with a separate
watermark note. That means only the cited spans were rechecked; it makes no
claim that the current file is complete or that a gate was measured. The
optional `--verdicts FILE` is a read-only cross-check against the bundle;
nothing is imported or written, and no model or network call is made.

`ase export motif:<id> --out DIR --local-map FILE` writes a shareable evidence bundle
from the pinned motif query. Addresses come from repeated `--address
PROJECT|TASK` or from a private local `--addresses FILE`; the query config,
representative policy, redaction policy and citation limits are fixed by the
frozen contract and are not CLI options.

```bash
ase export "motif:$MOTIF_ID" \
  --out .tmp/p4/bundle \
  --local-map .tmp/p4/local-map.json \
  --addresses "$TASK_ADDRESSES" \
  --index "$INDEX" --json
ase verify .tmp/p4/bundle --local-map .tmp/p4/local-map.json --json
```

The output directory must be empty. `bundle.json`, `motif.json`, `evidence.md`
and `questions.md` contain only observations, opaque aliases, digests and fixed
questions; no real source path, alias map, journal, raw tool output, verdict,
catalog entry or local tracking identity is included. The `--local-map` file is
written outside the bundle and is required to reopen evidence. Export selects at
most five representatives per motif in the fixed earliest → latest →
most-divergent → failure → fill order, redacts result text before publication,
and drops trailing fill/divergent/failure representatives if the 32 KiB bundle
cap is reached. A private staging copy is reopened and verified first; a digest,
span, selector, view, redaction or leak mismatch refuses publication and leaves
the output directory untouched. Export reports evidence counts only for that
prepublication check; it does not measure a gate.

`ase import verdicts FILE --bundle DIR --local-map FILE --store FILE` imports
externally written judgments into a separate local assertion layer. The bundle
and every referenced evidence row are reopened through the private local map
before any write; the verdict file is never a source-evidence substitute and
the tool never writes or infers a judgment.

```bash
ase import verdicts fixtures/bundle/verdicts.json \
  --bundle fixtures/bundle \
  --local-map fixtures/bundle/local-map.json \
  --store .tmp/bundle-store.json --json
ase verify fixtures/bundle \
  --local-map fixtures/bundle/local-map.json \
  --verdicts fixtures/bundle/verdicts.json --json
```

The `--store FILE` is a local `ase-assertions-v1` JSON layer, never part of
the bundle. Each accepted row keeps its `assertion_id`, bundle id/digest,
motif/occurrence/question ids, judgment fields, sorted evidence references,
input identity, path-free evidence span references, the digest of the verdict
file and the judge/generated-at provenance. A local registry maps evidence ids
to the bundle that verified them, so an id from a different bundle is reported
`cross_bundle_reference` rather than silently accepted. Contract §4.3 checks
run in order: unsupported schema, wrong bundle id, stale bundle digest/version,
missing judge, unknown occurrence/question, missing or cross-bundle or
unverified evidence, stale input identity, recomputed assertion id conflict,
and catalog key/kind grammar. A content-identical replay is
`duplicate_ignored`; an id reused with another body is `assertion_id_conflict`.
Rejected rows are never stored; all rows that pass are inserted together in one
atomic replace, and when no row passes the store is left byte-identical. The
command exits nonzero if any row was rejected. No model, catalog join or gate
measurement is performed here.

`ase join skill-gap --store FILE --catalog FILE --bundle DIR --local-map FILE`
compares only verified persisted assertions with a user-supplied
`ase-skill-catalog-v1` file. It reopens the bundle evidence through the private
map again, drops assertions whose bundle identity, judged content or cited
spans no longer verify, and emits one row per surviving assertion. Matching is
exact byte equality on `(catalog_kind, key or alias)`: `match`, `no_match`,
`ambiguous` or `insufficient_input`. An incomplete catalog kind yields
`insufficient_input`, never `no_match`, and a catalog miss is explicitly not
evidence that a skill is absent or should be created.

```bash
ase join skill-gap --store .tmp/bundle-store.json \
  --catalog fixtures/bundle/catalog.json \
  --bundle fixtures/bundle \
  --local-map fixtures/bundle/local-map.json --json
```

Each row carries `assertion_id`, `occurrence_id`, sorted `evidence_refs`,
path-free `evidence_span_refs`, `verdict_digest`, sorted `entry_ids`, `reasons`
and a deterministic `join_id`, so row → verdict → occurrence → span stays
openable without exposing a private source path. Add `--verdicts FILE` to
recheck each assertion's verdict digest and judged content against the original
verdict file as well. This command is read-only: it never writes the assertion
store, bundle or catalog, never calls a model, and reports no gate result.

**Pilot gate evidence.** On the pinned two-motif OMP pilot,
`export → verify → import verdicts → join skill-gap` ran locally with
user-supplied verdicts and no LLM invocation: 4/4 frozen candidates had
verified evidence (100%, against a ≥80% target requiring 4/4), 19/19
bundle evidence rows and 4/4 accepted assertions reopened, and the join
reported 4/4 `match`. A tampered bundle was rejected outside the positive
rates. These numbers apply only to that pilot and its frozen denominator;
they are not a quality result, a catalog-wide claim or a gate measurement
for other corpora. The measurement records are workspace-local, not part of
the published source or a release asset.


`ase work project` derives `Turn`/`Episode` rows under rule
`omp-turn-episode` / `turn-episode-v1`, compatible with the released
reference identifier `s4t1-turn-v1` (isolated as `COMPATIBLE_REFERENCE_RULE`). A turn is one assistant `message` entry with a
`message.usage` object; the active leaf is the graph leaf with the greatest
`(entry timestamp, ordinal)` and the active path walks `parentId` to the root.
Episodes are runs of turns between user boundaries; the primary boundary is a
user message, and the wide boundary also counts a `custom_message` with
`attribution = "user"`. Parallel tool calls remain one turn. Branch choice,
window-sensitive counts, abandoned turns and empty episodes are reported
separately, so a total cannot hide opposite-sign errors.

Each row is task-agnostic: no resolver or motif membership is read, and a
session whose task is unresolved still projects identically. `--leaf ENTRY_ID`
projects an explicit older tip instead of the active leaf. Detailed session
output embeds a versioned, index-pinned evidence reference per turn, episode
boundary and selected leaf; `ase work reopen` checks the generation, file
digest, span, index checksum, line digest and selector, then recomputes the
whole projection. Any mismatch is `stale`, never silently repaired.
`ase work compare` aligns product rows with an independent reference by exact
journal SHA-256 and reports per-journal diffs before aggregation, both
cancellation directions and excluded/unresolved rows.

`ase work delta` compares two **explicitly named** child sessions of one
project/task membership confirmed by native `task_dispatch_v1` /
`task_dispatch_child_v1` joins (`task-dispatch-provenance` / `dispatch-v1`),
under rule `omp-attempt-delta` / `attempt-delta-v1`. It passes no naming,
marker, path, adapter or motif input to the resolver: a bare task token with
two project scopes is `ambiguous_membership`, a session whose observed claim
names another task or project is `task_mismatch`, and a session with only a
`task_id=` prose mention or a candidate claim is `unconfirmed_membership`.
Every rejection exits 3 with the admitted attempts and the observed (never
promoted) claims; no partial delta is emitted.

Attempt order is **dispatch parent-record time**, tie-broken by
`spawn_index`, `dispatch_id` and child session id. Attempt labels are labels,
not chronology: the report lists every admitted attempt's rank and states
whether label order agrees with time order. The delta itself is
`after − before` over named T2 counts, plus turn and episode-boundary changes
keyed by entry id with the same generation-pinned evidence used by
`ase work session`. Both sides are reopened against the pinned index and raw
bytes before a delta is produced; `--before-out` / `--after-out` write full
projections that `ase work reopen` can verify independently. A mismatch is
`evidence_unverified`, never a silent re-projection.

`ase analytics` (rule `runtime-analytics` / `runtime-analytics-v1`) consumes
the same Turn/Episode projection and reports per-turn stored facts beside it: nominal
`message.usage.cost.total` (catalog USD, never repriced, no `stats.db` read),
`message.duration`, `message.ttft`, and the half-open
`[message.timestamp, message.completedAt)` clock interval. Cost and duration
are sums over turns where the producer stored the field; the clock column is
the **union** of those intervals, so overlapping parallel calls count once in
clock time and once each in duration. Missing fields are counted in explicit
coverage and excluded from sums — never filled with zero or an inferred price.
The half-open `--start`/`--end` period uses `message.timestamp`, the same field
the pinned T1 comparison uses. A scoped analytics run projects every root
journal (and its subagents), so a journal that began before the period still
contributes its in-window turns with complete branch context; the `period`
totals in the report include only in-window turns, while `totals` stays the
unscoped all-branch reconciliation. Duration is accumulated as exact
fixed-point nanoseconds so the branch balance is exact at corpus scale instead
of failing on float rounding; the cost reconciliation gate is unchanged.
Project provenance comes only from the `<project>/<name>.jsonl` tree layout;
unattributed sessions land in an explicit `<unknown-project>` group, never
under a model or adapter candidate. Buckets are keyed by `(model, project)` and
report active-leaf, all-branch and abandoned statistics separately, with a
reconciliation that must agree with the turn counts. `ase analytics reopen`
re-checks generation, span, checksum, selector and the runtime facts themselves
against raw bytes. `ase work project` keeps its separate header-time selection
contract and is unaffected by analytics window scoping.

`ase fixture --out <DIR>` writes synthetic OMP/Codex fixtures (including a
`.zstd` file) for reproducible behavioral checks. No real transcript text is
written to fixtures or artifacts.

## Offline evaluation (`ase judge`)

`ase judge` runs the evaluation over a pinned, reopenable projection: one
unit is one journal session projected at its active graph leaf, and one
decision is an adjacent pair of human user messages on that path. The input
pin is the pinned cohort manifest plus its path-free units file and private
path map; the run re-reads every selected journal, checks the pinned size and
SHA-256, and refuses to continue if the active leaf, human-message count or
product user count differs from the pin.

```bash
ase judge run --root <PINNED_CORPUS> \
  --manifest <manifest.json> --units units.jsonl --private private.json \
  --store .tmp/p3.db --scope primary --workspace . --out .tmp/p3-run.json
ase judge verify --root <PINNED_CORPUS> --units units.jsonl \
  --private private.json --store .tmp/p3.db --include-verification
ase judge report --store .tmp/p3.db --include-verification
ase judge show --store .tmp/p3.db --unit <UNIT_ID> --decision 0
```

Three generators are separated by identity, version and cache namespace:

- `baseline-rules` (`baseline-rubric-v2`) is a deterministic bilingual
  lexical baseline. It applies the frozen rubric's observable priorities:
  explicit task reset/hand-off, an independent named deliverable, then
  same-deliverable refinement; acknowledgement-only messages and undecidable
  pairs stay `unknown`.
- The boundary LLM (`slm-boundary-v2`) and Judge (`judge-adjudicate-v2`)
  are separate stages. The Judge sees the baseline and boundary labels as
  codes, and its output is recorded separately.

An assertion carries its two exact source anchors (ordinal, entry id,
offset/length, line SHA-256), the rendered-input digest, the projection and
prompt/rule versions, and the declared generator identity. `insert` rejects an
assertion with missing or malformed evidence, re-opens both spans against the
source and refuses to store prose. The cache key is
`(input_sha256, generator, rule/prompt version, generator identity, view/render
version, verification namespace)`; it never contains a whole-journal digest,
so appending elsewhere in a journal does not invalidate a decision, while a
changed span, changed version or changed model identity does.

Model access has two backends. The first is a workspace-local process: a
runner program that reads one JSON request on stdin and writes one JSON
response on stdout:

```json
{"protocol":"ase-local-model/v1","stage":"slm","generator_id":"<id>",
 "prompt_version":"slm-boundary-v2","input_sha256":"<digest>","prompt":"<text>"}
```

```json
{"protocol":"ase-local-model/v1","label":"split|continue|unknown",
 "confidence":"high|medium|low","abstain_reason":null,
 "model_id":"<declared id>","model_sha256":"<declared 64-hex digest>"}
```

Configure it with `--slm-runner PROG --slm-arg ... --slm-model-id ID` plus
`--slm-model-sha256 HEX` or `--slm-weights <workspace file>` (the same flags
exist with the `judge-` prefix).

The second backend is one typed API profile resolved by `ase llm`:
`--slm-profile NAME` and `--judge-profile NAME` (plus optional
`--slm-api-timeout-ms` / `--judge-api-timeout-ms`). The profile selects exactly
one of the five protocols:

| API kind | Request shape | Credential location | Effort |
|---|---|---|---|
| `chat-completions` | `model`, `messages`, optional `reasoning_effort` | `Authorization: Bearer` | profile value, default `medium`; `none` is explicit |
| `responses` | `model`, `input`, optional `reasoning.effort` | `Authorization: Bearer` | profile value, default `medium`; `none` is explicit |
| `gemini` | native `systemInstruction` + `contents` | `x-goog-api-key` | never sent |
| `decision` (Console System One) | `model`, `state`, `questions` | `Authorization: Bearer` | never sent |
| `gemini-decision` | `model`, `input`, `questions` | `x-goog-api-key` | never sent |

The open-ended question criteria are the shared rubric options
(`split`/`continue`/`unknown`). A generation model must return one JSON object
with a verbal confidence (`high`/`medium`/`low`); a decision API returns a
calibrated probability. The store keeps those semantics separate: generation
values are written as `verbal:<level>`, decision values as
`probability:<value>`, and `judge show` reports a `confidence_semantics`
object. Neither value is translated into the other or thresholded by an
uncalibrated cutoff. Prompt/rubric versions were bumped to
`slm-boundary-v2`/`judge-adjudicate-v2`; historical `-v1` assertions remain
valid evidence and are not reinterpreted.

The transport performs exactly one request: no retry, no redirect (WinHTTP
redirects are disabled and a 3xx is rejected), and no credential forwarding.
On Windows TLS and system trust come from WinHTTP; other platforms support the
validated loopback `http` path and report HTTPS as unavailable. Errors are
stable codes only (`auth_rejected`, `bad_request`, `rate_limited`,
`redirect_refused`, typed rejection codes); a response body, prompt or key is
never placed in an error or report. The API cache identity covers the typed
API, normalized endpoint, provider, model, resolved effort and versioned wire
semantics, and never the credential, so one store cannot mix protocols or
effort configurations. Preparing a stage performs no network call, which is
why a warm replay makes zero model calls.

Session data leaves the workspace only with an explicit opt-in. A non-loopback
API profile is refused before any request unless the run passes
`--allow-remote-session-data`; the command then prints the stage, provider,
typed API and destination origin. Loopback synthetic runs need no opt-in.

A program outside the workspace, a missing program, an unreachable endpoint, a
timeout, an invalid response, an effort the protocol cannot carry or a
credential failure is recorded as `unavailable`/`failed`/`rejected`; the stage
produces no assertions and never falls back to the baseline, fixture output or
a different model/effort. `show`, `report` and `verify` are read-only, do not
construct a runner and cannot reach a model or a network.

`ase judge fixture` and `ase judge contract-runner` support the synthetic
contract check. The contract runner is a protocol double; runs that use it must
pass `--verification-only`, which uses a separate cache namespace and report
partition, so its labels are never read as evaluation results.

Current status: the baseline ran on the full pinned cohort. A narrow
real-inference verification (two units, five decisions) ran cold and warm
before the Ollama backend was removed. T5 verification replaced that backend
with a loopback synthetic check of all five protocols: auth header location,
exact body shape, effort omission, typed parse, redacted failures,
cold/warm cache and zero-call query paths (90/90 checks, no provider request).
Live synthetic smoke passed for Go Chat Completions, Go Responses, and
Gemini Decisions. It does not establish full-cohort quality or cache
behavior; T3 measures those separately. The evaluation does not read session
recaps, and `unknown` is never merged into either positive label.

**Local measurement update (supersedes the earlier unmeasured-status sentence):**
the cache/zero-call gate was measured in a workspace-local experiment on all 1,400 pinned decisions: both model stages ran
cold with a temporary pre-send `redact-v1` policy and a fail-closed payload
allowlist, the unchanged replay made 0 outbound calls with an identical
assertion digest, and `verify` reopened 4,200/4,200 anchors. The experiment
patch was reverted; the shipped CLI has no judge redaction option and keeps
policy `none`. The 26-decision ablation is descriptive against frozen hand
labels with no quality threshold, so no cohort quality verdict or
historical-recap equivalence follows. Public source publishes the judge and
evidence-bundle
code and synthetic fixtures, not these measurement records.

## LLM API configuration (`ase llm`)

`ase llm` is the configuration/credential/transport boundary for the optional
Judge API backends. It decides which full request endpoint, typed API,
model, effort and credential one call uses, and implements exactly one typed
request for that protocol. Every command except `ase llm smoke` is local and
opens no socket. `smoke` sends one fixed synthetic rubric pair and never sends
journal, database, recap or excerpt data. Endpoint URLs and provider names are
supplied by the owner; there is no catalog or endpoint inference.

Five typed APIs are configured independently:

| API kind | Family | Default effort |
|---|---|---|
| `chat-completions` | OpenAI-compatible generation | `medium` |
| `responses` | OpenAI-compatible generation | `medium` |
| `decision` | Console System One Decision | none |
| `gemini` | native Gemini generation (distinct endpoint/auth) | none |
| `gemini-decision` | native Gemini Decisions (distinct endpoint/auth) | none |

Only the two OpenAI-compatible generation protocols use the OpenAI effort
parameter. `none` is a valid explicit value there (for a target that advertises
only `none`/`high`) and is sent verbatim; there is no automatic conversion or
retry with another effort. Native Gemini generation and both decision kinds
never resolve or send an effort, and `OPENAI_API_EFFORT` is reported as ignored
for them.

Supported commands:

```bash
ase llm config                     # redacted local status (no network)
ase llm config set --profile chat --provider openai \
  --type chat-completions \
  --endpoint https://api.openai.com/v1/chat/completions \
  --model <MODEL> --use
ase llm config set --profile gemini --provider gemini-native --type gemini \
  --endpoint <FULL_GEMINI_GENERATE_URL> --model <MODEL>
ase llm config set --profile gemini-decision --provider gemini-native \
  --type gemini-decision --endpoint <FULL_GEMINI_DECISION_URL> --model <MODEL>
ase llm config use --profile responses
ase llm config enroll --profile chat      # one-time hidden key input
ase llm key set --profile chat            # same hidden input, key only
ase llm key clear --profile chat
ase llm status --json
ase llm smoke --profile chat --json   # fixed synthetic wire check
```

`config enroll` stores the key for the provider and endpoint origin in the
profile it saves. Runtime environment overrides do not redirect enrollment.

`--json` emits a machine-readable redacted report (profile, provider, API
type, endpoint, normalized origin, model, effort, credential
presence/backend/family, checks and `network: not_used`). It never contains
the key.

The config schema is version 2: every profile must name an explicit
`provider`. A version-1 file is rejected; there is no compatibility shim for
the unpublished early candidate.

### One-time enrollment (owner action)

Run this once in PowerShell or a native terminal, as the Windows user who will
run `ase`:

```powershell
ase llm config enroll --profile chat --provider openai --type chat-completions `
  --endpoint https://api.openai.com/v1/chat/completions --model <MODEL>
```

The command prints a prompt and reads the key from hidden stdin. The key must
not be passed as an argument: `ase llm key set --key VALUE`, `--key=VALUE` and a
positional key are rejected without echoing the value. The prompt is written to
stderr; the key is never written to stdout, shell history (there is no argument
to record), the config file, the repository or a debug/error payload.

On Windows the key is stored as a **Windows Credential Manager Generic
credential** (`CRED_PERSIST_LOCAL_MACHINE`, user-scoped). Its target is derived
from the explicit provider and the normalized endpoint origin, and both are
verified on read. A later process under the same Windows user resolves it
without any key environment variable and without re-entering the key. The
config file holds only non-secret settings:

```json
{
  "version": 2,
  "active": "chat",
  "profiles": {
    "chat": {
      "provider": "openai",
      "api": "chat-completions",
      "endpoint": "https://api.openai.com/v1/chat/completions",
      "model": "<MODEL>"
    }
  }
}
```

If a platform has no supported native user credential store, `ase llm key set`
and `ase llm key clear` fail explicitly (`credential_platform_unsupported`)
before asking for a key; there is no plaintext fallback. A process key override
remains usable where the CLI supports it.

### Provider scope and credential identity

Provider names are explicit, user-defined aliases such as `openai`,
`opencode-go`, `gemini-native` or a custom name. There is no catalog or
network discovery. A saved credential is identified by the pair
**`(provider, normalized origin)`**:

- the same provider on the same trusted origin shares one key across profiles,
  so `chat` and `responses` above need one enrollment;
- distinct providers on the same origin are isolated: `openai` and
  `opencode-go` pointing at the same host do not share a saved key;
- switching a profile's provider or changing its host/port changes the lookup
  pair, so the previous key is never reused and status reports the missing
  credential for the new pair;
- `ase llm key clear` removes only that provider+origin entry.

### Environment overrides

`ASE_LLM_PROVIDER` sets or overrides the explicit provider scope (required for
environment-only configuration). `OPENAI_API_TYPE`,
`OPENAI_API_ENDPOINT`, `OPENAI_API_MODEL` and `OPENAI_API_EFFORT` override the
selected profile field by field; whitespace-only values are ignored. Status
lists the applied override names (never values).

Process key overrides are ephemeral and never written to the store:

- `OPENAI_API_KEY` applies only to `chat-completions`, `responses` and
  `decision`;
- `GEMINI_API_KEY` applies only to `gemini` and `gemini-decision`;
- the two families never fall back to each other.

A nonempty key for the selected kind overrides its stored key; otherwise the
stored `(provider, origin)` key is used. A provider override or endpoint
override therefore cannot silently reuse a saved key from another scope.

### Endpoint validation

A profile stores the full request URL. It must be an absolute `http`/`https`
URL with a non-root request path; user-info (`user:password@host`), fragments
and credential-shaped query parameters (`api_key`, `token`, `sig`, …) are
rejected. Plain `http` is accepted only for loopback hosts; remote hosts need
`https`. The URL is normalized (lowercased host, default port omitted) and the
credential origin is `scheme://host:port`, always with an explicit port. Native
Gemini generation and Gemini Decision endpoints are owner-supplied full URLs;
they are never inferred from a catalog or model name.

### Typed transport and synthetic smoke

`ase-llm` separates resolution, protocol framing and the single network call:

- `resolve_profile(env, profile)` — explicit provider, typed API, validated
  endpoint, model and effort with environment overrides applied;
- `resolve(env, profile)` / `resolve_with_store(...)` — the same plus an
  `ApiKey` for the exact `(provider, origin)` pair (or the kind's process key
  family); the key is reachable only through `expose()` and its `Debug`
  representation is redacted;
- `wire` — builds one request body for the selected typed API and parses one
  response envelope. Auth location is protocol-defined (`Authorization:
  Bearer` or `x-goog-api-key`); the credential is not part of the request
  description;
- `transport` — the only network path. Windows uses WinHTTP with redirects
  disabled; other platforms accept the validated loopback `http` path and
  report HTTPS as unavailable. There is no plaintext fallback for a remote
  host and no proxy configured by the program;
- `call` — exactly one send and one typed parse, no retry. A 3xx is
  `redirect_refused`; a 4xx/5xx maps to a stable code without the response
  body; a malformed envelope is a typed rejection code. The cache identity is
  a hash of typed API, normalized endpoint, provider, model, effort and
  versioned wire/parse semantics, never a key;
- `status(env, profile)` — the redacted local status report; it opens no
  socket.

`ase llm smoke [--profile NAME] [--timeout-ms N] [--json]` sends one fixed
synthetic rubric pair through the selected protocol and prints the auth header
name, whether an effort was sent, the request's top-level body keys, the typed
label/confidence semantic, usage and model. It prints no credential, prompt or
response body. It is the only command that opens a socket.

On failure, `smoke` exits nonzero and prints a separate
`ase-llm-smoke-diagnostic/v1` JSON report on stdout. It distinguishes explicit
incomplete/truncated Responses state (with an allowlisted reason), refusal or
missing output text, and text that does not map to the rubric. The report carries
only fixed codes, the HTTP status when known, output-part presence flags, and
the extracted text's length and JSON delimiter presence — never a credential,
endpoint query, prompt, response body, output/refusal text or provider error
text. No retry or fallback request is made; the success report schema is
unchanged.

For generation APIs (`chat-completions`, `responses`, `gemini`), the fixed
synthetic instruction requests the rubric JSON object explicitly: one object
with `label` (`split`/`continue`/`unknown`), `confidence`
(`high`/`medium`/`low`) and `abstain_reason` (a short reason only for
`unknown`, otherwise `null`). The two decision APIs keep the typed-choice
instruction and their own answer contract. This is a prompt instruction only:
no JSON response format is forced on the request and no retry is made.

`ase judge run` uses the same resolution: `--slm-profile NAME` and
`--judge-profile NAME`. A request that carries the rendered decision pair to a
non-loopback endpoint is refused unless the run passes
`--allow-remote-session-data`; the command then prints the stage, provider,
typed API and destination origin. Loopback smoke and loopback evaluation need
no opt-in, and no synthetic fixture in this repository is a live provider
request.

### Verification status

`ase llm status` is the local check: it validates configuration shape, the
provider+origin binding, the endpoint and the credential presence, and reports
`network: not_used`. The T4 R2 verification used an isolated
`ASE_LLM_CREDENTIAL_STORE=dpapi-file:<dir>` store inside the workspace (DPAPI,
same Windows user) to prove cross-process persistence without writing the real
user credential store; the native Windows Credential Manager write/read path
was compiled and its missing-entry read path was exercised, but no real user
store entry was created by the agent. The owner enrollment above is the
remaining native action.

T5 verification ran the release CLI against a synthetic loopback server for
all five protocols: auth header location and value, exact request body shape,
effort presence/omission, typed parse and confidence semantic, error/body
redaction, redirect refusal without following, no retry, cold/warm cache per
protocol, cache namespace separation on a shared store, zero-call
`report`/`show`/`verify` paths, and refusal of a non-loopback profile without
opt-in. It made no provider request and used no real key. The script and
machine-readable result are
`artifacts/sprint-6/ASE-S6-T5/smoke_t5.py` and `evidence-smoke-a1.json`.

## Source coordinates

- JSONL: `(file, off, len)` addresses raw file bytes.
- `.jsonl.zst`: the same tuple addresses the **decompressed** stream; the
  compressed byte offset is deliberately not used as an address.

`show` reports encoding, coordinate system, freshness, and the stored checksum.
With `--explain` or `--content`, it rereads the span and checks that checksum.

`resolve task:<TOKEN>` reconciles the derived task signals under rule
`task-identity-reconciliation` / `task-resolve-v2`. A project-scoped key comes
from a parsed `file-name-grammar` / `naming-v1` candidate or from one unique,
consistent native dispatch join under `task-dispatch-provenance` /
`dispatch-v1`; a `task_id=` marker classified as `assignment_context` by
`marker-v1` corroborates that key only inside the same file. A dispatch join
needs exactly one native `task_dispatch_v1` parent record and one
`task_dispatch_child_v1` link for the same runtime `dispatch_id`, structured
`parent_session_id`/`session_id` values that match the journals containing the
records, distinct parent and child journals, and both raw spans re-verified
against indexed bytes. It returns the executing child session with
`project`/`task`/`attempt` and both record spans. Output is one of:

- `assigned`: exactly one project-scoped key and no layer conflict.
- `ambiguous`: several project-scoped keys carry the token, or the file's own
    prose assigns only different task ids; every candidate is returned with
    `support`/`refute`/`context` evidence and reasons.
- `unresolved`: no project-scoped naming key or unique dispatch join;
    assignment markers without a project, mentions, literal path hits and
    `parentSession` edges remain candidates or context, never an assignment.
    Dispatch claims that are malformed, missing a counterpart, duplicated,
    wrong-session, mismatched, contradicted by the child's own parsed name or
    competing for one child are reported as rejected claims and assign nothing.

Each evidence item is labelled `observed` or `derived` with its rule
name/version. Marker evidence is pinned to one generation, event, span,
selector and block digest and can be reopened with `ase open`; prose text is
never emitted. Dispatch evidence carries the parent and child record spans,
both checksums and a `reopen` command for each. A missing/tombstoned postings
layer is reported as `marker_layer.status = "unavailable"` rather than as "no
markers". Project is part of the key, so the same token in different projects
is never merged. Dispatch records are matched by exact runtime task identity,
never by a shared numeric suffix, path, worker name or prose echo. A journal
with no tracking records keeps the previous naming/marker behaviour. The
address index already stores every custom record's span and `customType` tag,
so this integration needs no index format change; existing indexes remain
readable because the type-string boundary is derived from the type table.
Rebuild only when the index predates the dispatch records themselves.

`ase audit naming` groups indexed file names by parent directory and reports
coverage, clusters, unparsed names and abstentions under rule `file-name-grammar`
version `naming-v1`. An explicit `Task`/`T` marker decides the project/task
split; without one, multiple plausible digit boundaries stay as tied candidates
and no task key is emitted. Attempt numbers are labels, not chronology. The
audit reads path/session metadata from the address index only: no transcript,
span content, postings or model is consulted. Project and parent remain in
every task key, so cross-project and cross-parent collisions are reported
rather than merged.

`ase audit delegation` emits versioned `delegated_from` edges from
`parentSession` declarations (rule `parent-session-delegation` /
`delegation-v1`). A target is accepted only by exact session-id uniqueness in
the index; missing, duplicated or self targets stay unlinked with every
candidate retained, and neither file order nor path proximity selects a
target. Payload and path-derived observations are reported separately: a
header span only locates the source file and is not claimed as proof that the
id text is in a payload. Every edge carries source/target locators and an
explicit null task key, so a parent alone is a delegation edge, never a task
assignment. No transcript, prose or model is read.

`ase audit markers` (rule `task-id-marker` / `marker-v1`) observes `task_id=`
markers in allowlisted user/assistant prose blocks of one published
generation. It re-derives each block from the pinned raw span with its
versioned selector and scans it only when the span hash and block digest
verify; thinking, tool output, system and call-signature text are never
scanned. Every occurrence is kept separately with generation, event, span,
selector, in-block offsets and rule version, and is classified from local
assignment versus mention cues as `assignment_context`, `mention` or
`ambiguous` (abstain). Output is metadata-only: no prose text is stored or
emitted, and the exact block is reopened with `ase open` using the same
generation. No TaskKey, `assigned_to` edge or session-level assignment is
produced by the marker audit itself; `resolve task:` performs that
reconciliation with naming and delegation evidence.

`ase adapter omp-stock` reads unmodified OMP stock journals and emits
advisory claims in three independent layers: `parent_child` (a `task` call in
one journal and a child journal whose `parentSession` names it), `repo`
(declared `cwd`, a derived candidate-only `repo_key`, and bounded path mentions
under the workspace), and `task` (a
structured child label, or a task token corroborated by the parent prompt and
the child handoff). Each claim carries `confirmed`, `candidate`, `missing`,
`unavailable` or `conflict`; the layers never promote one another. Multi-child
task calls keep one scoped claim per sibling, and multi-task sessions are
reported as `multi_task` rather than merged into one key. Mention-only prose
and name-only about a child stay candidates and are never counted as
confirmed.

Claims are stored only when `--out` is supplied. They never write resolver
assignment, motif membership, the address index or a source journal; ordinary
`show`, `search`, `resolve`, `motif` and `verify` behavior is unchanged.
`--reopen CLAIMS.json` pins the claims generation, re-reads every spanned
byte, checks the index FNV checksum and strong line digest, and re-runs the
versioned selector/decoder before trusting the value. Any mismatch marks that
claim `stale` and keeps it for inspection. Confirmations depend on observed
journal fields and, optionally, a child handoff `.md`; missing producer
metadata is `missing`, not an error. `ase fixture-adapter` writes a synthetic
stock-shaped project covering matching, multi-child, missing, conflict and
multi-task cases without real transcript content.

## Search layer

`ase index` defaults to `--layer address` to preserve the original address-only behavior. Use
`--layer postings` to add postings to an existing address, or `--layer all` to
derive both layers from one decode pass. Each successful publication writes an
immutable generation directory and replaces one sidecar pointer
(`<index>.pub.json`) atomically. An address-only rebuild tombstones the pointer.

`search` returns event, span, versioned selector, role, generation and ranking
metadata only — no stored prose or snippet. Thinking, tool output, system and
unknown blocks are excluded from FTS. Missing, tombstoned or mismatched
sidecars return `layer_unavailable` with exit code 4; `show`/`resolve`/`verify`
still read the binary address.

`open` requires the `--generation` value from the search hit and rejects a
mismatch before reading raw. It resolves `(generation, event, selector)`,
re-reads the raw span and re-derives the block from raw JSON. It reports
`verified_against_index_snapshot` only when both the span checksum and the
block digest match; edited raw is reported as `unverified` even though the
search hit is still served.

OMP call blocks use the same argument extraction when indexed and reopened: a
tool call's legacy `args` field wins over `arguments`, and the selected field's
parsed JSON value is used unchanged (object keys become sorted call-shape
classes, JSON strings stay scalars). Reopened call digests therefore match the
indexed block without changing the digest algorithm or index format.

`shingle` reports versioned `k=3..5` CallSig windows. A sequence continues only along a same-session explicit parent chain; forks, unresolved parents and session boundaries start a new sequence and are abstained from. Each row carries its constituent event/selector evidence.

`ase-motif` turns that substrate into motif evidence. Membership for a task token comes from one native dispatch join per attempt: name grammar, path matches and `parentSession` declarations are never passed in, so they cannot widen it. Cohort members may be addressed as `PROJECT|TASK`, the locked `(project, task)` sampling key; the project scope filters the same dispatch validator before reconciliation, so one token in two projects stays two independent samples, while a bare token remains ambiguous and never falls back to a project. For each admitted child session it rebuilds the writer's parent-chain sequences from the call blocks of one pinned generation and keeps every call as `(generation, event, selector, span, block_hash)`; a sequence never crosses a fork or a session boundary, and each call reopens through the same generation, span-checksum, selector and digest checks as `ase open`. Families are the deterministic single-linkage components of a Jaccard overlap over the sequence's `k=3..5` shingle tokens, with `tau` a caller parameter rather than a measured gate.

`ase motif` aligns each family sequence to the family's deterministic skeleton with a banded Needleman–Wunsch pass. Insertions (a call the sequence added) and substitutions (a different call at the same column) are reported as separate counts; a sequence with substitutions inside `--max-substitutions` and no missing column is a near miss, not support. Support is counted per **task**, so repeated attempts and repeated occurrences of one task are one observation. Duplicate sequence text is deduped before counting under rule `motif-sequence-text` / `seq-text-v1`: `text_hash` is the SHA-256 of a sequence's reopened CallSig strings, and a copy is deduped only within the same resolved task key, so distinct tasks are never merged and no sequence crosses a branch or session boundary. The canonical copy is counted; duplicate copies are listed in `deduplicated_sequences`, excluded from family/null/sequence-denominator counting, and every occurrence (including the duplicate) remains reopenable evidence. Dedupe operates on whole projected sequences, never on the calls inside one: call order is preserved for alignment, as required by the data contract. `support_relaxed[g]` allows at most `g` skipped calls between motif columns and is monotonic in `g` by construction. Each motif reports its denominator (`tasks_total`, `tasks_in_family`, `sequences_in_family`, `sequences_deduplicated`) and every excluded task with a reason. A fixed-seed (`null_seed`) permutation of each sequence's call order supplies the null model: `observed`, `expected`, `stddev`, `z` and `lift`. Every occurrence call carries its generation, event, selector, span and digest and was reopened from raw bytes before it entered the motif; `ase open` can re-verify one call individually.
