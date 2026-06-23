# pg_glot

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

**pg_glot** は PostgreSQL に **CJK（韓国語・日本語・中国語）の全文検索と BM25 + dense
ハイブリッド（RRF）検索**を加える（pgvector の上に）。形態素・分割エンジンは純 Rust（lindera +
組み込み辞書）なので、外部辞書のインストールは不要。

独立して採用できる 2 つの拡張として提供される:

- **`pg_glot`** — CJK トークナイザと `korean` / `japanese` / `chinese` のテキスト検索
  config（通常の PostgreSQL 全文検索）、および `glot.rrf` 融合プリミティブ。
- **`pg_glot_hybrid`** — BM25 インデックスと BM25 + dense ハイブリッド検索（`glot.rank`,
  `glot.hybrid`）。`pg_glot` + `pg_textsearch` + `pgvector` の上に載る。

> **ステータス** — 両拡張とも動作。MIRACL dev で BM25/RRF を実測
> （[`bench/RESULTS.md`](bench/RESULTS.md)）。ko が最も厳密に検証済み（POS ablation・research
> との同等性）。ja/zh も測定値はあるが、製品品質は ko 水準では未検証。設計・意思決定の全文は
> [`docs/DESIGN.md`](docs/DESIGN.md)。

## コンポーネント

単一のモノレポ（Cargo workspace）。各拡張はクリーンな境界を持ち、独立してインストールできる:

| コンポーネント | 役割 | 依存 |
|---|---|---|
| `crates/glot-tokenizer` | 純 Rust の CJK トークナイザ（lindera + 組み込み ko-dic / IPADIC / CC-CEDICT） | — |
| `extensions/pg_glot` | カスタム TS parser → `korean` / `japanese` / `chinese` config；`glot` スキーマ（`glot.rrf`）を所有 | glot-tokenizer |
| `extensions/pg_glot_hybrid` | CJK BM25 + RRF ハイブリッド — `glot.rank` custom scan（`ORDER BY … LIMIT`）+ `glot.hybrid` SRF | pg_glot + pg_textsearch + pgvector |

## インストール

**必要なものだけをインストールする** — `pg_textsearch`・`pgvector` は取り込みではなく依存
（`requires`）で、各自のスケジュールでアップグレードできる。

| 目的 | インストール | 自動的に入る |
|---|---|---|
| フルハイブリッド（BM25 + dense RRF） | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector（`requires` で自動） |
| CJK 全文検索のみ（`to_tsvector` / `@@` / `ts_rank`） | `CREATE EXTENSION pg_glot;` | なし — 追加依存ゼロ |
| RRF 融合プリミティブ（`glot.rrf`） | `CREATE EXTENSION pg_glot;` | —（`glot` スキーマは `pg_glot` に同梱） |
| PostgreSQL 外でトークナイザのみ | `glot-tokenizer` クレートに依存 | — |

ハイブリッド経路には `shared_preload_libraries = 'pg_textsearch, pg_glot_hybrid'` が必要
（プリビルドの Docker イメージは設定済み）。`pg_textsearch` は BM25 用、`pg_glot_hybrid` は
`glot.rank` custom-scan hook の登録用。境界がクリーンなので将来のリポ分離は機械的。

## 使い方

### `pg_glot` — CJK 全文検索

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

### `pg_glot_hybrid` — BM25 + ハイブリッド RRF

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

-- Flagship: ハイブリッド（BM25 + dense、RRF 融合）を通常の KNN のように 1 行で。
-- body/emb は実列、2 つの質問はリテラル。plain ORDER BY ... LIMIT + リテラルなら
-- planner が GlotHybrid custom scan を選ぶ（2 つのインデックス leg + RRF）。
SELECT id, body
FROM   docs
ORDER  BY glot.rank(body, emb, '東京 大学', '[ ... ]'::vector) DESC
LIMIT  10;

-- 明示的な SRF 形式（合成可能; 同じ RRF 結果、hook 不要）
SELECT id, score
FROM   glot.hybrid('docs', 'id', 'body', 'emb',
                   '東京 大学', '[ ... ]'::vector, 60, 60, 10);

-- または事前計算した id リストを RRF プリミティブで直接融合（pg_glot に同梱）
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

**`glot.rank`（flagship）** は `shared_preload_libraries = 'pg_glot_hybrid'` が必要 —
`GlotHybrid` custom-scan hook を登録するため（プリビルドの Docker イメージは設定済み）。hook が
無いと planner が質問を書き換えられず `glot.rank` は **非 RRF スコアにフォールバック**する。
`body`/`emb` は実列参照、2 つの質問はリテラル; テーブルに BM25 インデックスが必要（HNSW もあれば
dense がインデックス、無ければ exact スキャン）。

**`glot.hybrid`（明示形式）** は第 1 引数（`'docs'`, `regclass`）が対象テーブル、続く 3 つは
キー/本文/ベクトル列。テーブルに BM25 インデックス（`text_config` 一致）とベクトルインデックスが
必要でキー列は `bigint`。preload hook 無しでも動作。必要ならスキーマ修飾: `'myschema.docs'`。

`'public.japanese'`（および `'japanese'`）を `korean`/`chinese` に変えれば言語が切り替わる。

## 検索品質（MIRACL dev、実測）

実際の `pg_glot` + `pg_textsearch` の BM25 インデックスを `bench/` で測定。詳細・制約・再現は
[`bench/RESULTS.md`](bench/RESULTS.md)。**dev passages のサブセットなので公式リーダーボードと
直接比較は不可（参考値）。**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

ko の BM25 は research MeCab（0.6385）の 99.7%。RRF（dense BGE-M3 + BM25）は 3 言語すべてで
BM25 を有意に +0.12〜0.17 改善（p < 0.001）。

**lindera なしの素の PostgreSQL では?**（形態素解析なし、recall@10）

| lang | PG native（`simple`） | pg_trgm | **lindera** |
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
