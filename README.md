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
| `extensions/pg_glot` | (Layer A) custom TS parser -> `korean`/`japanese`/`chinese` text search config | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF hybrid (`glot.hybrid`) | pg_glot + pg_textsearch + pgvector |

Install: `CREATE EXTENSION pg_glot_hybrid CASCADE;` builds the whole dependency stack in one line.

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
licenses; Kiwi (LGPL) is an opt-in feature).
