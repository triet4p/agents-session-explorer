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

# Re-read raw bytes at every indexed span
ase verify --all --index ase.index --json

# Deterministic in-process show-latency sample over OMP spans
ase bench-show --n 2000 --seed 1 --index ase.index --json
```

Defaults: `--index ase.index`; OMP root `C:/Users/admin/.omp/agent/sessions`;
Codex root `C:/Users/admin/.codex/sessions`.

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

`shingle` reports versioned `k=3..5` CallSig windows. A sequence continues only along a same-session explicit parent chain; forks, unresolved parents and session boundaries start a new sequence and are abstained from. Each row carries its constituent event/selector evidence.

`ase-motif` (library only in this step; no CLI yet) turns that substrate into motif evidence. Membership for a task token comes from one native dispatch join per attempt: name grammar, path matches and `parentSession` declarations are never passed in, so they cannot widen it. For each admitted child session it rebuilds the writer's parent-chain sequences from the call blocks of one pinned generation and keeps every call as `(generation, event, selector, span, block_hash)`; a sequence never crosses a fork or a session boundary, and each call reopens through the same generation, span-checksum, selector and digest checks as `ase open`. Families are the deterministic single-linkage components of a Jaccard overlap over the sequence's `k=3..5` shingle tokens, with `tau` a caller parameter rather than a measured gate.
