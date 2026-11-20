# agents-session-explorer — P0a addresses, P0b derived search

P0a is a standalone, address-only source index for OMP/Codex session files. It
answers *where* an observation came from. P0b adds a generation-pinned,
contentless full-text and shingle layer for allowlisted prose and tool-call
signatures. Neither layer calls a model.

## Design documentation

- [Use cases and requirements](docs/USE_CASE.md) — user workflows and acceptance criteria.
- [Scope](docs/SCOPE.md) — supported behavior, non-goals, and phase boundaries.
- [Static architecture](docs/STATIC_ARCHITECTURE.md) — current components and proposed changes.
- [Data contracts](docs/DATA_CONTRACTS.md) — provenance, source coordinates, and derived data.
- [P0b verification](artifacts/P0b/summary.md) — fixture proofs, release measurements and limits.
- [Initialization record](docs/INITIALIZATION.md) — original design and corpus observations.

## Build

```bash
cargo build --release
```

Rust 1.91.1 or newer. Produces `target/release/ase` (`ase.exe` on Windows).

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
ase resolve task:<TOKEN> --index ase.index --json --explain   # candidates only

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
`resolve task:` returns candidate path matches only; P0c owns task identity
and assignment.

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
