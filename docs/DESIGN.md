# pg_glot_hybrid — 설계 및 의사결정 문서

- 상태: 설계 확정 단계 (구현 직전). 토대·토크나이저·통합방식·라이선스·데이터까지 검증으로 닫힘.
- 최종 갱신: 2026-06-19
- 인접 연구 레포: `/Users/jaesolshin/Documents/GitHub/textsearch` (8단계 한국어 검색 실험)

---

## 1. 목표와 배경

### 1.1 출발점
인접 연구(`textsearch` 레포)가 8단계 실험으로 도출한 결론:

> **textsearch_ko(MeCab) + pg_textsearch BM25 + pgvector HNSW + DB-side RRF**
> → MIRACL NDCG 0.77 @1.79ms / EZIS NDCG 0.86 @0.92ms.
> Elasticsearch와 품질 동등, 단일 노드 latency 2~5배 우위.

### 1.2 문제의식
이 스택을 쓰려면 사용자가 mecab 설치 → textsearch_ko 설치 → pg_textsearch 설치 → pgvector 설치 → RRF SQL 직접 작성을 해야 함. **번거롭다.** 이를 `pg_glot_hybrid`라는 단일 확장으로 "원클릭"화하는 것이 목표.

### 1.3 1차 목표
- **공개용 확장(커뮤니티 배포) + 상업 사용 가능.**
- "원클릭" 체감 = 자기완결 + 최소 설치.

---

## 2. 아키텍처 개요

### 2.1 핵심 통찰
4개를 하나의 `.so`로 합치지 않는다. **"글루(직접 구현) + 의존선언 + 배포 패키징"** 세 가지로 푼다. 연구의 진짜 산출물(= 남이 안 가진 것)은 **RRF 하이브리드 SQL 글루 + 한국어 토크나이저↔FTS 통합**이다.

### 2.2 4겹 스택
```
┌─ pg_glot_hybrid (우리가 직접 구현하는 pgrx 확장) ──────────────────┐
│  ① lindera (의존 crate, 순수 Rust + 임베드 ko-dic)         ← 토크나이저 엔진
│       ↑ 호출                                                          │
│  ② 우리 glue (직접 구현):                                             │
│     (a) 커스텀 TS parser → CREATE TEXT SEARCH CONFIGURATION korean    │  ← 통합점(필수)
│         (+ opt-in feature: korean_kiwi)                               │
│     (b) glot.hybrid() 등 RRF 하이브리드 SQL                      │  ← 검색 오케스트레이션
└───────────────────────────────────────────────────────────────────────┘
        │ requires                              │ requires
        ▼                                       ▼
  ③ pg_textsearch (BM25 인덱스)            ④ pgvector (HNSW Dense)
     text_config='korean' 으로                  벡터 컬럼 검색
     우리 korean config를 소비
```

### 2.3 인덱스/쿼리 흐름
- **색인**: 텍스트 → (`korean` config = lindera) → pg_textsearch BM25 인덱스 / 임베딩 컬럼 → pgvector HNSW
- **쿼리**: 질의텍스트 → BM25(pg_textsearch) top-k + 질의벡터 → Dense(pgvector) top-k → `glot.hybrid` RRF 융합 → 최종 순위

---

### 2.4 계층 분해 + 리포 전략 (2026-06-19 확정)
BM25 계층을 pg_textsearch에 고정시키지 않고, 재사용성을 위해 **2계층으로 분해**한다(하이브리드 RRF는 별도 확장 없이 B에 흡수 — 2026-06-19 갱신). 재사용성은 "별도 리포"가 아니라 **깨끗한 확장/크레이트 경계**에서 나오므로, **v0.1은 모노레포(Cargo workspace)로 시작하고 API/seam 검증 후 별도 리포로 분리**한다(경계가 이미 있어 분리는 기계적).

```
pg_glot_hybrid/  (모노레포, Cargo workspace)   ※ 이름은 스캐폴딩 시 확정
├── crates/
│   └── glot-tokenizer/   순수 Rust lib: lindera 래퍼 + analyzer trait (+opt-in kiwi)
│                            → crates.io 게시, Postgres 밖에서도 재사용
├── extensions/
│   ├── (A) pg_glot    pgrx: 커스텀 TS parser → korean config. (의존: glot-tokenizer)
│   │                          순수 한국어 FTS/GIN/ts_rank만 원하면 이것만 설치. textsearch_ko 대체물.
│   └── (B) pg_glot_hybrid  pgrx: BM25 + RRF 하이브리드. (requires: A + pg_textsearch + vector)
│                              "깔면 기존 pgvector 위에 한국어 BM25+RRF가 얹힘"
├── bakeoff/  docs/  ...
```

- **설치 UX는 단일 명령**: `CREATE EXTENSION pg_glot_hybrid CASCADE` → requires 체인으로 pg_glot + pg_textsearch + vector 자동 생성. 분리해도 풀스택 사용자 비용 0, 순수 FTS 사용자는 pg_glot만.
- **dense 생태계 자동 호환**: RRF의 dense leg를 pgvector 표준 인터페이스(`ORDER BY embedding <=> q`)에 작성 → **pg_acorn / pg_cuvs / pgvectorscale / VectorChord(vchord)** 등 pgvector 호환 가속이 깔려 있으면 **투명하게 자동 적용**. dense 백엔드 어댑터(seam) 불필요(과설계 회피). 임베딩 생성은 범위 밖(사용자 제공) 또는 **pg_aidb**(ysys143 RAG 플랫폼) 통합.

- **각 계층 독립 설치 가능**(control 파일별 `CREATE EXTENSION`), 토크나이저는 크레이트로 독립 재사용 → 모노레포라도 재사용성 100% 유지.
- **이름 확정(2026-06-19, 2계층)**: A=`pg_glot`(토크나이저/korean config), B=`pg_glot_hybrid`(BM25+RRF). **별도 `pg_hybrid_ko` 폐기 — RRF는 B에 흡수.** 공유 crate=`glot-tokenizer`, repo=`pg_glot_hybrid`(namesake).
- B의 BM25는 재구현 아님(pg_textsearch 의존 + 한국어 config + RRF 편의함수). 자체 BM25 독립은 범위 밖.

## 3. 핵심 의사결정 (결정 / 근거 / 증거 / 기각된 대안)

### D1. 확장 형태 = 메타 확장 (글루 + 의존 + Docker)
- **결정**: pg_textsearch·pgvector는 흡수하지 않고 `requires`로 의존. 우리는 글루(TS parser + RRF SQL)만 직접 구현. 진짜 원클릭 체감은 Docker 프리빌드 이미지로 `CREATE EXTENSION pg_glot_hybrid CASCADE` 한 줄.
- **근거**: 남의 확장을 통째로 재배포하면 라이선스·유지보수·릴리스 사이클이 전부 우리에게 묶임. `CREATE EXTENSION ... CASCADE`는 의존 확장을 자동 생성하나, 네이티브 라이브러리/.so 빌드까지 자동화하진 못함 → 진짜 원클릭은 Docker 또는 빌드 스크립트.

### D2. 토대 = pgrx (Rust)
- **결정**: 확장을 pgrx(Rust)로 작성.
- **근거**: 토크나이저를 순수 Rust(lindera)로 정했기 때문(D3). pg-extension-lab 스킬이 PGXS·pgrx 양쪽 1급 지원이라 "스킬 지원"은 결정 근거가 아니었고, 순수하게 토크나이저 선택이 토대를 결정.

### D3. 기본 토크나이저 = lindera (ko-dic 임베드)
- **결정**: 기본 MeCab 계열 백엔드 = **lindera**. hephaex/mecab-ko = 예비(2순위). C/PGXS + C++ mecab 폴백 = 불필요.
- **근거 (bake-off 실측, §5)**: lindera NDCG@10 0.3287 ≥ baseline(python-mecab-ko) 0.3147, 토큰 Jaccard 0.965~0.976, 최속, ko-dic 바이너리 임베드(외부 사전 0), 성숙도 압도(crates.io 130만+ DL·다수 기여·Meilisearch 실사용), MIT(+ko-dic Apache).
- **기각**:
  - *eunjeon mecab-ko (C++)*: 엔진은 ~20년 검증됐으나 **2018-07-20 이후 휴면**. C++ → libmecab 링크/libstdc++/GPL트리플 → 순수-Rust 원클릭 정체성과 충돌.
  - *hephaex/mecab-ko (순수 Rust)*: NDCG 동등이나 **성숙도 리스크 높음**(1인·2026 신생·DL 344), 동등성 미검증(CRF 재학습 트랙 + 세종 후처리로 분기 가능), 사전 자체빌드·`MECAB_DICDIR` 필수·sejong CLI 버그 등 운영부담.
  - *vibrato(미평가)*: 진지한 순수-Rust 대안이나 기성 ko-dic 패키지 없음(직접 컴파일), 같은 사전이라 출력은 lindera와 거의 동일할 것 → 토대 뒤집을 가능성 낮음. 필요시 bake-off 추가 가능.
  - *sudachi.rs / goya*: 일본어 전용(ko-dic 없음) → 제외.

### D4. tsvector 통합 = 자체 구현 (커스텀 TS parser), textsearch_ko 미사용
- **결정**: 한국어 `korean` 텍스트서치 config를 **pgrx에서 lindera 백엔드로 직접 구현**. textsearch_ko는 쓰지 않음.
- **근거/증거**:
  - **pg_textsearch는 자체 토크나이저가 없고 PG regconfig에 위임**: `USING bm25(content) WITH (text_config='english')`, `text_config` required (vendor `pg_textsearch_original/README.md:167,548`). 연구 배선: `to_tsvector('public.korean', text)` + `CREATE INDEX USING bm25(text) WITH (text_config='public.korean')` (`experiments/phase7_scaling/phase7_hybrid_setup.py:100,110`). **미리 만든 tsvector 컬럼 인덱싱 경로 없음** → `korean` regconfig가 반드시 필요.
  - **textsearch_ko는 MeCab 하드와이어**: `extensions/textsearch_ko/ts_mecab_ko.c:22 #include <mecab.h>`, `mecab_node_t`/`mecab_new` 직접 호출, mecab-ko-dic 전용 CSV 필드 파싱. `CREATE TEXT SEARCH PARSER korean`의 파서 함수가 전부 libmecab 호출. → lindera를 감쌀 수 없음.
  - **연구는 다른 분석기를 textsearch_ko로 테스트하지 않았다**: `experiments/phase2_tsvector/phase1_tsvector.py`에 두 분리 경로 — textsearch_ko(`to_tsvector('korean',…)`, MeCab 전용) vs `setup_python_bm25_index`(Python `BM25Embedder_PG(tokenizer=...)`, {mecab,kiwi-cong,kiwi-knlm,okt}). kiwi/okt는 **Python 토큰화 → sparse/역색인**으로 측정됐지 textsearch_ko가 아님. "OS레벨 등록"은 MeCab 사전(mecab-ko-dic) 설치를 의미하며 여전히 MeCab.
- **두 축의 결합성**: textsearch_ko ↔ C++ mecab-ko는 한 몸(libmecab 전용), lindera ↔ 자체구현도 한 몸. 즉 lindera를 고르면 자체구현이 강제됨.
- **추가 이점**: 연구에는 PG 안에서 분석기를 갈아끼우는 통합 pluggable glue가 없었음(MeCab=PG-native, 나머지=Python 우회). 우리 자체구현 seam은 이를 PG-native하게 통일하는 **개선**.
- **수용한 트레이드오프**: 커스텀 TS parser(`prsstart`/`gettoken`/`prsend`/`lextype`, pg_sys 저수준) 작성 필요. textsearch_ko C 레퍼런스 + pg_regress TDD로 관리 가능하다고 합의.

### D5. v0.1 분석기 구성 = lindera 기본 + Kiwi opt-in
- **결정**: 기본 빌드 = lindera 단독(순수 Rust·자기완결·원클릭). **Kiwi = feature-flag opt-in**(libkiwi LGPL FFI 동적링크).
- **근거**: 연구 서사가 "MeCab=속도/안정성, kiwi-cong=품질 1위" → 사용자 실질 가치 = MeCab계열 + Kiwi. hephaex는 lindera와 같은 mecab-ko-dic 계열이라 사용자 체감 가치 적음. Kiwi를 opt-in으로 두면 기본은 순수-Rust 자기완결 유지, 품질 원할 때만 LGPL 도입. pluggable seam을 2종으로 실증.
- **제외**: JVM 분석기(Okt/KKMA) — in-process pgrx 임베드 부적합.

### D6. 하이브리드 = DB-side RRF (2단 API)
- **결정**: B(pg_glot_hybrid)에 RRF를 **2층 API**로 제공.
  - **`glot.rrf(bm25_ranked, dense_ranked, k)`** — 일반 fusion 프리미티브. (id, rank) 리스트만 받아 융합. 백엔드·언어·스키마 무관, 커스텀 스키마/외부(pg_aidb)도 재사용.
  - **`glot.hybrid(rel, key_col, text_col, vec_col, q_text, q_vec, k)`** — pgvector 편의 어댑터. 내부에서 BM25 leg(korean config+pg_textsearch) + dense leg(pgvector `<=>`) 실행 후 `glot.rrf` 융합. 흔한 케이스 원콜. 연구 Phase 7 DB-side RRF CTE를 함수로 포장.
- **근거**: 연구가 DB-side RRF의 latency 우위(왕복 1회) 입증. 2층 분리로 casual=원콜 / power=glot.rrf+CTE 자유조합(스키마 과적합 회피).
- **핵심**: dense leg를 pgvector 표준 `<=>`로 작성 → "pgvector RRF 어댑터" 하나가 pg_acorn/pg_cuvs/pgvectorscale/vchord 전체 어댑터 역할(별도 N개 불필요). 어댑터는 key_col(PK/ctid)로 두 leg를 묶음. 여전히 thin(RRF ~20줄 + 편의 래퍼).
- **선택적 융합 method (Bayesian, 미확정·측정 대상)**: `glot.hybrid(..., method => 'rrf'|'bayesian')`로 융합을 플러그블화. Bayesian = BM25 점수를 확률로 캘리브레이션 후 dense와 융합(cf. instructkr/**bb25**, Rust). 단 **기본은 RRF**: 연구 Phase 7 실측에서 Bayesian은 도메인 의존(EZIS 0.9249>RRF 0.8641, MIRACL 0.7272<RRF 0.7683) + 5배 느림(9.55 vs 1.79ms). bb25 +1.0%(영어)도 명백한 승리 아님. → **opt-in으로만**, testbench(MDN vs MIRACL 도메인역전)에서 RRF 대비 실측해 자리값 증명. **bb25 라이선스 미명시 → 코드 벤더링 금지, 캘리브레이션 로직은 공개기법이라 자체 재구현.** **(Codex 리뷰: ABI + 인덱스 의미가 안정될 때까지 Bayesian 착수 보류 — distraction.)**

### D7. 의존 = pg_textsearch + pgvector (requires)
- **결정**: 둘 다 흡수하지 않고 `requires`로 의존.
- **근거**: 둘 다 PostgreSQL License(허용적). 흡수 시 유지보수/릴리스 결합도↑.
- **BM25 엔진 대안 검증(2026-06-19, WebFetch 실측)**: 자체구현은 비권장(인덱스 AM=난제 시스템 프로젝트, 차별점 아님). 대안 엔진:
  - **pg_search (ParadeDB)** = **AGPL-3.0**(LICENSE 확인) → permissive·상업 목표 **dealbreaker**. + Tantivy 자체 토크나이저라 regconfig 미소비(lindera-tantivy 별도 통합 필요).
  - **VectorChord-bm25 (TensorChord)** = **AGPL-3.0 OR ELv2 듀얼**(LICENSE 확인) → 둘 다 permissive 아님. + 자체 토크나이저.
  - **pg_textsearch (Timescale)** = PostgreSQL License + **우리 `korean` regconfig 그대로 소비**(통합비용 0) → **1순위 확정**.
  - 어댑터 복수지원은 v0.1 **YAGNI**(타 엔진은 regconfig 미소비라 어댑터 비용 큼). Layer B에 BM25 백엔드 seam만 남기고 기본=pg_textsearch.

### D8. 라이선스 = PostgreSQL License (또는 BSD 2-clause)
- **결정**: 확장 본체 = PostgreSQL License(또는 BSD). 기본 경로 GPL 0.
- **근거**: §6 라이선스 감사 결과 전 구성요소 허용적. 상업 사용 OK.

### D9. testbench & 데이터
- **결정**: 2-tier — (1) 레포 동봉 마이크로 코퍼스(스모크/pg_regress), (2) 다운로드형 풀 벤치(NDCG/scaling). 정확성=pg_regress, 품질=`ko_bench`. 데이터: **MIRACL-ko**(Dense우세 + scaling), **MDN-ko**(BM25우세/도메인역전). EZIS는 동봉 불가(§6).
- **근거**: §5/§6/§7.

---

### D10. 계층 분해 + 모노레포-우선 리포 전략
- **결정**: **2계층**(A=`pg_glot` 토크나이저/config, B=`pg_glot_hybrid` BM25+RRF) + 공유 `glot-tokenizer` 크레이트. 별도 하이브리드 확장(`pg_hybrid_ko`)은 **폐기**(RRF를 B에 흡수). **v0.1 = 모노레포(Cargo workspace), 안정화 후 별도 리포 분리.** (§2.4)
- **2계층 근거(2026-06-19 갱신)**: `CREATE EXTENSION pg_glot_hybrid CASCADE`가 A+pg_textsearch+vector를 자동 생성 → 분리해도 설치 UX 동일(다운사이드 0), 순수 FTS 수요는 A만 설치. dense는 pgvector 표준 인터페이스에 작성 → pg_acorn/pg_cuvs/pgvectorscale/vchord와 자동 호환(dense seam 불필요). 별도 하이브리드 확장은 pg_aidb의 hybrid와 중복이라 폐기.
- **근거**: 토크나이저 계층(A)은 BM25/벡터와 무관하게 독립 가치(crown jewel) → 고정하면 재사용 손실. 생태계 정도(pgvector/pg_textsearch/textsearch_ko가 각각 독립)와 일치. 단 재사용성은 깨끗한 경계에서 나오지 별도 리포에서 나오는 게 아니며, API 불안정기의 멀티리포는 CI×3·버전 매트릭스·경계 조기고착 비용 → 모노레포 우선이 정석.
- **기각**: (지금부터 별도 리포 3개) 독립 릴리스/브랜딩 즉시 확보하나 코디네이션 비용을 불안정기에 선지불. (단일 확장 통합) 재사용 계층화 포기.

## 4. 구성요소 관계 — 엔진 vs 사전

```
[엔진 계보 — 같은 MeCab식 Viterbi의 독립 재구현들]
MeCab(taku910, C++, JP) → kuromoji(Java) → kuromoji-rs(Rust) → lindera(Rust)
mecab-ko(은전한닢/eunjeon, C++)  ← MeCab의 얇은 한국어 포크 (별도 가지)
hephaex/mecab-ko(Rust)          ← eunjeon mecab-ko의 독립 재구현 (Mario Cho, 1인)

[사전 — 한국어다움의 원천]
mecab-ko-dic(은전한닢, Apache-2.0)  ← 어휘 + 연결비용(matrix) + 미등록어 정의
```

- **lindera는 독립 Rust 엔진**이고 mecab-ko 코드에서 파생된 게 아니다(형제 관계). 셋(mecab-ko, lindera, Nori)이 공유하는 것은 **사전 mecab-ko-dic**.
- 한국어 토큰화 결과를 좌우하는 건 **사전**이므로 엔진이 달라도 출력이 매우 비슷(bake-off Jaccard 0.97).
- **Nori 패턴과 동일**: Nori = Lucene Java 엔진 + mecab-ko-dic. lindera = Rust 엔진 + mecab-ko-dic. mecab-ko = MeCab C++ + mecab-ko-dic.
- lindera는 `lindera-ko-dic` crate(은전한닢 mecab-ko-dic 재패키징, embed feature)로 사전을 임베드. 비트 단위로 eunjeon 원본과 동일하진 않을 수 있음(필요시 `lindera-dictionary-builder`로 원본 CSV에서 직접 빌드 가능).
- **eunjeon mecab-ko vs lindera 신뢰성**: 엔진 검증연한은 mecab-ko(MeCab) 우위, 활발한 유지보수·pgrx 임베드 적합성은 lindera 우위, 한국어 검색 품질은 동급. **우리 맥락(순수-Rust 자기완결 확장 + 검색 NDCG 기준 + 유지보수)에서는 lindera가 더 신뢰할 선택.** (hephaex와는 다름 — hephaex가 신뢰성 낮았던 것)

---

## 5. Bake-off — 방법론과 한계

산출물: `bakeoff/` (`REPORT.md`, `bakeoff_results.json`, `tokenizers.py`, `run_bakeoff.py`, `reproduce.sh`).

### 5.1 측정 기준
- 데이터셋: MIRACL-ko — `docs_ko_miracl.json`(~1,000 docs) + `queries_dev.json`(213 queries, qrels=`relevant_ids`). MDN-ko(800 docs, 토큰 일치율만 — qrels 없음).
- 베이스라인(golden): **python-mecab-ko** (연구가 실제 사용).

### 5.2 메트릭
- **NDCG@10** (이진 관련성, gain 1/0, DCG=Σ1/log2(rank+1) top-10, IDCG 정규화), +Recall@10, MRR. 213 질의 평균.
- 토큰 일치율: 시퀀스 exact / 집합 Jaccard / micro P·R. (exact는 과도하게 엄격 → Jaccard 주지표)

### 5.3 절차
각 토크나이저: ~1000 docs 토큰화(표층형, 품사필터 없음) → 순수 Python `SimpleBM25`(k1=1.2, b=0.75) 색인 → 질의 토큰화·스코어·정렬 → NDCG. **오직 토크나이저만 다르고 나머지 동일 = 통제 비교.**

### 5.4 결과
| 토크나이저 | NDCG@10 | R@10 / MRR | Jaccard(MIRACL/MDN) | 비고 |
|---|---|---|---|---|
| python-mecab-ko (baseline) | 0.3147 | 0.379/0.372 | 1.0/1.0 | golden |
| **lindera (ko-dic)** | **0.3287** | 0.381/0.399 | 0.965/0.976 | 회귀 없음(유의성 미검정), 최속, 임베드 |
| hephaex (sejong OFF) | 0.3066 | 0.363/0.366 | 0.873/0.902 | 동등 |
| hephaex (sejong ON) | 0.3108 | 0.365/0.378 | 0.734/0.731 | 토큰 크게 달라도 NDCG 동등 |

### 5.5 한계 (반드시 함께 읽을 것)
1. **상대 비교용이지 절대 품질 아님.** 실제 pg_textsearch BM25가 아니라 순수 Python BM25.
2. **연구 README의 BM25 NDCG 0.6385와 직접 비교 불가** — 그건 10K 코퍼스 + 실제 pg_textsearch + (아마)내용어 추출, 여기는 ~1K + Python BM25 + raw 표층형.
3. **델타(+0.014) 유의성 미검정**(bootstrap CI/p-value 없음) → 안전한 결론은 "lindera가 베이스라인보다 **나쁘지 않다**". **(Codex 리뷰)** 공개 v0.1 전 **lindera+실제 pg_textsearch로 Phase7 품질 재현을 release gate**로(통과 전 NDCG parity 홍보 금지); bootstrap CI·per-query win/loss·10K/100K 재측정 추가.
   - **2026-06-22 실측 + release gate 재정의** (`bench/eval_glot.py`, 실제 pg_glot+pg_textsearch, MIRACL dev): ko BM25 NDCG@10 = **0.606**(research MeCab 0.6385의 95%). 갭을 메우려 시도한 **POS 필터·english_stem 둘 다 measure-first로 기각**(POS는 ko 무효+ja recall만, english_stem은 ko ASCII 2.8%라 무변). 조사 결과 research `korean`은 **MeCab + korean_stem**(우리는 lindera + simple)이라 **토크나이저가 본질적으로 다름** → 정확한 NDCG parity는 비현실적. **release gate를 "research 0.6385 재현"이 아니라 "lindera baseline + 회귀 없음 + 측정 가능한 개선"으로 재정의.** 상세·ja/zh 수치는 `bench/RESULTS.md`.
   - **2026-06-22 후속 — POS accept-list로 사실상 parity 달성**: 위에서 "POS 필터 무효"라 한 판정이 **틀렸다**(잘못된 넓은 allowlist). MeCab same-harness로 갭을 재분해하니 **토크나이저 분절은 lindera ≈ MeCab**(무필터 0.606 vs 0.6038)이고 **진짜 레버는 정확한 POS accept-list**였다(`NNG,NNP,NNB,NNBC,NR,VV,VA,MM,MAG,XSN,XR,SH,SL`). lindera·MeCab 모두 같은 ko-dic POS 체계라 그대로 이식 → ko BM25 NDCG **0.634**(research 0.6385의 99.4%, MeCab 0.633 초과). **외부 의존·토크나이저 교체 없이 정체성(순수 Rust 임베드) 그대로 달성.** lindera baseline을 **0.634**로 상향. (가설→측정→재측정 자기수정의 사례)
4. 사전 3종 모두 mecab-ko-dic 계열이나 **컴파일 파이프라인 상이**(python: 원조 mecab `sys.dic`; hephaex: 자체 builder + `mecab-ko-dic-2.1.1-20180720`; lindera: 자체 임베드). 비트 동일성 미확정.
5. **확장 핵심 원칙**: 토큰 일치율 ≠ NDCG. 비트 동일성 불필요, 우리 testbench NDCG가 기준. 절대 production 품질은 확장 자체 testbench(pg_regress + ko_bench, 실제 pg_textsearch + 10K/100K)에서 재측정.

---

## 6. 라이선스 감사 (서브에이전트 검증 완료)

| 구성요소 | 라이선스 | 흡수/의존 | 비고 |
|---|---|---|---|
| 확장 본체(pg_glot_hybrid) | **PostgreSQL License**(또는 BSD 2-clause) | — | 공개+상업 OK |
| lindera | MIT | 의존(crate) | 순수 Rust, MeCab 미링크 |
| lindera-ko-dic (= mecab-ko-dic) | Apache-2.0 | 의존(crate, embed) | 외부 사전 설치 0 |
| pg_textsearch (Timescale/Tiger) | **PostgreSQL License** (TSL 아님 — 우려는 오해였음) | requires | text_config 필수 |
| pgvector | PostgreSQL License | requires | — |
| Kiwi (libkiwi, opt-in) | LGPL 2.1+ | 동적링크 FFI(opt-in) | 동적링크 유지 시 우리 소스 공개의무 없음. 정적링크 금지. 모델데이터(`sj.*`) 라이선스 불명확 → 동봉 말고 빌드 시 다운로드 |
| (참고) textsearch_ko | BSD 2-clause | 미사용 | MeCab 하드와이어라 채택 안 함 |
| (참고) MeCab 본체 | GPL/LGPL/BSD 트리플 | 미사용 | 기본 경로에 C++ 엔진 없음 |

**판정**: 공개+상업 배포에 라이선스 장애 없음. 기본 경로 GPL 0. 충족 조건: lindera/ko-dic NOTICE(MIT/Apache) 표기, Kiwi opt-in 시 LGPL 표기 + 동적링크 유지. (textsearch_ko를 안 쓰므로 MeCab GPL트리플·BSD 고지 부담 소멸.)

### 6.1 사실 정정 기록
- "pg_textsearch = TSL(사용제한)" 우려 → **실제 PostgreSQL License**.
- "BSD = 상업 금지" 오해 → **BSD/PostgreSQL License는 상업 사용 허용**(상업 금지는 CC BY-NC, TSL/BSL/SSPL 같은 것).
- "벤더링 금지" → 법적 금지 아님(전부 permissive). 다만 기본 경로는 lindera라 MeCab 벤더링 자체가 불필요해짐.

---

## 7. 데이터 — testbench 코퍼스

| 데이터 | 동봉 가부 | 라이선스 | 역할 |
|---|---|---|---|
| MIRACL-ko (corpus_10k/queries_dev/docs) | **가능** | Apache-2.0 + 위키 CC BY-SA(출처표시·share-alike) | Dense우세 + **scaling(1K/10K/100K)** |
| MDN-ko (`mdn/translated-content` files/ko) | **가능** | 산문 CC BY-SA 2.5, 코드 CC0/MIT | **BM25우세/도메인역전 품질벤치** |
| `data/miracl/mmarco_ko/*` | **불가** | MS MARCO 비상업 한정 | 제외 |
| EZIS (Oracle 매뉴얼 파생) | **불가** | Oracle 저작권 | 사내 비공개 테스트만 |

### 7.1 MDN-ko 볼륨 실측 (HEAD 2dcc9a1)
- 3,344 페이지 / 16MB / 1,106만 문자. BM25우세 핵심섹션(js·api·css·html·http·glossary)=2,845페이지(85%).
- 청킹 시 passage 1.8만~4.3만. **10K 가능, 100K scaling 불가** → MDN-ko=도메인역전 전용, scaling=MIRACL-corpus-ko 분담.

### 7.2 2-tier 데이터 전략
- Tier 1(레포 동봉, 수백~1K): pg_regress 정확성 + 즉시 스모크 `ko_bench`. 작게 유지.
- Tier 2(다운로드형, 커밋 X): NDCG 품질 + latency/scaling. `make bench-data`로 원본에서 fetch(레포 비대화·재배포 용량 회피).
- 질의·qrels: MIRACL은 실측 qrels, MDN은 Claude 합성(연구와 동일 방식).

### 7.3 제외 파일(이식 시 차단)
`data/EZIS_Oracle_Manual.pdf`, `data/ezis/*`, `data/miracl/mmarco_ko/*`, `data/kiwi_models/*`, Kiwi 모델데이터.

---

## 8. 스코프 & 비목표

- **Dense 임베딩(BGE-M3 등) 생성은 범위 밖.** v0.1은 사용자가 벡터 컬럼을 제공하고, 우리는 그 위에서 pgvector 검색 + RRF만 수행(연구도 retrieval-only로 측정, 임베딩 추론 ~200ms 제외).
- JVM 분석기(Okt/KKMA/KOMORAN) 미지원.
- 관리형 PostgreSQL(RDS/Cloud SQL)은 커스텀 C/Rust 확장 로드를 막으므로 수혜 대상은 self-hosted/Docker.

---

## 9. 미결 사항 / 다음 단계

1. **스캐폴딩 환경 점검**: `cargo-pgrx`·`pg_config`·대상 PG 버전 확인. (← 현재 여기서 멈춤)
2. **모노레포 스캐폴딩**: Cargo workspace + `crates/glot-tokenizer` + `extensions/{pg_glot, pg_glot_hybrid}`. control 파일(B: `requires = 'pg_glot, pg_textsearch, vector'`), pg_regress 뼈대. 별도 하이브리드 확장 없음(RRF는 B에 포함). (§2.4)
3. **analyzer seam (Layer A)**: `glot-tokenizer` 크레이트 — trait(`tokenize(text) -> Vec<Token>`) + lindera 구현(+opt-in kiwi feature). `pg_glot` 확장 — 커스텀 TS parser → `korean` config 등록.
   **1차 마일스톤(Codex 리뷰 반영, 단순 골든으로 부족)**: ① `ts_parse`/`ts_debug`/`to_tsvector` 골든 → ② 파서 ABI 메모리/panic/에러경로 테스트 → ③ pg_textsearch BM25 인덱스 `text_config='korean'` 라운드트립 → ④ index-time/query-time 일관성 검증 → ⑤ REINDEX/업그레이드/사전버전 동작.
4. **하이브리드 API**: Phase7 RRF SQL 이식 → `glot.hybrid()` 시그니처 확정.
5. **testbench**: `bakeoff/` 평가코드를 `ko_bench`로 발전.
6. (선택) vibrato를 bake-off에 추가해 후보공간 airtight.
7. (선택) Kiwi 모델데이터 재배포 가부 bab2min에게 서면 확인.

---

## 10. 주요 기술 리스크

(등급은 2026-06-19 Codex 설계 리뷰 반영해 상향 조정됨.)

| 리스크 | 등급 | 대응 |
|---|---|---|
| 커스텀 TS parser(ABI 직면) 정확성 — palloc 소유권, panic-across-FFI, one-shot, lextype | **높음** | **별도 C-ABI 파서 하니스 선행**(빈입력·장문·멀티바이트offset·잘못된인코딩·혼합토큰·에러후재호출·tx abort·memory-context cleanup 테스트). 모든 FFI 경계 `catch_unwind`→`ereport(ERROR)`. |
| 토큰 메모리 소유권 | **높음** | 파서 상태=PG-allocated 불투명 구조체, 토큰은 활성 memory context에 `palloc` **복사**, context reset로 해제. Rust 차용 금지. |
| one-shot 토큰화 대용량 메모리 스파이크 | **높음** | 1KB~10MB·100K행 색인 벤치. lindera가 허용하면 스트리밍 검토. |
| `Tokenizer &mut self` + 백엔드 전역 초기화 | **높음** | 공유 가변상태 회피 — 불변 사전/analyzer 데이터만 전역, per-call 경량 상태 인스턴스화. Send/Sync 래퍼/컴파일 assert. |
| **pg_textsearch 의존(제품 리스크)** — 신생 확장, AM/DDL/연산자/VACUUM/복구/업그레이드 변동 | **높음** | 버전 범위 핀(docs+CI), 정확 릴리스 통합테스트, **얇은 BM25 provider trait**로 파손 국소화, 업그레이드=호환성 이벤트로 취급. |
| index-time vs query-time 파서 의미 동일성 | **높음(검증 전)** | 1차 마일스톤에 BM25 라운드트립 포함(create→query→update/delete→requery), `ts_debug`/`to_tsvector` 대조. |
| 토크나이저/인덱스 재현성(사전=인덱스 정의의 일부) | **높음** | `glot.dictionary_version()` 노출 + 사전 빌드 해시, 사전 변경 시 REINDEX 문서/경고, 업그레이드 스크립트. |
| lextype 토큰 클래스 설계 미흡 → 랭킹 손상 | 중간 | 한국어 lexeme·Latin·숫자·혼합alnum·URL 등 의도적 분류, `ts_debug` 골든, ADD MAPPING 안정·문서화. |
| `glot.hybrid(rel,key_col)` 동적SQL + SECURITY DEFINER 풋건 | 중간 | definer 회피, `regclass`/`format('%I')`/`SET search_path=pg_catalog,pg_temp`. `glot.rrf` 주력·어댑터 experimental. |
| MVCC/인덱스 일관성(두 leg 가시성 차이) | 중간 | 동시 insert/update/delete 테스트(RC/RR), 스냅샷 일관성 문서화. |
| WAL/크래시 안전(의존성 영역이나 수용테스트 필요) | 중간 | 재시작·abort·실패한 색인빌드·REINDEX·drop/create cascade 스모크 CI. |
| 2-확장 분리 운영비용("다운사이드 0"은 과표현) | 중간 | control 2개·업그레이드 체인·의존순서·`korean` config 이름충돌·부분설치 문서화, 객체 네임스페이스 주의. |
| lindera 토큰화가 연구 베이스라인과 비트 동일 아님 | 낮음 | 안전 주장="회귀 없음"; 절대품질은 testbench 재측정 |
| Kiwi(opt-in) libkiwi 정적링크 시 LGPL 추가의무 | 낮음 | 동적링크 유지 강제 |

---

## 11. 검증 로그 (무엇을, 어떻게 확인했나)

- 라이선스 감사 #1(서브에이전트): textsearch_ko=BSD, pg_textsearch=PostgreSQL License(TSL 아님), pgvector=PostgreSQL License, MeCab=GPL/LGPL/BSD, mecab-ko-dic=Apache. 로컬 `vendor/` LICENSE + GitHub 원격 확인.
- 라이선스 감사 #2(서브에이전트): Kiwi=LGPL 2.1+(LICENSE 파일), 모델데이터 불명확, MIRACL 정통=Apache+CC-BY-SA, mmarco_ko=MS MARCO 비상업, EZIS=Oracle 저작권.
- MDN 라이선스(WebFetch): 산문 CC BY-SA 2.5, 코드 CC0/MIT.
- MDN 볼륨(서브에이전트): blobless clone 전수 카운트 3,344 페이지.
- hephaex/mecab-ko(서브에이전트): 순수 Rust 재구현, Apache/MIT, eunjeon의 독립 재구현(공식 후속 아님, Mario Cho 1인), 동등성 미검증·성숙도 리스크.
- lindera(WebFetch): Rust 90.8%, kuromoji-rs fork, C/C++ 의존 없음, MIT, ko-dic embed/path.
- pg_textsearch 입력 API(소스/README): text+text_config(regconfig) 필수, tsvector 입력 경로 없음.
- textsearch_ko 결합(소스): `#include <mecab.h>` 하드와이어, `korean` TS parser가 libmecab 호출.
- 연구 분석기 배선(소스): kiwi/okt는 Python BM25Embedder/sparse 경로, textsearch_ko는 MeCab 전용.
- Bake-off(실측): lindera NDCG 0.3287 ≥ baseline 0.3147.
- Codex(o-series) 설계 리뷰(2026-06-19): TS parser·pg_textsearch 리스크 높음으로 상향, 1차 마일스톤 확장(BM25 라운드트립·index/query 일관성·REINDEX), bake-off 주장 수위 하향 + Phase7 재현 release gate, 사전 버전 재현성·MVCC·보안(SECURITY DEFINER) 갭 추가, Bayesian 보류. §10/§9/§5.5/D3/D6 반영.

---

## 부록 A. 한국어 형태소 분석기 후보 지형
| 후보 | 유형 | ko-dic 준비도 | 성숙도 | 판정 |
|---|---|---|---|---|
| lindera | 순수 Rust(kuromoji계) | 임베드 즉시 | 130만+ DL | **채택** |
| vibrato | 순수 Rust MeCab 재구현 | 직접 컴파일 | 견실 | 미평가 대안 |
| hephaex/mecab-ko | 순수 Rust 재구현 | 자체빌드 | 1인·DL 344 | 예비 |
| eunjeon mecab-ko | C++ MeCab 포크 | 원본 | 휴면(2018) | 정전이나 C++/휴면 |
| sudachi.rs / goya | 순수 Rust | 일본어 전용 | 성숙(sudachi) | 한국어 불가 |
| Kiwi | C++ (LGPL) | 자체모델 | 품질1위 | **opt-in 2번째** |
| Khaiii/KOMORAN/KKMA | C++/Java | 무거움 | — | pgrx 부적합 |
