# pg_glot_hybrid

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

pgvector の上に **CJK（韓国語・日本語・中国語）の BM25 + ハイブリッド（RRF）検索**を載せる
PostgreSQL 拡張ファミリ。形態素・分割エンジンは純 Rust（lindera + 組み込み辞書）なので、
外部辞書のインストールは不要。

> ステータス: Layer A（`pg_glot`）・Layer B（`pg_glot_hybrid`）が動作。MIRACL dev で BM25/RRF
> を実測（[`bench/RESULTS.md`](bench/RESULTS.md)）。設計の全文は
> [`docs/DESIGN.md`](docs/DESIGN.md)。ko が最も厳密に検証済み（POS ablation・research との
> 同等性）。ja/zh も測定値はあるが、製品品質は ko 水準では未検証。

## 構成（モノレポ、Cargo workspace）

| コンポーネント | 役割 | 依存 |
|---|---|---|
| `crates/glot-tokenizer` | 純 Rust の CJK トークナイザ（lindera + 組み込み ko-dic/IPADIC/CC-CEDICT） | — |
| `extensions/pg_glot` | (Layer A) カスタム TS parser → `korean`/`japanese`/`chinese` config；`glot` スキーマ（`glot.rrf`）を所有 | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF ハイブリッド（`glot.hybrid`） | pg_glot + pg_textsearch + pgvector |

## インストール — 層ごとに分離

単一のモノレポだが、**各層はクリーンな境界を持つ独立した拡張/クレートなので、必要なものだけを
インストールする。** `pg_textsearch`・`pgvector` は取り込みではなく依存（`requires`）で、各自の
スケジュールでアップグレードできる。

| 目的 | インストール | 自動的に入る |
|---|---|---|
| フルハイブリッド（BM25 + dense RRF） | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector（`requires` で自動） |
| CJK 全文検索のみ（`to_tsvector` / `@@` / `ts_rank`） | `CREATE EXTENSION pg_glot;` | なし — 追加依存ゼロ |
| RRF 融合プリミティブ（`glot.rrf`） | `CREATE EXTENSION pg_glot;` | —（`glot` スキーマは Layer A に同梱） |
| PostgreSQL 外でトークナイザのみ | `glot-tokenizer` クレートに依存 | — |

`pg_textsearch` は `shared_preload_libraries = 'pg_textsearch'` が必要（プリビルドの Docker
イメージは設定済み）。現在はモノレポだが、境界がクリーンなので将来のリポ分離は機械的。

## 使い方

### Layer A — CJK 全文検索（`pg_glot` のみ）

```sql
CREATE EXTENSION pg_glot;

-- 言語 config（korean / japanese / chinese）でトークン化
SELECT to_tsvector('korean',   '한국어 형태소 분석');
SELECT to_tsvector('japanese', '東京都に住む');

-- 通常の PostgreSQL 全文検索と同様にマッチ/ランキング
SELECT id
FROM   docs
WHERE  to_tsvector('japanese', body) @@ to_tsquery('japanese', '東京')
ORDER  BY ts_rank(to_tsvector('japanese', body), to_tsquery('japanese', '東京')) DESC;
```

### Layer B — BM25 + ハイブリッド RRF（`pg_glot_hybrid`）

```sql
CREATE EXTENSION pg_glot_hybrid CASCADE;   -- pg_glot + pg_textsearch + pgvector を自動導入

CREATE TABLE docs (id bigint PRIMARY KEY, body text, emb vector(1024));

-- CJK config 上の BM25 インデックス（config 名はスキーマ修飾が必須）
CREATE INDEX ON docs USING bm25(body) WITH (text_config = 'public.japanese');
-- dense インデックス
CREATE INDEX ON docs USING hnsw (emb vector_cosine_ops);

-- BM25 単独ランキング。注意: 質問はリテラル（planner hook）で、
-- plain ORDER BY ... LIMIT（インデックススキャン）であること。
SELECT id FROM docs ORDER BY body <@> '東京 大学' LIMIT 10;

-- 1 回の呼び出し = BM25(body) + dense(emb) を RRF で融合
SELECT id, score
FROM   glot.hybrid(
           'docs',                -- rel (regclass)
           'id', 'body', 'emb',   -- key / text / vector 列
           '東京 大学',           -- 質問テキスト  (BM25 leg)
           '[ ... ]'::vector,     -- 質問ベクトル  (dense leg)
           k       => 60,         -- RRF k         (既定 60)
           per_leg => 60,         -- leg ごとの top-k (既定 60)
           n       => 10);        -- 最終行数      (既定 10)

-- または事前計算した id リストを RRF プリミティブで直接融合（Layer A に同梱）
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

`'public.japanese'`（および `'japanese'`）を `korean`/`chinese` に変えれば言語が切り替わる。

## 検索品質（MIRACL dev、実測）

実際の pg_glot + pg_textsearch の BM25 インデックスを `bench/` で測定。詳細・制約・再現は
[`bench/RESULTS.md`](bench/RESULTS.md)。**dev passages のサブセットなので公式リーダーボードと
直接比較は不可（参考値）。**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

ko の BM25 は research MeCab（0.6385）の 99.7%。RRF（dense BGE-M3 + BM25）は 3 言語すべてで
BM25 を有意に +0.12〜0.17 改善（p<0.001）。

**lindera なしの素の PG では?**（形態素解析なし、recall@10）

| lang | PG native（simple） | pg_trgm | **lindera** |
|---|---|---|---|
| ko | 0.479 | 0.327 | **0.798** |
| ja | 0.179 | 0.516 | **0.773** |
| zh | 0.017 | 0.364 | **0.646** |

空白のない ja/zh では native の `simple` がほぼ崩壊（zh R 0.017）、pg_trgm も部分文字列しか
捉えられない。形態素・分割（lindera）が CJK 検索の要であることを示す。

## 開発

```bash
make unit          # 純 Rust トークナイザの単体テスト（PG 不要）
make run           # cargo pgrx run pg17 → psql
make test          # pg_regress + pg_test (pg17)
```

対象 PostgreSQL: **17**（pgrx 管理）。基盤 = pgrx（Rust）。言語を絞るには feature を指定
（例: `--no-default-features --features "pg17 korean"`、デフォルトは CJK 3 言語）。

## ライセンス

PostgreSQL License。第三者の告知は [`NOTICE`](NOTICE) を参照。デフォルトのビルド経路に GPL
コードなし（lindera=MIT、ko-dic=Apache-2.0、IPADIC/CC-CEDICT は各辞書ライセンス）。Kiwi（LGPL）
バックエンドは設計済みだが（[`docs/DESIGN.md`](docs/DESIGN.md) D5）**未実装** — 追加時は
opt-in・動的リンク。
