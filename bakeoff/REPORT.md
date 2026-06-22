# 한국어 형태소 분석기 Bake-off 결과

**목적**: `pg_textsearch_ko` 확장의 한국어 형태소 분석 백엔드를 데이터로 결정하기 위해,
3개 토크나이저를 같은 코퍼스에 돌려 ①토큰 일치율(vs 베이스라인) ②BM25 NDCG를 측정한다.

- 실행일: 2026-06-19
- 측정 머신: macOS (darwin 25.5.0, Apple Silicon), Rust/cargo 1.95.0
- 베이스라인(golden): **python-mecab-ko** (`from mecab import MeCab`, `.morphs()`)
- 평가 하니스: `textsearch/experiments/common/bm25_module.py` 및
  `phase1_analyzer_comparison.py`의 BM25/NDCG 로직을 재사용·확장
  (`run_bakeoff.py`의 `SimpleBM25` + `ndcg_at_k`).

---

## 1. 비교 표

| 토크나이저 | 빌드상태 | 토큰일치율 MIRACL (exact / Jaccard) | 토큰일치율 MDN (exact / Jaccard) | NDCG@10 (MIRACL) | 비고 |
|---|---|---|---|---|---|
| **python-mecab-ko** (baseline) | OK (golden) | 1.000 / 1.000 | 1.000 / 1.000 | **0.3147** | 연구가 실제 쓴 기준. R@10=0.3786, MRR=0.3719 |
| **lindera (ko-dic)** | OK | **0.353 / 0.965** | **0.363 / 0.976** | **0.3287** | **베이스라인 초과**. R@10=0.3811, MRR=0.3986. crates.io `embed-ko-dic` |
| hephaex (sejong OFF) | OK | 0.036 / 0.873 | 0.100 / 0.902 | 0.3066 | 거의 동등(-0.008). 사전 자체 빌드 필요 |
| hephaex (sejong ON) | OK | 0.006 / 0.734 | 0.001 / 0.731 | 0.3108 | 토큰형 크게 다름에도 NDCG 동등. wakati 미지원(TSV+EOS 출력) |

부가 지표 (token-set micro / 토큰화 속도):

| 토크나이저 | micro precision / recall (MIRACL) | tokenize 1000 docs (s) |
|---|---|---|
| python-mecab-ko | 1.00 / 1.00 | 0.46 |
| lindera (ko-dic) | 0.981 / 0.984 | 0.14 (최속) |
| hephaex (sejong OFF) | 0.927 / 0.935 | 0.59 |
| hephaex (sejong ON) | 0.832 / 0.852 | 0.88 |

- 데이터: MIRACL-ko **1,000 docs / 213 queries**(qrels = `queries_dev.json`의 `relevant_ids`),
  MDN 한국어 **800 docs**(프론트매터·KumaScript·코드블록·마크다운 마크업 제거 후 본문).
- 토큰 일치율 샘플: 코퍼스당 앞 **800개** 문서.
- 토큰 정의: **표층형(surface) 시퀀스**, 품사 필터 없음(베이스라인 `.morphs()`와 동일 조건).
  BM25는 이 표층형 토큰을 그대로 색인(3종 동일 적용).

---

## 2. 사전 정합성 (중요 — 비교 오염 방지)

공정 비교의 핵심은 "세 토크나이저가 같은 사전을 쓰는가"였다.

- **python-mecab-ko**: site-packages의 컴파일된 사전
  (`.venv/.../mecab_ko_dic/dictionary/`, `sys.dic` 80MB + `matrix.bin` 20MB). 이는
  **mecab-ko-dic 계열**(은전한닢)이며 원조 MeCab(C)의 double-array 포맷이다.
- **hephaex**: 자체 포맷(yada double-array + rkyv)을 사용한다. python 쪽 `sys.dic`/`matrix.bin`을
  `MECAB_DICDIR`/`-d`로 그대로 물려봤으나 **포맷 비호환**으로 토큰화가 깨졌다
  (예: "한국어"→"한/국어"). 따라서 **동일 계열 원본
  `mecab-ko-dic-2.1.1-20180720`을 hephaex `mecab-ko-dict-builder`로 재컴파일**하여 사용했다
  (816,283 엔트리). 이후 "한국어 형태소 분석 테스트 입니다"로 정상 동작 확인.
- **lindera**: `embed-ko-dic` feature로 **ko-dic이 바이너리에 임베딩**되어 있다
  (`-d embedded://ko-dic`). lindera가 임베딩하는 ko-dic도 mecab-ko-dic 계열이다.

→ **세 토크나이저 모두 mecab-ko-dic 계열 사전**을 사용하므로 사전 출처는 사실상 동일하다.
단, **컴파일 파이프라인이 셋 다 다르다**(python: 원조 mecab; hephaex: 자체 builder;
lindera: 자체 빌드 + 임베딩). 사전 **버전**은 python-mecab-ko가 명시 버전을 노출하지 않아
정확히 확정하지 못했으나, hephaex에는 명시적으로 `2.1.1-20180720`을 빌드해 물렸다.
토큰 차이의 상당 부분은 사전 내용보다 **Viterbi 비용/연결행렬 컴파일·미등록어(unknown) 처리·
복합어 분해 정책의 구현 차이**에서 비롯된 것으로 보인다(아래 해석 참조).

---

## 3. 해석 — 토큰 일치율과 NDCG는 분리해서 읽어야 한다

- **시퀀스 정확일치율(exact)는 신뢰성 지표로 부적합**하다. lindera조차 35%, hephaex는 3.6%다.
  이는 한 글자만 경계가 달라도 0이 되는 매우 엄격한 척도이기 때문이다.
  더 의미 있는 것은 **Jaccard / micro precision·recall**이다.
- **lindera**: Jaccard 0.965(MIRACL)·0.976(MDN), micro P/R ~0.98로 베이스라인과 사실상 동일한
  토큰 집합을 만든다. 그 결과 **NDCG@10이 베이스라인을 오히려 초과(0.3287 vs 0.3147)**하고
  R@10·MRR도 모두 더 높다. 검색 품질·토큰 충실도·속도 3박자가 모두 베이스라인 이상.
- **hephaex (sejong OFF)**: Jaccard 0.87~0.90으로 토큰이 더 흩어지지만,
  **NDCG는 0.3066으로 베이스라인과 사실상 동등(-0.008)**. "토큰이 좀 달라도 검색엔 충분"의 전형.
- **hephaex (sejong ON)**: 세종 코퍼스 호환 출력으로 토큰 형태가 크게 달라져
  Jaccard 0.73까지 떨어지지만, **NDCG는 0.3108로 OFF보다 오히려 약간 높다.**
  즉 BM25 검색 품질은 토큰 표면형의 일치보다 "변별력 있는 내용어가 보존되는가"에 더 민감하다.
  단, sejong 모드는 CLI에서 `-O wakati`를 무시하고 TSV+EOS로만 출력하는 동작 차이가 있어
  통합 시 출력 파서를 따로 둬야 한다(본 측정에서는 EOS 파싱으로 처리).

**결론적 해석**: NDCG 관점에서 세 후보(및 hephaex 두 모드)는 모두 베이스라인과 동급이며,
**lindera는 베이스라인을 능가**한다.

---

## 4. 권고

**Rust 후보 중 lindera를 1순위로 채택 권고.**

근거:
1. **NDCG@10이 베이스라인 이상**(0.3287 > 0.3147)이며 R@10·MRR도 모두 우위 — 제품 검색 품질이
   기존 연구 기준선보다 떨어지지 않고 오히려 개선될 여지.
2. **토큰 충실도 최고**(Jaccard 0.965~0.976) — 기존 색인/쿼리 동작과의 호환성 위험이 가장 낮음.
3. **성숙도·배포 용이성 우위** — crates.io 정식 배포(`lindera-cli 3.0.7`), ko-dic을 바이너리에
   임베딩(`embedded://ko-dic`)하여 **외부 사전 설치·경로 설정 불필요**. PostgreSQL 확장에서
   링크/번들하기 가장 단순.
4. **토큰화 속도도 최속**(1000 docs 0.14s).

**hephaex는 2순위(예비)**: 순수 Rust·NDCG 동등이라는 장점은 있으나,
(a) 사전을 자체 builder로 별도 컴파일해야 하고(배포 복잡), (b) 기본 `Tokenizer::new()`가
미니 사전으로 폴백해 `MECAB_DICDIR` 설정이 필수이며, (c) sejong/wakati 출력 일관성 등
운영상 손이 더 간다.

**C/PGXS + 검증된 mecab 폴백은 불필요**해 보인다 — Rust 후보(lindera)가 베이스라인 이상의
NDCG를 실측으로 보였기 때문이다. 다만 확장 통합 단계에서 lindera를 Rust로 직접 링크할지
(`pgrx` 기반), 아니면 IPC/CLI로 호출할지는 별도 아키텍처 결정 사항이다.

---

## 5. 제약·한계 (숨기지 않음)

- **사전 버전 차이 미완전 확정**: python-mecab-ko가 노출하는 사전 버전 문자열을 확인하지 못했다.
  hephaex에는 `2.1.1-20180720`을 명시 빌드했고 lindera는 자체 임베딩 ko-dic을 쓴다. 셋 다
  mecab-ko-dic 계열이나 **빌드 시점/버전이 비트 단위로 동일하다고 보장할 수는 없다.**
- **sejong 모드 출력 형식 차이**: hephaex `--sejong`은 `-O wakati`를 무시하고 TSV+EOS로 출력한다
  (CLI 동작). 본 측정은 EOS 기반 파서로 보정했다. 단건 호출은 정상이나 배치 통합 시 주의.
- **exact 시퀀스 일치율은 과도하게 엄격**하여 단독 판단 근거로 쓰지 않았다(Jaccard/micro 우선).
- **MDN은 qrels이 없어 NDCG 미측정**(요구사항대로 토큰 일치율만).
- BM25는 순수 Python 구현(하니스 차용)으로 측정했으며, PostgreSQL 내장 `bm25_ranking()`
  SQL 경로와 수치가 1:1 동일하다는 보장은 아니다(상대 비교 목적엔 충분).

---

## 6. 정확한 재현 명령

```bash
# 전체 자동 재현
bash /Users/jaesolshin/Documents/GitHub/pg_textsearch_ko/bakeoff/reproduce.sh

# 또는 단계별:
VENV_PY=/Users/jaesolshin/Documents/GitHub/textsearch/.venv/bin/python

# 1) lindera-cli (ko-dic 임베딩)  — feature 이름은 embed-ko-dic
cargo install lindera-cli --no-default-features --features=embed-ko-dic
echo "한국어 형태소 분석 테스트입니다" | ~/.cargo/bin/lindera tokenize -d embedded://ko-dic -o wakati

# 2) hephaex CLI 빌드
cd /tmp/hephaex_mecab_ko/rust && cargo build --release -p mecab-ko-cli

# 3) hephaex 용 ko-dic 바이너리 사전 빌드 (python sys.dic 비호환 -> 원본 재컴파일)
cd /tmp/hephaex_mecab_ko/data
curl -sLO https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz
tar xzf mecab-ko-dic-2.1.1-20180720.tar.gz
cd /tmp/hephaex_mecab_ko/rust
cargo run --release -p mecab-ko-dict-builder -- build \
    --input /tmp/hephaex_mecab_ko/data/mecab-ko-dic-2.1.1-20180720 \
    --output /tmp/hephaex_mecab_ko/data/dict-output

# 4) bake-off 측정
cd /Users/jaesolshin/Documents/GitHub/pg_textsearch_ko/bakeoff
$VENV_PY run_bakeoff.py
```

## 산출물

- `bakeoff/tokenizers.py` — 3종 토크나이저 어댑터(동일 인터페이스 text -> List[str])
- `bakeoff/run_bakeoff.py` — M1/M2 측정 + JSON 저장 (textsearch 하니스 재사용)
- `bakeoff/bakeoff_results.json` — 실측 결과 (위 표의 원본 수치)
- `bakeoff/reproduce.sh` — 전체 재현 스크립트
- `bakeoff/REPORT.md` — 본 보고서
