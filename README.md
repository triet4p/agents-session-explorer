# agents-session-explorer — P0a addresses, P0b derived search

P0a is a standalone, address-only source index for OMP/Codex session files. It
answers *where* an observation came from. P0b adds a generation-pinned,
contentless full-text and shingle layer for allowlisted prose and tool-call
signatures. P0c resolves task identity from file-name grammar, `parentSession`
delegation and `task_id` prose markers with inspectable evidence. No layer
calls a model.

## Design documentation

- [Use cases and requirements](docs/USE_CASE.md) — user workflows and acceptance criteria.
- [Scope](docs/SCOPE.md) — supported behavior, non-goals, and phase boundaries.
- [Static architecture](docs/STATIC_ARCHITECTURE.md) — current components and proposed changes.
- [Data contracts](docs/DATA_CONTRACTS.md) — provenance, source coordinates, and derived data.
- [P0b verification](artifacts/P0b/summary.md) — fixture proofs, release measurements and limits.
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
replaced by synthetic fixtures); no historic or P0d claim is made.

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

# P0c-2 naming grammar: synthetic fixture, metadata-only audit
ase fixture-naming --out .tmp/naming
ase index --omp-root .tmp/naming/omp --codex-root .tmp/naming/codex --index .tmp/naming/ase.index
ase audit naming --index .tmp/naming/ase.index --json --explain

# P0c-3 parentSession delegation: synthetic fixture, metadata-only audit
ase fixture-delegation --out .tmp/delegation
ase index --omp-root .tmp/delegation/omp --codex-root .tmp/delegation/codex --index .tmp/delegation/ase.index
ase audit delegation --index .tmp/delegation/ase.index --json --explain

# P0c-4 task_id markers: synthetic fixture, metadata-only audit
ase fixture-markers --out .tmp/markers
ase index --layer all --omp-root .tmp/markers/omp --codex-root .tmp/markers/codex --index .tmp/markers/ase.index
ase audit markers --index .tmp/markers/ase.index --json --explain
ase open --event N --selector S --generation G --index .tmp/markers/ase.index --content

# P0c-5 task identity reconciliation: synthetic fixture, metadata-only resolve
ase fixture-resolve --out .tmp/resolve
ase index --layer all --omp-root .tmp/resolve/omp --codex-root .tmp/resolve/codex --index .tmp/resolve/ase.index
ase resolve task:101 --index .tmp/resolve/ase.index --json --explain

# P0c-9 native dispatch provenance: synthetic producer-shaped fixture
ase fixture-dispatch --out .tmp/dispatch
ase index --layer all --omp-root .tmp/dispatch/omp --codex-root .tmp/dispatch/codex --index .tmp/dispatch/ase.index
ase resolve task:410 --index .tmp/dispatch/ase.index --json --explain
ase resolve task:10410 --index .tmp/dispatch/ase.index --json --explain
ase show span:<FILE>:<OFF>:<LEN> --index .tmp/dispatch/ase.index --json --explain

# P0d-2 motif alignment and support: synthetic fixture, occurrence reopen
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

# P1 Turn/Episode projection: active leaf by default, explicit older leaf on request
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
ase work delta --task T701 --project S31 --before s4t3-before --after s4t3-after \
  --omp-root .tmp/delta/omp --index .tmp/delta/ase.index \
  --out .tmp/delta/delta.json --before-out .tmp/delta/before.json \
  --after-out .tmp/delta/after.json --json
ase work reopen .tmp/delta/before.json --index .tmp/delta/ase.index --json
ase work reopen .tmp/delta/after.json --index .tmp/delta/ase.index --json

# P2 runtime analytics: nominal stored cost, duration vs clock union, coverage
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

Defaults: `--index ase.index`; OMP root `C:/Users/admin/.omp/agent/sessions`;
Codex root `C:/Users/admin/.codex/sessions`.

`ase work project` derives `Turn`/`Episode` rows under rule
`omp-turn-episode` / `turn-episode-v1`, compatible with the locked Sprint-4
reference rule `s4t1-turn-v1`. A turn is one assistant `message` entry with a
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
the same P1 projection and reports per-turn stored facts beside it: nominal
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
reconciliation that must agree with the P1 turn counts. `ase analytics reopen`
re-checks generation, span, checksum, selector and the runtime facts themselves
against raw bytes. `ase work project` keeps its separate header-time selection
contract and is unaffected by analytics window scoping.

`ase fixture --out <DIR>` writes synthetic OMP/Codex fixtures (including a
`.zstd` file) for reproducible behavioral checks. No real transcript text is
written to fixtures or artifacts.

## Source coordinates

- JSONL: `(file, off, len)` addresses raw file bytes.
- `.jsonl.zst`: the same tuple addresses the **decompressed** stream; the
  compressed byte offset is deliberately not used as an address.

`show` reports encoding, coordinate system, freshness, and the stored checksum.
With `--explain` or `--content`, it rereads the span and checks that checksum.

`resolve task:<TOKEN>` reconciles the P0c signals under rule
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

`ase index` defaults to `--layer address` to preserve P0a behavior. Use
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
