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
| `extensions/pg_glot` | (Layer A) 自定义 TS parser → `korean`/`japanese`/`chinese` config；拥有 `glot` schema（`glot.rrf`） | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF 混合（`glot.hybrid`） | pg_glot + pg_textsearch + pgvector |

## 安装 — 按层分离安装

虽是单一 monorepo，但**每层都是边界清晰的独立扩展/crate，因此只需安装所需部分。**
`pg_textsearch`・`pgvector` 是依赖（`requires`）而非内置，可按各自节奏升级。

| 你想要 | 安装 | 自动带入 |
|---|---|---|
| 完整混合（BM25 + dense RRF） | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector（经 `requires` 自动） |
| 仅 CJK 全文检索（`to_tsvector` / `@@` / `ts_rank`） | `CREATE EXTENSION pg_glot;` | 无 — 零额外依赖 |
| RRF 融合原语（`glot.rrf`） | `CREATE EXTENSION pg_glot;` | —（`glot` schema 随 Layer A 提供） |
| 在 PostgreSQL 之外仅用分词器 | 依赖 `glot-tokenizer` crate | — |

`pg_textsearch` 需要 `shared_preload_libraries = 'pg_textsearch'`（预构建 Docker 镜像已配置）。
当前为 monorepo，但边界清晰，日后拆分为独立仓库是机械性的。

## 用法

### Layer A — CJK 全文检索（仅 `pg_glot`）

```sql
CREATE EXTENSION pg_glot;

-- 用语言 config（korean / japanese / chinese）分词
SELECT to_tsvector('korean',   '한국어 형태소 분석');
SELECT to_tsvector('chinese',  '北京欢迎你');

-- 像普通 PostgreSQL 全文检索一样匹配/排序
SELECT id
FROM   docs
WHERE  to_tsvector('chinese', body) @@ to_tsquery('chinese', '北京')
ORDER  BY ts_rank(to_tsvector('chinese', body), to_tsquery('chinese', '北京')) DESC;
```

### Layer B — BM25 + 混合 RRF（`pg_glot_hybrid`）

```sql
CREATE EXTENSION pg_glot_hybrid CASCADE;   -- 自动安装 pg_glot + pg_textsearch + pgvector

CREATE TABLE docs (id bigint PRIMARY KEY, body text, emb vector(1024));

-- 在 CJK config 上建 BM25 索引（config 名需 schema 限定）
CREATE INDEX ON docs USING bm25(body) WITH (text_config = 'public.chinese');
-- dense 索引
CREATE INDEX ON docs USING hnsw (emb vector_cosine_ops);

-- 仅 BM25 排序。注意: 查询须为字面量（planner hook），
-- 且为 plain ORDER BY ... LIMIT（索引扫描）。
SELECT id FROM docs ORDER BY body <@> '北京 大学' LIMIT 10;

-- 一次调用 = BM25(body) + dense(emb) 经 RRF 融合
SELECT id, score
FROM   glot.hybrid(
           'docs',                -- rel (regclass)
           'id', 'body', 'emb',   -- key / text / vector 列
           '北京 大学',           -- 查询文本   (BM25 leg)
           '[ ... ]'::vector,     -- 查询向量   (dense leg)
           k       => 60,         -- RRF k      (默认 60)
           per_leg => 60,         -- 每路 top-k (默认 60)
           n       => 10);        -- 最终行数   (默认 10)

-- 或用 RRF 原语直接融合你预先算好的 id 列表（随 Layer A 提供）
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

**选择表:** 第一个参数（`'docs'`，`regclass`）即要检索的表，随后三个是该表的键/正文/向量列名。
该表须已建有正文列的 BM25 索引（与 `text_config` 一致）和向量列的向量索引，且键列须为 `bigint`。
如需可加 schema 限定: `'myschema.docs'`。

将 `'public.chinese'`（及 `'chinese'`）换成 `korean`/`japanese` 即可切换语言。

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
（lindera=MIT，ko-dic=Apache-2.0，IPADIC/CC-CEDICT 遵循各自词典许可）。Kiwi（LGPL）后端
已设计（见 [`docs/DESIGN.md`](docs/DESIGN.md) D5）但**尚未实现** — 若加入将为 opt-in・动态链接。
