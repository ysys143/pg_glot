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
| `extensions/pg_glot` | (Layer A) カスタム TS parser → `korean`/`japanese`/`chinese` text search config | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF ハイブリッド（`glot.hybrid`） | pg_glot + pg_textsearch + pgvector |

インストール: `CREATE EXTENSION pg_glot_hybrid CASCADE;` の一行で依存スタックを自動構築。

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
コードなし（lindera=MIT、ko-dic=Apache-2.0、IPADIC/CC-CEDICT は各辞書ライセンス。Kiwi（LGPL）
は opt-in feature）。
