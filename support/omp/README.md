# Pinned OMP dispatch-tracking patch (Windows x64)

This is the deliberate support path for the standalone patch needed to build a
compatible OMP runtime for `ase resolve task:`. No native addon, executable,
journal, index or other build output is bundled here.

## What is pinned

- Upstream: `https://github.com/can1357/oh-my-pi`, tag `v19.0.0` = commit
  `57771ae562b9345e0579bceb2d68d585e3e6927e`.
- Patch file: `omp-15c418d.patch`, source commit
  `15c418dd2bf169e9e65e3bfca2dc70ebbbac21f7`
  (`feat(task): add caller-declared tracking and dispatch provenance`). The
  upstream branch/commit is not public; this file is the distribution path.
- The verified candidate from REL-020-2/a1 replayed to commit `d8797ba` on a
  clean `v19.0.0` clone. The pinned copy here replays cleanly to the same tree
  (eight byte-identical files listed below); the replay commit id differs only
  because the author header was redacted.
- Scope: 8 files, TypeScript/docs/test only. No Rust input and no native API
  change, so the public `19.0.0` addon ABI matches the patched runtime.

## Integrity

| Item | Value |
| --- | --- |
| Pinned patch size | 28,979 bytes |
| Pinned patch SHA-256 | `998cadb7377212f49b9f3f5580ec2573ce530949563237e14e29862ac8853f60` |
| Original verified candidate SHA-256 | `147bcd94fc0024ac09735009d87cabf4764a9362c9607ae52688a24860c3fb96` (28,989 bytes) |
| Upstream base commit | `57771ae562b9345e0579bceb2d68d585e3e6927e` (`v19.0.0`) |
| Public addon package | `@oh-my-pi/pi-natives-win32-x64@19.0.0` |
| Addon tarball SHA-1 | `925f74023c34c518597715f4475f777349ffc22b` |
| Addon tarball SHA-512 (hex) | `2d1b2b1a3ebe62901f6d338e572c81637a3f2615834c047d2459d06f01b57eb74f32e1deac6cfab6d5de96213fd95a28f2baafd086e1b83bc16c58921fc1c614` |
| Extracted addon SHA-256 | `383ac3c818829c39f25e0cf4c43ced76fbf1266683f89126312ae5d53584bc2a` |

Recompute the patch digest with `sha256sum omp-15c418d.patch` (PowerShell:
`Get-FileHash -Algorithm SHA256 omp-15c418d.patch`). It is meaningful only
against base `57771ae562b9345e0579bceb2d68d585e3e6927e`.

## Header redaction

The original `git format-patch` header contained a private work email in the
`From:` line. This copy keeps the author name for attribution and substitutes a
neutral `noreply@localhost` address so `git am` has a valid ident; no private
email, local path or machine metadata is redistributed. A line-by-line `diff`
against the verified candidate shows that single header line as the only
change: commit envelope, subject, body, diffstat and every hunk are unchanged.
Replay on the clean base produced these SHA-256 values, identical to the
verified snapshot:

| Replayed file | SHA-256 |
| --- | --- |
| `docs/tools/task.md` | `75ece4a12c4a9b417c01d6833efe1691988ea409b179bc52093dab08bb82f01c` |
| `packages/coding-agent/CHANGELOG.md` | `f9e9ee51d13c20d2858760dec6f0b700b7e5b6a024da11d92474d9bdc7b35eab` |
| `packages/coding-agent/src/task/executor.ts` | `7c4fad134b96ed7ba25671dd5870c9c5c5397a1c4544e51d6a685ce3a5eb48f7` |
| `packages/coding-agent/src/task/index.ts` | `1b7c81e25bf76703a1a0f7e2cd91f1e458ad62841fdd29a1619c07cf5dc4506a` |
| `packages/coding-agent/src/task/structured-subagent.ts` | `c0740ea78121a33581f982e62fd9af228a7f7854c95dd6f7762a0ccc70a7e470` |
| `packages/coding-agent/src/task/types.ts` | `b152776990c66729b20a601a5c2e6c62cf45f7eeac4aa6a8158b19ce929b7d5b` |
| `packages/coding-agent/test/task/structured-subagent.test.ts` | `e95d8e9b179bc7f1884b08cdd857bda0171ca50e4f3d8905447761046acb3e7a` |
| `packages/tui/src/tools/task.ts` | `d4b0d88bbee936576b53aa816c06cc9c6ff5ba0555f817f6af3936fcfa7ef067` |

## Attribution and license notices

- Patch author attribution is retained as `Le Minh Triet` in the patch header.
- Upstream project: oh-my-pi by Stencil Labs, Inc.
  (`https://github.com/can1357/oh-my-pi`). Redistribution or modification must
  respect the upstream repository license and contributing terms.
- The Windows x64 addon comes from the public npm registry; its tarball carries
  `LICENSE` and `THIRD-PARTY-NOTICES.txt`. Those notices stay with the package:
  no addon binary, executable or notice text is copied into this support path.
- No credential, model call, user journal or index is stored here.

## Use

```
git clone --depth 1 --branch v19.0.0 https://github.com/can1357/oh-my-pi.git
cd oh-my-pi
git apply --check ../support/omp/omp-15c418d.patch
git am --keep-cr ../support/omp/omp-15c418d.patch
bun install --frozen-lockfile
npm pack @oh-my-pi/pi-natives-win32-x64@19.0.0
tar -xzf oh-my-pi-pi-natives-win32-x64-19.0.0.tgz
cp package/pi_natives.win32-x64-baseline.node packages/natives/native/
bun run --cwd=packages/coding-agent build
```

On Windows PowerShell use `copy` in place of `cp`. The `../support/...` path
assumes the OMP clone sits next to the ASE checkout; substitute the actual path
otherwise. Apply only to `v19.0.0`; the digest and hunks do not apply to another
base. The addon's npm/SLSA provenance to upstream `57771ae` was verified in
REL-020-2/a1 and is not repeated here. The root README covers the side-by-side
Windows x64 setup and index-rebuild compatibility.

## Limitations

- Only new OMP sessions with caller-declared `tracking` produce dispatch
  records; untracked sessions stay unresolved.
- The patch is not published as an upstream ref, tag or release asset; users
  obtain it from this support path at the release commit.
- No claim about historic sessions, holdout membership, P0d, performance, or
  any non-`v19.0.0` OMP version is made here.
