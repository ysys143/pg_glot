# pg_textsearch_ko

기존 pgvector 위에 **한국어 BM25 + 하이브리드(RRF) 검색**을 얹는 PostgreSQL 확장 가족.
형태소 분석 엔진은 순수 Rust(lindera + 임베드 ko-dic)라 외부 사전 설치가 필요 없다.

> 상태: **개발 초기(스캐폴딩)**. Layer A(`pg_tsvector_ko`) 우선 구현 중. 설계 전문은 [`docs/DESIGN.md`](docs/DESIGN.md).

## 구조 (모노레포, Cargo workspace)

| 구성요소 | 역할 | 의존 |
|---|---|---|
| `crates/korean-tokenizer` | 순수 Rust 한국어 토크나이저 (lindera + ko-dic) | — |
| `extensions/pg_tsvector_ko` | (Layer A) 커스텀 TS parser → `korean` text search config | korean-tokenizer |
| `extensions/pg_textsearch_ko` | (Layer B) 한국어 BM25 + RRF 하이브리드 *(follow-on)* | pg_tsvector_ko + pg_textsearch + pgvector |

설치(예정): `CREATE EXTENSION pg_textsearch_ko CASCADE;` 한 줄로 의존 계층 자동 생성.

## 개발

```bash
make unit          # 순수 Rust 토크나이저 유닛테스트 (PG 불필요)
make run           # cargo pgrx run pg17 → psql
make test          # pg_regress + pg_test (pg17)
```

타깃 PostgreSQL: **17** (pgrx-managed). 토대=pgrx(Rust).

## 라이선스

PostgreSQL License. 제3자 고지는 [`NOTICE`](NOTICE) 참조. 기본 빌드 경로에 GPL 코드 없음
(lindera=MIT, ko-dic=Apache-2.0; Kiwi(LGPL)는 opt-in feature).
