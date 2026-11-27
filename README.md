# agents-session-explorer

`ase` is a local, read-only index over OMP and Codex session journals. It records
*where* an observation came from — file, byte span, event, selector and generation —
and answers search, navigation and evidence questions without sending your session
text anywhere. The index has two layers: an address layer over raw journal bytes and
an optional derived search layer. Every search hit can be reopened against the raw
bytes it was built from, and an edited source is reported as `unverified` instead of
silently trusted.

Resolving task identity, projecting turns/episodes, comparing attempts, motif
analysis, evidence-bundle export/verify, and the optional offline `judge` layer all
reuse the same reopenable evidence. No query path calls a model. `ase judge` and
`ase llm` are the only model-related commands, and they are explicit opt-in.

## Download (Windows x64)

Latest release: [v0.7.0](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.7.0).

| Asset | Contents |
|---|---|
| `ase-0.7.0-windows-x64.zip` | `ase.exe`, `README.md`, `THIRD_PARTY_NOTICES.txt` |
| `SHA256SUMS.txt` | SHA-256 of the archive and its members |

Requirements:

- Windows 10 or 11, x64. This is the tested platform boundary; Windows Server,
  Windows 10, ARM64 and 32-bit Windows were not tested.
- Microsoft Visual C++ 2015–2022 Redistributable (x64), which provides
  `VCRUNTIME140.dll`. It is not bundled in the archive. Install it from Microsoft's
  official `https://aka.ms/vc14/vc_redist.x64.exe` page if `ase.exe` fails to start.
- No Rust, Cargo, Git, Visual Studio, Windows SDK or source checkout is needed to
  run the binary.

**The binary is unsigned.** Windows SmartScreen may warn on first run. If it does,
do not disable or bypass the protection; verify the SHA-256 first, and decide for
yourself whether your policy allows an unsigned build. A hash match proves only that
the bytes equal the published reference, not who produced them.

### Verify, extract and run

```powershell
$zip = Join-Path $HOME 'Downloads\ase-0.7.0-windows-x64.zip'
$sums = Join-Path $HOME 'Downloads\SHA256SUMS.txt'   # download both assets first

$expected = ((Get-Content -LiteralPath $sums | Select-String 'ase-0.7.0-windows-x64.zip').Line -split '\s+')[0]
$actual   = ((certutil -hashfile $zip SHA256)[1] -replace ' ', '').ToLowerInvariant()
if ($actual -ne $expected.ToLowerInvariant()) {
    throw "STOP: archive SHA-256 mismatch. Do not extract or run."
}

$dest = Join-Path $HOME 'ase-0.7.0'
Expand-Archive -LiteralPath $zip -DestinationPath $dest
$ase  = Join-Path $dest 'ase.exe'
& $ase --help
if ($LASTEXITCODE -ne 0) { throw 'ase --help failed' }
```

There is no installer: extraction creates or changes nothing outside the destination
directory, and `ase` does not modify `PATH`, the registry or a service. To remove it,
delete that directory and any index directory you created; see
[Uninstall](#uninstall).

`certutil -hashfile` is used because it ships with Windows and does not depend on
optional PowerShell modules. Compare the digest before extracting anything.

## Quick start

The commands below create a synthetic fixture and index only that fixture through
explicit roots. They do not read your real session directories.

```powershell
$work    = Join-Path $HOME 'ase-quickstart'
$fixture = Join-Path $work 'fixture'
$index   = Join-Path $work 'ase.index'
New-Item -ItemType Directory -Force -Path $work | Out-Null

# 1. Synthetic OMP + Codex journals (writes only below --out)
& $ase fixture --out $fixture

# 2. Index with BOTH roots named explicitly
& $ase index --layer all `
    --omp-root   (Join-Path $fixture 'omp') `
    --codex-root (Join-Path $fixture 'codex') `
    --index $index --json

# 3. Search returns metadata and ready-to-use open flags
$result = & $ase search 'SYNTHETIC USER TOKEN' --index $index --json --open-args | ConvertFrom-Json
$hit    = $result.hits[0]

# 4. Reopen the exact block from raw bytes
& $ase open @($hit.open_args.flags) --json
```

Expect `publication.status = "ready"`, `files = 3`, `sessions = 3`, and
`status = "verified_against_index_snapshot"` with `raw_span_hash_match` and
`block_digest_match` both `true` for `open`.

**Always pass `--omp-root` and `--codex-root` explicitly.** Without them, `ase index`
falls back to `ASE_OMP_ROOT` / `ASE_CODEX_ROOT` and then to
`$HOME\.omp\agent\sessions` and `$HOME\.codex\sessions`, which can index your real
profile. A missing root and an empty root both index zero files and still report
`ready`, so check the root contents before indexing.

The same flow works with a build from source (`cargo build --release --locked`),
using `.\target\release\ase.exe` instead of the downloaded `$ase`.

## Build from source

Requirements:

- Rust 1.91.1 or newer with the `x86_64-pc-windows-msvc` target, plus the MSVC
  linker and Windows SDK from Visual Studio Build Tools (Windows), or the platform
  C toolchain for other targets.
- Git, to fetch the tagged source.

```bash
git clone --branch v0.7.0 https://github.com/triet4p/agents-session-explorer.git
cd agents-session-explorer
cargo build --release --locked --target x86_64-pc-windows-msvc
# Windows output: target\x86_64-pc-windows-msvc\release\ase.exe
```

On other platforms, drop `--target` and the MSVC requirement; only Windows x64 has
release evidence. `zstd` and SQLite C sources are compiled into the binary, so the
C toolchain is required at build time but no extra DLLs are needed at runtime
beyond the Windows system components and `VCRUNTIME140.dll`.

The released binary was built from the `v0.7.0` tag with path remapping
(`--remap-path-prefix` for Rust; `/pathmap` for the MSVC C compiler) so the image
does not embed the build host's source paths. The build is repeatable in procedure
but not byte-for-byte reproducible: PE timestamps and PDB identity differ between
runs, so rebuilding will not reproduce the published digest.

## Command map

`ase --help` prints the full grammar. The main areas are:

| Area | Commands and purpose |
|---|---|
| Index | `ase index` builds the address layer and optional postings layer; one generation is published atomically behind `<index>.pub.json`. |
| Search and evidence | `ase search`, `ase open`, `ase show`, `ase shingle`, `ase verify` return metadata and reopen exact spans; `--content` is the only way to print source text. |
| Navigation | `ase search --open-args` and `ase open --session-args` emit exact `open`/`show`/`resolve`/`work` flags for a verified block; ambiguous session ids are refused. |
| Identity | `ase resolve`, `ase audit naming`, `ase audit delegation`, `ase audit markers` reconcile naming grammar, delegation and `task_id=` markers with inspectable evidence. |
| Work and analytics | `ase work project\|session\|reopen\|compare\|delta` projects turns/episodes; `ase analytics` reports stored cost, duration and clock unions. |
| Patterns | `ase motif`, `ase shingle`, and `ase adapter omp-stock` produce advisory, evidence-pinned claims; adapter claims never write membership. |
| Evidence exchange | `ase export motif:<id>`, `ase verify <bundle>`, `ase import verdicts`, `ase join skill-gap` move path-free evidence bundles without shipping source paths. |
| Optional model layer | `ase judge`, `ase llm` run offline evaluation against a local runner or an explicitly configured API profile. |
| Fixtures | `ase fixture`, `ase fixture-*` write synthetic journals for every layer; they contain no real transcript text. |

Run any subcommand with `--help` for its exact flags, for example
`ase work --help` or `ase judge --help`.

## How data is stored

- JSONL spans are addressed as `(file, off, len)` in raw file bytes. A
  `.jsonl.zst` file is addressed by its **decompressed** stream, never by compressed
  offsets.
- `ase index --layer address` writes address data only. `--layer postings` adds the
  derived search layer; `--layer all` derives both in one decode pass.
- Each successful publication writes an immutable generation directory and replaces
  one sidecar pointer atomically. A mismatched or missing sidecar returns
  `layer_unavailable` for search while address-based commands keep working.
- `search` returns coordinates, ranking metadata and generation — no stored prose or
  snippet. `open` re-reads the raw span, re-derives the block and reports
  `verified_against_index_snapshot` only when the span checksum and block digest
  both match.

## Safety and privacy defaults

- Session roots are never discovered implicitly when you pass `--omp-root` and
  `--codex-root`; explicit roots are the documented path for any real corpus.
- Query, search, resolve, work, motif and analytics paths do not call a model or the
  network. `ase judge` makes a model call only for an explicit runner or configured
  profile; remote session-data egress requires `--allow-remote-session-data`.
- No telemetry is collected or sent.
- Bundles contain path-free observations and opaque aliases; the alias map stays in a
  file outside the bundle.
- `ase fixture*` output is synthetic and contains no real transcript text.

## Limitations

- The Windows x64 binary is **unsigned** and accepted on the owner's approval of
  fixture and build-host evidence. It has not been run on a clean or freshly
  installed Windows machine, a restored VM, or a new user profile, so a missing
  VC++ runtime, SmartScreen reputation and enterprise policy behavior are untested.
- Only Windows 11 x64 was exercised. Windows 10, Windows Server, ARM64 and 32-bit
  Windows are untested, as are PowerShell 7-specific behavior and long-running use.
- The build embeds no build-host source paths after remapping, but it is not
  byte-reproducible: PE timestamps and PDB identity change per build.
- No real-user pilot or real corpus was run for this release. Counts quoted in
  older design records are fixture measurements and are not product claims.
- The project does not declare a license (see below), so no permission to
  redistribute the ASE source or binary is granted by this document.

## Uninstall

Delete the extraction directory and the directory that contains your index. If you
used `--index DIR\ase.index`, the derived data lives beside it as `ase.index.pub.json`
and a `generations` directory; delete those with the index directory. Never delete a
source session root or a shared runtime such as the VC++ Redistributable when
removing `ase`.

## Documentation

- [Scope and gates](docs/SCOPE.md) — supported behavior, non-goals, layer boundaries.
- [Data contracts](docs/DATA_CONTRACTS.md) — provenance, coordinates, derived data.
- [Static architecture](docs/STATIC_ARCHITECTURE.md) — components and data flow.
- [Use cases and requirements](docs/USE_CASE.md) — user workflows and acceptance criteria.
- [Windows binary assessment](docs/WINDOWS_BINARY_ASSESSMENT.md) — build provenance,
  runtime inventory, verification record and known untested limits.

## License

This repository has **no `LICENSE` or `NOTICE` file**, and the workspace manifests
declare no `license` field. No license terms are granted for the `agents-session-explorer`
source or for the `ase` binary by this README or by publication of the repository.
The project owner must be asked for permission before redistributing either.

Third-party components keep their own licenses. `THIRD_PARTY_NOTICES.txt`, also
inside the release archive, lists the exact crates and C sources compiled into the
binary with their license texts. The separately installed Microsoft VC++
Redistributable is governed by Microsoft's own terms and is not included here.
