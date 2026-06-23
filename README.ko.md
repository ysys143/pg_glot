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
| `extensions/pg_glot` | (Layer A) 커스텀 TS parser → `korean`/`japanese`/`chinese` config; `glot` 스키마(`glot.rrf`) 소유 | glot-tokenizer |
| `extensions/pg_glot_hybrid` | (Layer B) CJK BM25 + RRF 하이브리드(`glot.hybrid`) | pg_glot + pg_textsearch + pgvector |

## 설치 — 계층별 분리 설치

하나의 모노레포지만 **각 계층은 깨끗한 경계를 가진 독립 확장/크레이트라, 필요한 것만 설치**한다.
`pg_textsearch`·`pgvector`는 흡수가 아니라 의존(`requires`)이라 각자 일정으로 업그레이드한다.

| 원하는 것 | 설치 | 자동 포함 |
|---|---|---|
| 풀 하이브리드(BM25 + dense RRF) | `CREATE EXTENSION pg_glot_hybrid CASCADE;` | pg_glot + pg_textsearch + pgvector (`requires`로 자동) |
| CJK 전문검색만 (`to_tsvector` / `@@` / `ts_rank`) | `CREATE EXTENSION pg_glot;` | 없음 — 추가 의존 0 |
| RRF 융합 프리미티브(`glot.rrf`) | `CREATE EXTENSION pg_glot;` | — (`glot` 스키마는 Layer A에 동봉) |
| PostgreSQL 밖에서 토크나이저만 | `glot-tokenizer` 크레이트 의존 | — |

`pg_textsearch`는 `shared_preload_libraries = 'pg_textsearch'` 필요(프리빌드 Docker 이미지는
설정 완료). 지금은 모노레포지만 경계가 깨끗해 추후 리포 분리는 기계적이다.

## 사용법

### Layer A — CJK 전문검색 (`pg_glot`만)

```sql
CREATE EXTENSION pg_glot;

-- 언어 config(korean / japanese / chinese)로 토큰화
SELECT to_tsvector('korean',   '한국어 형태소 분석');
SELECT to_tsvector('japanese', '東京都に住む');

-- 일반 PostgreSQL 전문검색처럼 매칭/랭킹
SELECT id
FROM   docs
WHERE  to_tsvector('korean', body) @@ to_tsquery('korean', '형태소')
ORDER  BY ts_rank(to_tsvector('korean', body), to_tsquery('korean', '형태소')) DESC;
```

### Layer B — BM25 + 하이브리드 RRF (`pg_glot_hybrid`)

```sql
CREATE EXTENSION pg_glot_hybrid CASCADE;   -- pg_glot + pg_textsearch + pgvector 자동 설치

CREATE TABLE docs (id bigint PRIMARY KEY, body text, emb vector(1024));

-- CJK config 위에 BM25 인덱스 (config 이름은 스키마 한정 필수)
CREATE INDEX ON docs USING bm25(body) WITH (text_config = 'public.korean');
-- dense 인덱스
CREATE INDEX ON docs USING hnsw (emb vector_cosine_ops);

-- BM25 단독 랭킹. 주의: 질의는 리터럴이어야 하고(planner hook),
-- plain ORDER BY ... LIMIT(인덱스 스캔)이어야 한다.
SELECT id FROM docs ORDER BY body <@> '형태소 분석' LIMIT 10;

-- Flagship: 하이브리드(BM25 + dense, RRF 융합)를 일반 KNN처럼 한 줄로.
-- body/emb는 진짜 컬럼, 질의 둘은 리터럴. plain ORDER BY ... LIMIT + 리터럴이면
-- planner가 GlotHybrid custom scan을 선택(두 인덱스 leg + RRF).
SELECT id, body
FROM   docs
ORDER  BY glot.rank(body, emb, '형태소 분석', '[ ... ]'::vector) DESC
LIMIT  10;

-- 명시적 SRF 형태(합성 가능; 동일 RRF 결과, hook 불필요)
SELECT id, score
FROM   glot.hybrid('docs', 'id', 'body', 'emb',
                   '형태소 분석', '[ ... ]'::vector, 60, 60, 10);

-- 또는 미리 만든 id 리스트를 RRF 프리미티브로 직접 융합 (Layer A에 동봉)
SELECT id, score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60);
```

**`glot.rank` (flagship)** 은 `shared_preload_libraries = 'pg_glot_hybrid'` 가 필요하다 —
`GlotHybrid` custom-scan hook을 `_PG_init`에서 등록하기 때문(프리빌드 Docker 이미지는 설정 완료).
hook이 없으면 planner가 질의를 재작성하지 못해 `glot.rank`가 **비-RRF 점수로 폴백**한다.
`body`/`emb`는 진짜 컬럼 참조, 질의 둘은 리터럴; 테이블에 BM25 인덱스 필요(HNSW도 있으면 dense가
인덱스, 없으면 exact 스캔).

**`glot.hybrid` (명시적 형태):** 첫 인자(`'docs'`, `regclass`)가 대상 테이블, 그 뒤 셋은 키/본문/
벡터 컬럼. 테이블에 BM25 인덱스(`text_config` 일치)와 벡터 인덱스가 있어야 하고 키 컬럼은
`bigint`. preload hook 없이도 동작. 필요하면 스키마 한정: `'myschema.docs'`.

`'public.korean'`(및 `'korean'`)을 `japanese`/`chinese`로 바꾸면 언어가 전환된다.

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
(lindera=MIT, ko-dic=Apache-2.0, IPADIC/CC-CEDICT=각 사전 라이선스). Kiwi(LGPL) 백엔드는
설계는 됐으나([`docs/DESIGN.md`](docs/DESIGN.md) D5) **아직 미구현** — 추가 시 opt-in·동적링크.
