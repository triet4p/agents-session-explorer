# Lessons Learned

## [2026-11-20] Corpus denominator drifts during live measurement

**Symptom:** Two index runs minutes apart on the same roots produced different event counts (1,821,081 then 1,821,124) and byte totals; `docs/INITIALIZATION.md` still quotes 3,533 files / 6.171 GB while the live OMP root held 3,873 files / 6,896,241,246 bytes.

**Root cause:** The OMP session corpus is append-only and actively growing; file count and byte size sampled before a run are not the denominator of the resulting index.

**Fix / workaround:** Record corpus file count, byte total, event count and index run timestamp from the same snapshot used for each gate measurement; quote the exact event pool in benchmark evidence. Never reuse a size sampled earlier in the session as the gate denominator.

**Watch out for:** Any before/after performance comparison or published gate against this corpus — a changed denominator can look like a regression or improvement that is only new data.


## [2026-11-20] Borrowed &str fields silently drop escaped JSON prose

**Symptom:** an OMP user line containing `\u00e9` produced zero prose projections even though the same line re-serialized without escapes produced two. Search found assistant text but not the user's two blocks on that line.

**Root cause:** serde_json can deserialize into `&'a str` only when it can borrow from the source bytes. An escaped JSON string needs allocation, so deserializing a struct field typed `Option<&str>` fails; the projection function treated that as "no blocks" and returned an empty vector.

**Fix / workaround:** type projected text and tool names as owned `Option<String>` and borrow with `as_deref()` only for hashing/normalization. Re-derivation from raw uses `serde_json::Value` (owned), so it already handled escapes. Watch for empty projection counts on lines with Unicode escapes or `\"`.

**Watch out for:** lines with Unicode escapes, escaped quotes or unusual tool names; verify projection counts with a small fixture before trusting corpus statistics.

## [2026-11-20] FTS5 auxiliary functions reject a table alias

**Symptom:** `SELECT bm25(f) FROM fts_prose f ...` failed with `no such column: f` even though `f MATCH ?` also appeared in the query.

**Root cause:** SQLite FTS5 auxiliary functions take the virtual table name, not a FROM alias.

**Fix / workaround:** do not alias the FTS table in search SQL; use `bm25(fts_prose)`, `fts_prose MATCH ?` and `JOIN ... ON b.rowid = fts_prose.rowid`. Verify FTS behavior with a tiny fixture before benchmarking a full corpus.

**Watch out for:** any search SQL that aliases an FTS5 virtual table; test the exact query on a fixture before benchmarking or publishing results.
