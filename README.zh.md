# pg_glot_hybrid

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

在 pgvector 之上叠加 **CJK（韩语・日语・汉语）BM25 + 混合（RRF）检索**的 PostgreSQL 扩展家族。
形态素／分词引擎为纯 Rust（lindera + 内嵌词典），无需安装外部词典。

> 状态: Layer A（`pg_glot`）与 Layer B（`pg_glot_hybrid`）已可用。在 MIRACL dev 上实测
> BM25/RRF（[`bench/RESULTS.md`](bench/RESULTS.md)）。完整设计见
> [`docs/DESIGN.md`](docs/DESIGN.md)。ko 验证最严格（POS ablation、与 research 持平）；ja/zh
> 已有测量值，但产品质量尚未达到 ko 的验证水平。

## 结构（monorepo，Cargo workspace）

| 组件 | 作用 | 依赖 |
|---|---|---|
| `crates/glot-tokenizer` | 纯 Rust CJK 分词器（lindera + 内嵌 ko-dic/IPADIC/CC-CEDICT） | — |
| `extensions/pg_glot` | (Layer A) 自定义 TS parser → `korean`/`japanese`/`chinese` text search config | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF 混合（`glot.hybrid`） | pg_glot + pg_textsearch + pgvector |

安装: `CREATE EXTENSION pg_glot_hybrid CASCADE;` 一行即可自动构建整个依赖栈。

## 检索质量（MIRACL dev，实测）

通过 `bench/` 在真实 pg_glot + pg_textsearch BM25 索引上测量。详情、限制与复现见
[`bench/RESULTS.md`](bench/RESULTS.md)。**这是 dev passages 子集，无法与官方排行榜直接比较
（仅供参考）。**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

ko 的 BM25 达到 research MeCab（0.6385）的 99.7%。RRF（dense BGE-M3 + BM25）在三种语言上都
显著优于 BM25，提升 +0.12~0.17（p<0.001）。

**没有 lindera，仅用原生 PG 会如何?**（无形态素分析，recall@10）

| lang | PG native（simple） | pg_trgm | **lindera** |
|---|---|---|---|
| ko | 0.479 | 0.327 | **0.798** |
| ja | 0.179 | 0.516 | **0.773** |
| zh | 0.017 | 0.364 | **0.646** |

对于无空格的 ja/zh，原生 `simple` 几乎失效（zh R 0.017），pg_trgm 也只能捕捉子串。
形态素／分词（lindera）是 CJK 检索的关键。

## 开发

```bash
make unit          # 纯 Rust 分词器单元测试（无需 PG）
make run           # cargo pgrx run pg17 → psql
make test          # pg_regress + pg_test (pg17)
```

目标 PostgreSQL: **17**（pgrx 管理）。基础 = pgrx（Rust）。如需减少语言，可启用 feature，
例如 `--no-default-features --features "pg17 korean"`（默认包含三种 CJK 语言）。

## 许可证

PostgreSQL License。第三方声明见 [`NOTICE`](NOTICE)。默认构建路径上没有 GPL 代码
（lindera=MIT，ko-dic=Apache-2.0，IPADIC/CC-CEDICT 遵循各自词典许可；Kiwi（LGPL）为
opt-in feature）。
