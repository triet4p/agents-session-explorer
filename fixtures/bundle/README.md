# Synthetic evidence-bundle fixture

This tree is a runnable, path-free evidence bundle plus the minimum private
inputs needed to reopen it. Every byte is synthetic: invented projects, tool
names, session ids and dispatch ids; no real journal, account, path or model
output is included.

## Files

- `bundle.json`, `motifs/` — one `ase-bundle-v1` motif bundle with two
  occurrences and eight reopenable evidence rows.
- `local-map.json` — the private `ase-local-sources-v1` alias map. Relative
  paths resolve next to this file.
- `sources/` — the three synthetic OMP journals named by the map (parent
  dispatch records and two child sessions).
- `verdicts.json` — three synthetic `ase-verdicts-v1` judgments whose
  assertion ids are valid for this bundle.
- `catalog.json` — a synthetic `ase-skill-catalog-v1` used by the join example.

## Commands

```bash
ase verify fixtures/bundle --local-map fixtures/bundle/local-map.json \
  --verdicts fixtures/bundle/verdicts.json --json
ase import verdicts fixtures/bundle/verdicts.json --bundle fixtures/bundle \
  --local-map fixtures/bundle/local-map.json --store .tmp/bundle-store.json --json
ase join skill-gap --store .tmp/bundle-store.json \
  --catalog fixtures/bundle/catalog.json --bundle fixtures/bundle \
  --local-map fixtures/bundle/local-map.json --json
```

All three commands are local: no network, model, catalog service or gate
measurement is involved. This tree uses the current `ase-bundle-v1` schema. The
earlier `fixtures/p4/` tree is retained only as the historical source publish;
its phase-named schema is rejected by the current commands.

## Regenerating

The bundle was produced from a two-task synthetic dispatch corpus with the
public CLI. Author two child sessions with identical calls under distinct
dispatch-confirmed project/task keys, then:

```bash
ase index --layer all --omp-root .tmp/corpus/omp --codex-root .tmp/corpus/codex \
  --index .tmp/corpus/ase.index
ase motif 'Alpha|1' 'Beta|2' --index .tmp/corpus/ase.index --json
ase export 'motif:<id>' --out fixtures/bundle --local-map fixtures/bundle/local-map.json \
  --address 'Alpha|1' --address 'Beta|2' --index .tmp/corpus/ase.index \
  --omp-root .tmp/corpus/omp --codex-root .tmp/corpus/codex --json
```
