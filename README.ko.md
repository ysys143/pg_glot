# pg_glot_hybrid

[English](README.md) · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

기존 pgvector 위에 **CJK(한국어·일본어·중국어) BM25 + 하이브리드(RRF) 검색**을 얹는
PostgreSQL 확장 가족. 형태소·분절 엔진은 순수 Rust(lindera + 임베드 사전)라 외부 사전
설치가 필요 없다.

> 상태: Layer A(`pg_glot`)·Layer B(`pg_glot_hybrid`) 동작. MIRACL dev로 BM25/RRF 실측
> ([`bench/RESULTS.md`](bench/RESULTS.md)). 설계 전문은 [`docs/DESIGN.md`](docs/DESIGN.md).
> ko가 가장 엄밀하게 검증됨(POS ablation·research parity); ja/zh도 측정값은 있으나 제품
> 품질은 ko 수준으로 검증 전.

## 구조 (모노레포, Cargo workspace)

| 구성요소 | 역할 | 의존 |
|---|---|---|
| `crates/glot-tokenizer` | 순수 Rust CJK 토크나이저 (lindera + 임베드 ko-dic/IPADIC/CC-CEDICT) | — |
| `extensions/pg_glot` | (Layer A) 커스텀 TS parser → `korean`/`japanese`/`chinese` text search config | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF 하이브리드(`glot.hybrid`) | pg_glot + pg_textsearch + pgvector |

설치: `CREATE EXTENSION pg_glot_hybrid CASCADE;` 한 줄로 의존 계층 자동 생성.

## 검색 품질 (MIRACL dev, 실측)

`bench/`로 실제 pg_glot + pg_textsearch BM25를 측정. 상세·한계·재현은
[`bench/RESULTS.md`](bench/RESULTS.md). **dev passages subset 측정이라 공식 리더보드와 직접
비교 불가(indicative).**

| lang | config | BM25 NDCG@10 | R@10 | RRF NDCG@10 |
|---|---|---|---|---|
| ko | `korean`   | **0.636** | 0.798 | 0.755 |
| ja | `japanese` | **0.565** | 0.773 | 0.691 |
| zh | `chinese`  | **0.459** | 0.646 | 0.625 |

ko BM25는 research MeCab(0.6385)의 99.7%. RRF(dense BGE-M3 + BM25)는 3개 언어 모두 BM25를
유의하게 +0.12~0.17 향상(p<0.001).

**lindera 없이 stock PG로는?** (형태소 분석 없는 대조군, recall@10)

| lang | PG native(simple) | pg_trgm | **lindera** |
|---|---|---|---|
| ko | 0.479 | 0.327 | **0.798** |
| ja | 0.179 | 0.516 | **0.773** |
| zh | 0.017 | 0.364 | **0.646** |

공백 없는 ja/zh에서 native simple은 거의 붕괴(zh R 0.017), pg_trgm도 부분문자열만 잡는다.
형태소·분절(lindera)이 CJK 검색의 핵심임을 보여준다.

## 개발

```bash
make unit          # 순수 Rust 토크나이저 유닛테스트 (PG 불필요)
make run           # cargo pgrx run pg17 → psql
make test          # pg_regress + pg_test (pg17)
```

타깃 PostgreSQL: **17** (pgrx-managed). 토대=pgrx(Rust). 언어를 줄이려면
`--no-default-features --features "pg17 korean"`처럼 feature를 켠다(기본은 CJK 셋 다).

## 라이선스

PostgreSQL License. 제3자 고지는 [`NOTICE`](NOTICE) 참조. 기본 빌드 경로에 GPL 코드 없음
(lindera=MIT, ko-dic=Apache-2.0, IPADIC/CC-CEDICT=각 사전 라이선스; Kiwi(LGPL)는 opt-in feature).
