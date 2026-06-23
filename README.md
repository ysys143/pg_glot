# pg_glot_hybrid

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

A family of PostgreSQL extensions that adds **CJK (Korean / Japanese / Chinese) BM25 +
hybrid (RRF) search** on top of pgvector. The morphological / segmentation engine is pure
Rust (lindera + embedded dictionaries), so no external dictionary install is required.

> Status: Layer A (`pg_glot`) and Layer B (`pg_glot_hybrid`) work. BM25/RRF measured on
> MIRACL dev ([`bench/RESULTS.md`](bench/RESULTS.md)). Full design in
> [`docs/DESIGN.md`](docs/DESIGN.md). Korean is the most rigorously validated (POS ablation,
> research parity); ja/zh have measurements but product quality is not yet validated to the
> ko level.

## Layout (monorepo, Cargo workspace)

| Component | Role | Depends on |
|---|---|---|
| `crates/glot-tokenizer` | Pure-Rust CJK tokenizer (lindera + embedded ko-dic/IPADIC/CC-CEDICT) | — |
| `extensions/pg_glot` | (Layer A) custom TS parser -> `korean`/`japanese`/`chinese` text search config; owns the `glot` schema (`glot.rrf`) | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF hybrid (`glot.hybrid`) | pg_glot + pg_textsearch + pgvector |

## Install — separable layers

It is one monorepo, but **each layer is an independent extension/crate with clean boundaries, so
you install only what you need.** `pg_textsearch` and `pgvector` are dependencies (`requires`),
not vendored — you upgrade them on their own schedule.

| You want… | Install | Pulls in |
|---|---|---|
| Full hybrid (BM25 + dense RRF) | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector (auto, via `requires`) |
| CJK full-text search only (`to_tsvector` / `@@` / `ts_rank`) | `CREATE EXTENSION pg_glot;` | nothing else — zero extra deps |
| The RRF fusion primitive (`glot.rrf`) | `CREATE EXTENSION pg_glot;` | — (the `glot` schema ships with Layer A) |
| The tokenizer outside PostgreSQL | depend on the `glot-tokenizer` crate | — |

`pg_textsearch` needs `shared_preload_libraries = 'pg_textsearch'` (the prebuilt Docker image
sets this up). Monorepo today; the clean boundaries make splitting into separate repos
mechanical later.

## Usage

### Layer A — CJK full-text search (`pg_glot` alone)

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

### Layer B — BM25 + hybrid RRF (`pg_glot_hybrid`)

```sql
CREATE EXTENSION pg_glot_hybrid CASCADE;   -- pulls pg_glot + pg_textsearch + pgvector

CREATE TABLE docs (id bigint PRIMARY KEY, body text, emb vector(1024));

-- BM25 index over the CJK config (schema-qualify the config name)
CREATE INDEX ON docs USING bm25(body) WITH (text_config = 'public.korean');
-- dense index
CREATE INDEX ON docs USING hnsw (emb vector_cosine_ops);

-- BM25-only ranking. NOTE: the query must be a literal (planner hook) and use
-- a plain ORDER BY ... LIMIT (index scan).
SELECT id FROM docs ORDER BY body <@> '형태소 분석' LIMIT 10;

-- one call = BM25(body) + dense(emb) fused by RRF
SELECT id, score
FROM   glot.hybrid(
           'docs',                -- rel (regclass)
           'id', 'body', 'emb',   -- key / text / vector columns
           '형태소 분석',          -- query text  (BM25 leg)
           '[ ... ]'::vector,     -- query vector (dense leg)
           k       => 60,         -- RRF k        (default 60)
           per_leg => 60,         -- top-k per leg (default 60)
           n       => 10);        -- final rows   (default 10)

-- or fuse your own pre-computed id lists with the RRF primitive (ships with Layer A)
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

**Selecting the table:** the first argument (`'docs'`, a `regclass`) is the table to search;
the next three are its key / text / vector column names. That table must already have a BM25
index on the text column (matching `text_config`) and a vector index on the vector column, and
the key column must be `bigint`. Schema-qualify if needed: `'myschema.docs'`.

Swap `'public.korean'` (and `'korean'`) for `japanese` or `chinese` to switch language.

## Search quality (MIRACL dev, measured)

Measured against a real pg_glot + pg_textsearch BM25 index via `bench/`. Details, caveats, and
reproduction in [`bench/RESULTS.md`](bench/RESULTS.md). **This is a dev-passages subset, so it
is not directly comparable to the official leaderboard (indicative only).**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

ko BM25 reaches 99.7% of research MeCab (0.6385). RRF (dense BGE-M3 + BM25) significantly
improves over BM25 by +0.12–0.17 in all three languages (p<0.001).

**Without lindera, what does stock PG give?** (no morphological analysis, recall@10)

| lang | PG native (simple) | pg_trgm | **lindera** |
|---|---|---|---|
| ko | 0.479 | 0.327 | **0.798** |
| ja | 0.179 | 0.516 | **0.773** |
| zh | 0.017 | 0.364 | **0.646** |

For space-less ja/zh the native `simple` config nearly collapses (zh R 0.017), and pg_trgm only
captures substrings. Morphological segmentation (lindera) is essential for CJK search.

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
path (lindera = MIT, ko-dic = Apache-2.0, IPADIC/CC-CEDICT under their respective dictionary
licenses). A Kiwi (LGPL) backend is designed (see [`docs/DESIGN.md`](docs/DESIGN.md) D5) but not
yet implemented; if added it would be opt-in and dynamically linked.
