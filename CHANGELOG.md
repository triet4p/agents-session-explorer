# Changelog

## [Unreleased]

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
