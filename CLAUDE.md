# pg_textsearch_ko — coding guidelines

pg_textsearch_ko is a **Rust / pgrx PostgreSQL extension family** for Korean search
(lindera-backed `korean` text search config → BM25 → RRF hybrid). Monorepo (Cargo
workspace); local dev/target PostgreSQL is **17** (pgrx-managed), other majors via Cargo
features. Full design, decisions, and license audit live in **`docs/DESIGN.md`** (read it first).

Build & test:
- `make unit` — pure-Rust tokenizer tests (`korean-tokenizer`), no PG, fastest loop.
- `cd extensions/<ext> && cargo pgrx test pg17` — `#[pg_test]` in a real PG17 (the bulk).
- `cargo pgrx run pg17` — interactive psql against the dev instance (manual checks).
- `make fmt` / `make lint` — `cargo fmt` / `clippy -D warnings`.

These conventions apply to all code-writing agents.

---

## Kent Beck — TDD & Tidy First

### 1. Red → Green → Refactor
- For new behavior, write the failing test **first**: a Rust unit test in `korean-tokenizer`
  (tokenizer logic) or a `#[pg_test]` (SQL-visible behavior, e.g. `to_tsvector('korean', …)`,
  `ts_debug`, `@@`). Confirm it is RED.
- Implement the minimum to make `cargo pgrx test pg17` (and/or `make unit`) GREEN.
- Refactor only once green, then re-run the tests.

### 2. Tidy First — never mix structural and behavioral changes
- Structural (rename / extract / move): behavior unchanged → tests/goldens unchanged.
- Behavioral (new feature / fix): structure unchanged.
- If both are needed: structural commit first, behavioral commit second — separate commits.

### 3. Make it work → make it right → make it fast (prove "fast"/"better")
- Simplest thing that passes first; then remove duplication and reveal intent.
- **No quality/performance claim without a reproducible measurement.** The `bakeoff/` numbers
  are a *relative, controlled* tokenizer comparison (pure-Python BM25, ~1K docs, no significance
  test) — never present them as absolute/product quality, and never equate them to the research
  repo's headline NDCG. Before any public NDCG claim, reproduce research Phase 7 quality with
  **real pg_textsearch** at scale (the release gate in `docs/DESIGN.md` §5.5 / D3).

### 4. Commit discipline
- Commit only when `cargo pgrx test pg17` and `make unit` are green with no new warnings.
- One commit = one logical unit; state whether it is a structural or behavioral change.
- Branch off `main` (don't commit straight to main). Use the git skills
  `session-and-git:branch-name-convention`, `session-and-git:commit-message-convention`,
  `session-and-git:pr-convention`. End commit messages with the Co-Authored-By trailer
  (`Claude Opus 4.8 (1M context)`).

### 5. Tests must be deterministic and self-validating
- `#[pg_test]` runs each test in its own transaction (rolled back) → independent & repeatable;
  assert SQL-visible truth (lexeme counts, `ts_debug` aliases, `@@` matches), not internals.
- Tokenizer unit tests are deterministic; assert the byte-offset invariant
  (`text[byte_start..byte_end] == surface`) — it is what the TS parser depends on.
- The lindera dictionary is part of the index definition: if it changes, old tsvectors/indexes
  are stale (REINDEX). Treat dictionary version as a reproducibility concern.

### 6. Test layers (a pgrx extension is NOT unit-test-heavy)
- **Rust unit** (`korean-tokenizer`) — tokenization, offsets, edge cases. Fast, isolated.
- **`#[pg_test]` / pg_regress** — the bulk: `to_tsvector`/`ts_debug`/`@@`, BM25 round-trip,
  index↔query consistency, against a real PG.
- **isolation** — concurrent insert/scan, cache safety (when the index layer lands).
- **bake-off / bench** (`bakeoff/`) — relative quality & latency; **INDICATIVE**, not a
  correctness gate.

---

## Custom TS-parser (C-ABI) safety — the high-risk seam

The `korean` parser crosses the C ABI (`internal`/`int4` functions PG calls directly). Keep it safe:
- Parser state via `Internal::new(...)` (palloc in the current memory context → auto-dropped on
  context delete; no leak on error). Never return Rust-owned pointers PG would free.
- Copy/return token bytes that live in the palloc-managed state; PG copies them during `gettoken`.
- Validate input is UTF-8 at the boundary → `ereport` on failure (DB encoding must be UTF8).
- Let `#[pg_guard]` (pgrx-applied) convert panics to `ereport`; never unwind across the ABI.
- A `SECURITY DEFINER` function (e.g. future `ko_search_hybrid`) must pin
  `SET search_path = pg_catalog, …, pg_temp`.

---

> Working if: a unit/`pg_test` precedes the code; structural and behavioral changes never share a
> commit; "make it work" precedes "make it right"; and no quality/performance claim ships without
> a reproducible measurement.
