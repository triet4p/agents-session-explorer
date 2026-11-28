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

Latest release: [v0.8.0](https://github.com/triet4p/agents-session-explorer/releases/tag/v0.8.0).

| Asset | Contents |
|---|---|
| `ase-0.8.0-windows-x64.zip` | `ase.exe`, `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.txt` |
| `SHA256SUMS.txt` | SHA-256 of the archive and its members |

The `v0.7.0` archive remains available; it predates the project `LICENSE` and the
`--text` modes.

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
$zip = Join-Path $HOME 'Downloads\ase-0.8.0-windows-x64.zip'
$sums = Join-Path $HOME 'Downloads\SHA256SUMS.txt'   # download both assets first

$expected = ((Get-Content -LiteralPath $sums | Select-String 'ase-0.8.0-windows-x64.zip').Line -split '\s+')[0]
$actual   = ((certutil -hashfile $zip SHA256)[1] -replace ' ', '').ToLowerInvariant()
if ($actual -ne $expected.ToLowerInvariant()) {
    throw "STOP: archive SHA-256 mismatch. Do not extract or run."
}

$dest = Join-Path $HOME 'ase-0.8.0'
New-Item -ItemType Directory -Path $dest | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zipFile = [System.IO.Compression.ZipFile]::OpenRead($zip)
try {
    foreach ($entry in $zipFile.Entries) {
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile(
            $entry, (Join-Path $dest $entry.FullName), $false)
    }
}
finally { $zipFile.Dispose() }

$ase  = Join-Path $dest 'ase.exe'
& $ase --help
if ($LASTEXITCODE -ne 0) { throw 'ase --help failed' }
```

There is no installer: extraction creates or changes nothing outside the destination
directory, and `ase` does not modify `PATH`, the registry or a service. The snippet
uses the .NET ZIP API because `Expand-Archive` fails on some Windows PowerShell 5.1
builds; it performs the same fresh-directory extraction with no overwrite. To remove
it, delete that directory and any index directory you created; see
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

### Daily fixture lookup

The JSON quick start above is unchanged and works in `v0.8.0`. The `--text` modes
described here and in the next two sections are new in `v0.8.0`; JSON output and
the `open_args.flags` contract stay the same for automation. Work from a fixture
in a workspace-local directory whose path contains a space, and always give the
CLI an explicit index path — never let roots default.

```powershell
$work    = Join-Path $HOME 'ase daily lookup'   # space on purpose
$fixture = Join-Path $work 'fixture'
$index   = Join-Path $work 'ase.index'
$empty   = Join-Path $work 'empty-source'
New-Item -ItemType Directory -Force -Path $work, $empty | Out-Null

# 1. Synthetic journals. Keep BOTH roots explicit; if you only want one source
#    type, point the other flag at a deliberately empty directory instead of
#    dropping it and falling back to the real profile.
& $ase fixture --out $fixture

# 2. Check scope and count before searching: the base fixture is 3 files,
#    OMP 1 (synthetic.jsonl) and Codex 2 (synthetic.jsonl, synthetic.jsonl.zst).
$ompCount   = @(Get-ChildItem -LiteralPath (Join-Path $fixture 'omp') -Recurse -File).Count
$codexCount = @(Get-ChildItem -LiteralPath (Join-Path $fixture 'codex') -Recurse -File).Count
if ($ompCount + $codexCount -ne 3) { throw "unexpected fixture count: $ompCount + $codexCount" }

# 3. Index both explicit roots into the explicit (space-containing) index path.
& $ase index --layer all `
    --omp-root   (Join-Path $fixture 'omp') `
    --codex-root (Join-Path $fixture 'codex') `
    --index $index --json

# 4. Readable hit list; JSON stays available for automation.
& $ase search 'SYNTHETIC USER TOKEN' --index $index --text
$result = & $ase search 'SYNTHETIC USER TOKEN' --index $index --json --open-args | ConvertFrom-Json
$hit    = $result.hits[0]

# 5. Copy keys from that hit and reopen on the SAME index. A literal selector
#    containing '|' must be single-quoted (PowerShell reads it as a pipeline),
#    e.g. --selector 'block-v1|message.content[0]|text'
$selector = [string]$hit.selector
& $ase open --event $hit.event --selector $selector `
    --generation $result.generation --index $index --text
```

`ase index` defaults to the `address` layer, which has no postings, so search needs
`--layer all` (or `--layer postings`). `status = "ready"` alone does not prove the
index has data: a missing or empty root also reports `ready` with zero files, so
check scope and count first. Pass both roots every time to avoid the fallback to
`ASE_OMP_ROOT` / `ASE_CODEX_ROOT` and the real profile directories.

Copy the keys, do not rebuild them: take `generation` from the search header and
`event`/`selector` from the same hit line, and reuse the exact `--index` path. Never
feed a hit string to `Invoke-Expression` and never choose a hit by display number or
line rank — the leading number is display order only.

Read the status before calling a block evidence; `exit 0` is not a verification
verdict:

- No hits prints `0` and exits `0` — a valid empty result, not a failed search.
- `layer_unavailable` exits `4`: check `--index`; if postings were not published,
  rebuild with `--layer all` and both roots.
- `generation_mismatch` exits `4`: the hit belongs to another generation. Search
  again on the intended `--index`; do not edit the generation copied from the old hit.
- Unreadable raw, a missing block mapping or an unusable publication exits `2`: it is
  never verified, and text mode does not repeat the raw path.
- `unverified` exits `0` when raw is readable but a hash check fails: inspect the
  source, and re-index then search again only when a new snapshot is intended. Only
  `verified_against_index_snapshot` (both `raw_span_hash_match` and
  `block_digest_match` true) is verified evidence.

To read local text, reopen the same hit without `--text`, confirm the status on that
same open, then add `--content` explicitly. Default JSON output and the
`open_args.flags` splat above are unchanged. Transcripts, shell queries, metadata and
the derived index stay local data — do not copy them into an evidence dossier.

### Piping output

A consumer that stops reading, such as `ase search 'TERM' --index $index --text |
Select-Object -First 5` (or `| head` on other shells), closes the pipe. `ase` then ends
output quietly with the command's normal exit code: no panic text, backtrace or pipe
error on stderr, and a `generation_mismatch` still exits `4`. Other write failures
remain errors: they are reported on stderr and the command exits `1`, so a command is
never turned into success because stdout closed. When
the consumer reads everything, stdout bytes, stderr and exit codes are unchanged, so
filtering with a pipe never changes the status of the command itself.

### Readable search hits

`ase search <QUERY> --index FILE --text` is new in `v0.8.0`. It prints the
returned hit count, the full pinned generation, the explicit `--index FILE`, and
one metadata line per hit with display order, role, span, freshness, event and
selector. It never prints the query, snippets, prose, journal paths or a
ready-to-run shell command. The leading number is display order only: reopen with
the event and selector from that hit, the generation from the header, and the same
`--index FILE`.

```powershell
& $ase search 'SYNTHETIC USER TOKEN' --index $index --text
```

`--phrase` and `--top-k` still apply. `--text` cannot be combined with
`--json`, `--open-args` or `--explain` (exit `2`); default and `--json` output
stay unchanged for automation. A successful search with no hits prints a zero
count with exit `0`; a missing or mismatched postings layer prints
`layer_unavailable` with exit `4` and is not a zero-hit result.

### Readable reopen status

`ase open --event N --selector S --generation G --index FILE --text` is new in
`v0.8.0`. It prints the evidence status, the exact block key, the span, the
encoding and coordinate system, both verification checks and the source freshness
hint. It never prints prose, call signatures or journal paths.

```powershell
& $ase open --event $hit.event --selector $hit.selector `
    --generation $hit.open_args.generation --index $index --text
```

`status=verified_against_index_snapshot` appears only when both
`raw_span_hash_match` and `block_digest_match` are true. For zstd sources the
span stays in `decompressed_stream_bytes`, and freshness is only a metadata hint,
not a byte check. `--text` cannot be combined with `--json`, `--content` or
`--session-args` (exit `2`).

Read the status before treating a block as evidence:

- A hit from another generation stops with `status=generation_mismatch` (exit `4`)
  before any raw read. Search again on the intended `--index`; do not edit the
  generation copied from the old hit.
- Readable raw with one failed check stays `status=unverified` (exit `0`). Inspect
  the source, and only re-index then search again when a new snapshot is intended.
- A missing block mapping, an unusable publication or an unreadable raw source
  remains an error (exit `2`) and is never reported as verified.

To read local text, reopen the same hit without `--text`, confirm the status, then
add `--content` explicitly. Default JSON output and the `open_args.flags` splat
above are unchanged.

## Build from source

Requirements:

- Rust 1.91.1 or newer with the `x86_64-pc-windows-msvc` target, plus the MSVC
  linker and Windows SDK from Visual Studio Build Tools (Windows), or the platform
  C toolchain for other targets.
- Git, to fetch the tagged source.

```bash
git clone --branch v0.8.0 https://github.com/triet4p/agents-session-explorer.git
cd agents-session-explorer
cargo build --release --locked --target x86_64-pc-windows-msvc
# Windows output: target\x86_64-pc-windows-msvc\release\ase.exe
```

On other platforms, drop `--target` and the MSVC requirement; only Windows x64 has
release evidence. `zstd` and SQLite C sources are compiled into the binary, so the
C toolchain is required at build time but no extra DLLs are needed at runtime
beyond the Windows system components and `VCRUNTIME140.dll`.

The released binary was built from the `v0.8.0` tag with path remapping
(`--remap-path-prefix` for Rust; `/pathmap` for the MSVC C compiler) so the image
does not embed the build host's source paths. The build is repeatable in procedure
but not byte-for-byte reproducible: PE timestamps and PDB identity differ between
runs, so rebuilding will not reproduce the published digest.

## Command map

`ase --help` prints the full grammar. The main areas are:

| Area | Commands and purpose |
|---|---|
| Index | `ase index` builds the address layer and optional postings layer; one generation is published atomically behind `<index>.pub.json`. |
| Search and evidence | `ase search` (JSON by default, `--text` for a metadata-only hit list), `ase open` (JSON by default, `--text` for a metadata-only reopen status), `ase show`, `ase shingle`, `ase verify` return metadata and reopen exact spans; `--content` is the only way to print source text. |
| Navigation | `ase search --open-args` and `ase open --session-args` emit exact `open`/`show`/`resolve`/`work` flags for a verified block. Locators keep the logical `--index FILE` and pin the generation; a republished path makes old flags stop with `generation_mismatch` (exit `4`) rather than read a newer snapshot. Ambiguous, path-derived and unverified ids are refused. |
| Identity | `ase resolve`, `ase audit naming`, `ase audit delegation`, `ase audit markers` reconcile naming grammar, delegation and `task_id=` markers with inspectable evidence. |
| Work and analytics | `ase work project\|session\|reopen\|compare\|delta` projects turns/episodes; `ase analytics` reports stored cost, duration and clock unions. |
| Patterns | `ase motif`, `ase shingle`, and `ase adapter omp-stock` produce advisory, evidence-pinned claims; adapter claims never write membership. |
| Evidence exchange | `ase export motif:<id>`, `ase verify <bundle>`, `ase import verdicts`, `ase join skill-gap` move path-free evidence bundles without shipping source paths. |
| Optional model layer | `ase judge`, `ase llm` run offline evaluation against a local runner or an explicitly configured API profile. |
| Fixtures | `ase fixture`, `ase fixture-*` write synthetic journals for every layer; they contain no real transcript text. |

Run any subcommand with `--help` for its exact flags, for example
`ase work --help` or `ase judge --help`.

`--help` and `-h` are read-only at every level: the root command, each top-level
command, and every group and leaf (including `audit`, `adapter`, `work`,
`analytics`, `import verdicts`, `join skill-gap`, `judge`, `llm` and the
`fixture-*` / `bench-*` commands) print that command's usage to stdout with
empty stderr and exit `0`. They need no positional argument and open no index,
config, credential store or network connection, and never prompt or write an
output file. Help is recognized only where a flag is expected: a value that
looks like help (for example `--event --help`) is still parsed as a value and
keeps its normal error, and an unknown command or leaf still exits `2`.

## Identity queries and motif cohorts

`ase resolve task:TOKEN` and `ase motif TOKEN` answer different questions even
when both print the same token. `resolve` reconciles a bare token through the
name grammar, `task_id=` markers, delegation declarations, literal path
matches and native OMP dispatch records; one project-scoped key can make it
`assigned` without any dispatch join. `motif TOKEN` sends the same bare token
only through the dispatch-backed membership validator, so a naming-only
assignment yields no admitted attempt. `motif 'PROJECT|TOKEN'` applies the
project scope before that validator and admits only attempts of the exact
`(project, task)` key, without falling back to another project.

When two projects share a token, a bare query stays ambiguous and their keys
are never merged. Quote the scoped address in PowerShell so the pipe stays in
one argument:

```powershell
ase resolve task:4 --index .\ase.index --json
ase motif 4 --index .\ase.index --json
ase motif 'Four4Alpha|4' 'Four4Beta|4' --index .\ase.index --json
```

The example uses the synthetic fixtures (`ase fixture-resolve`,
`ase fixture-motif`, `ase fixture-dispatch`), not a user's corpus. Each task
key is one support unit: three attempts on two keys give `support.exact = 2`,
not 3. An assigned resolver token is not a promise of a motif sample, and a
marker, path, suffix, prompt or adapter claim cannot create membership. The
identity contract is in [DATA_CONTRACTS.md](docs/DATA_CONTRACTS.md#task).

## How data is stored

- JSONL spans are addressed as `(file, off, len)` in raw file bytes. A
  `.jsonl.zst` file is addressed by its **decompressed** stream, never by compressed
  offsets.
- `ase index --layer address` writes address data only. `--layer postings` adds the
  derived search layer; `--layer all` derives both in one decode pass.
- Each successful publication writes an immutable generation directory and replaces
  one sidecar pointer atomically. A mismatched or missing sidecar returns
  `layer_unavailable` for search while address-based commands keep working.
- `--index FILE` is one logical path for every command that reads the index,
  including `audit`, `work`, `analytics`, `verify` and `bench-*`. When a usable
  publication exists, readers open its pinned generation address even if `FILE`
  itself has no address bytes; without one, address-based commands use `FILE`
  only when it is itself a usable address, and derived-layer commands refuse
  rather than report an empty result. Pass a direct `generations/<G>/address.bin`
  path for an address-only read.
- `search` returns coordinates, ranking metadata and generation — no stored prose or
  snippet. `open` re-reads the raw span, re-derives the block and reports
  `verified_against_index_snapshot` only when the span checksum and block digest
  both match.

- `open --session-args` locators keep the logical `--index FILE` and add an
  explicit `--generation` to their `show`, `resolve` and `work session` flags.
  Reusing a locator after a republish stops with `generation_mismatch` (exit `4`)
  before raw is read; search again on the intended `--index` for a new locator
  instead of editing the old pin. Saved flag arrays that point straight at an
  address still open only that address while it exists.

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
- The `--text` modes were exercised on synthetic fixtures and the build host only.
- The build embeds no build-host source paths after remapping, but it is not
  byte-reproducible: PE timestamps and PDB identity change per build.
- No real-user pilot or real corpus was run for this release. Counts quoted in
  older design records are fixture measurements and are not product claims.

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

This project is licensed under the [MIT License](LICENSE), with copyright held by
Le Minh Triet (2026). The `v0.8.0` source tree, Windows x64 archive and binary carry
that `LICENSE`. The earlier `v0.7.0` tag and archive remain as released, without a
project license file.

Third-party components keep their own licenses. `THIRD_PARTY_NOTICES.txt`, also
inside the release archive, lists the exact crates and C sources compiled into the
binary with their license texts. The separately installed Microsoft VC++
Redistributable is governed by Microsoft's own terms and is not included here.
