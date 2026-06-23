# pg_glot

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

**pg_glot** brings **CJK (Korean / Japanese / Chinese) full-text search and BM25 + dense
hybrid (RRF) retrieval** to PostgreSQL, on top of pgvector. The morphological / segmentation
engine is pure Rust (lindera + embedded dictionaries) — there is no external dictionary to
install.

It ships as two extensions you can adopt independently:

- **`pg_glot`** — the CJK tokenizer and the `korean` / `japanese` / `chinese` text-search
  configs (ordinary PostgreSQL full-text search), plus the `glot.rrf` fusion primitive.
- **`pg_glot_hybrid`** — BM25 indexing and BM25 + dense hybrid search (`glot.rank`,
  `glot.hybrid`), built on `pg_glot` + `pg_textsearch` + `pgvector`.

> **Status** — both extensions work; BM25/RRF measured on MIRACL dev
> ([`bench/RESULTS.md`](bench/RESULTS.md)). Korean is the most rigorously validated (POS
> ablation, research parity); Japanese and Chinese have measurements but are not yet validated
> to the Korean level. Full design and decisions in [`docs/DESIGN.md`](docs/DESIGN.md).

## Components

A single monorepo (Cargo workspace) with clean, independently-installable boundaries:

| Component | Role | Depends on |
|---|---|---|
| `crates/glot-tokenizer` | Pure-Rust CJK tokenizer (lindera + embedded ko-dic / IPADIC / CC-CEDICT) | — |
| `extensions/pg_glot` | Custom TS parser → `korean` / `japanese` / `chinese` text-search configs; owns the `glot` schema (`glot.rrf`) | glot-tokenizer |
| `extensions/pg_glot_hybrid` | CJK BM25 + RRF hybrid — `glot.rank` custom scan (`ORDER BY … LIMIT`) and `glot.hybrid` SRF | pg_glot + pg_textsearch + pgvector |

## Install

**You install only what you need** — `pg_textsearch` and `pgvector` are dependencies
(`requires`), not vendored, so you upgrade them on their own schedule.

| You want… | Install | Pulls in |
|---|---|---|
| Full hybrid (BM25 + dense RRF) | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector (auto, via `requires`) |
| CJK full-text search only (`to_tsvector` / `@@` / `ts_rank`) | `CREATE EXTENSION pg_glot;` | nothing else — zero extra deps |
| The RRF fusion primitive (`glot.rrf`) | `CREATE EXTENSION pg_glot;` | — (the `glot` schema ships with `pg_glot`) |
| The tokenizer outside PostgreSQL | depend on the `glot-tokenizer` crate | — |

The hybrid path needs `shared_preload_libraries = 'pg_textsearch, pg_glot_hybrid'` (the prebuilt
Docker image sets this up): `pg_textsearch` provides BM25, and `pg_glot_hybrid` registers the
`glot.rank` custom-scan hook. Splitting the monorepo into separate repos later is mechanical.

## Usage

### `pg_glot` — CJK full-text search

```sql
CREATE EXTENSION pg_glot;

-- tokenize via the language config (korean / japanese / chinese)
SELECT to_tsvector('korean',   '한국어 형태소 분석');
SELECT to_tsvector('japanese', '東京都に住む');

-- match / rank like any PostgreSQL full-text search
SELECT id
FROM   docs
WHERE  to_tsvector('korean', body) @@ to_tsquery('korean', '형태소')
ORDER  BY ts_rank(to_tsvector('korean', body), to_tsquery('korean', '형태소')) DESC;
```

### `pg_glot_hybrid` — BM25 + hybrid RRF

```sql
CREATE EXTENSION pg_glot_hybrid CASCADE;   -- pulls in pg_glot + pg_textsearch + pgvector

CREATE TABLE docs (id bigint PRIMARY KEY, body text, emb vector(1024));

-- BM25 index over the CJK config (schema-qualify the config name)
CREATE INDEX ON docs USING bm25(body) WITH (text_config = 'public.korean');
-- dense index
CREATE INDEX ON docs USING hnsw (emb vector_cosine_ops);

-- BM25-only ranking. NOTE: the query must be a literal (planner hook) and use
-- a plain ORDER BY ... LIMIT (index scan).
SELECT id FROM docs ORDER BY body <@> '형태소 분석' LIMIT 10;

-- Flagship: hybrid (BM25 + dense, fused by RRF) reads like a normal KNN query.
-- `body`/`emb` are real columns; the two queries are literals. A plain ORDER BY ... LIMIT
-- with literal queries lets the planner pick the GlotHybrid custom scan (both index legs + RRF).
SELECT id, body
FROM   docs
ORDER  BY glot.rank(body, emb, '형태소 분석', '[ ... ]'::vector) DESC
LIMIT  10;

-- Explicit set-returning form (composable; same RRF result, no hook needed)
SELECT id, score
FROM   glot.hybrid('docs', 'id', 'body', 'emb',
                   '형태소 분석', '[ ... ]'::vector, 60, 60, 10);

-- or fuse your own pre-computed id lists with the RRF primitive (ships with pg_glot)
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

**`glot.rank` (flagship)** needs `shared_preload_libraries = 'pg_glot_hybrid'` so the
`GlotHybrid` custom-scan hook is installed (the prebuilt Docker image sets this up). Without the
hook the planner cannot rewrite the query and `glot.rank` falls back to a non-RRF score.
`body`/`emb` are real column references; the two queries must be literals; the table needs a
BM25 index (and, ideally, an HNSW index — otherwise the dense leg is an exact scan).

**`glot.hybrid` (explicit form)** takes the table as its first argument (`'docs'`, a
`regclass`), then its key / text / vector columns. The table must have the BM25 index (matching
`text_config`) and a vector index, and the key column must be `bigint`. It works without the
preload hook. Schema-qualify if needed: `'myschema.docs'`.

Swap `'public.korean'` (and `'korean'`) for `japanese` or `chinese` to switch language.

## Search quality (MIRACL dev, measured)

Measured against a real `pg_glot` + `pg_textsearch` BM25 index via [`bench/`](bench). Details,
caveats, and reproduction in [`bench/RESULTS.md`](bench/RESULTS.md). **This is a dev-passages
subset, so it is not directly comparable to the official leaderboard (indicative only).**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

Korean BM25 reaches 99.7% of research MeCab (0.6385). RRF (dense BGE-M3 + BM25) improves
significantly over BM25 by +0.12–0.17 in all three languages (p < 0.001).

**Without lindera, what does stock PostgreSQL give?** (no morphological analysis, recall@10)

| lang | PG native (`simple`) | pg_trgm | **lindera** |
|---|---|---|---|
| ko | 0.479 | 0.327 | **0.798** |
| ja | 0.179 | 0.516 | **0.773** |
| zh | 0.017 | 0.364 | **0.646** |

For space-less Japanese and Chinese the native `simple` config nearly collapses (zh R 0.017),
and pg_trgm only captures substrings. Morphological segmentation (lindera) is essential for CJK
search.

## Development

```bash
make unit          # pure-Rust tokenizer unit tests (no PG needed)
make run           # cargo pgrx run pg17 -> psql
make test          # pg_regress + pg_test (pg17)
```

Target PostgreSQL: **17** (pgrx-managed). Foundation = pgrx (Rust). To build fewer languages,
enable features e.g. `--no-default-features --features "pg17 korean"` (default is all three CJK).

## License

PostgreSQL License. Third-party notices in [`NOTICE`](NOTICE). No GPL code on the default build
path (lindera = MIT, ko-dic = Apache-2.0, IPADIC / CC-CEDICT under their respective dictionary
licenses). A Kiwi (LGPL) backend is designed (see [`docs/DESIGN.md`](docs/DESIGN.md) D5) but not
yet implemented; if added it would be opt-in and dynamically linked.
