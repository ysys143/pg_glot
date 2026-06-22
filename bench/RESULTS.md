# glot 검색 품질 — BM25 NDCG (실제 pg_glot + pg_textsearch)

`bench/eval_glot.py`로 측정. 각 언어 config(`public.korean`/`japanese`/`chinese`)로
실제 pg_textsearch BM25 인덱스를 만들고 검색 → NDCG@10 / Recall@10. **bakeoff/(순수
Python BM25, INDICATIVE)와 달리 실제 엔진 측정**이지만, 아래 한계를 반드시 함께 읽을 것.

## 결과 (MIRACL dev, BM25-only, POS(ja)+english_stem+lextype 정교화 반영)

| lang | config | docs | queries | NDCG@10 | Recall@10 | empty |
|---|---|---|---|---|---|---|
| ko | `public.korean`   | 10000 | 213 | **0.6057** | 0.7672 | 0 |
| ja | `public.japanese` |  8066 | 860 | **0.5397** | 0.7298 | 0 |
| zh | `public.chinese`  |  3786 | 393 | **0.4577** | 0.6461 | 0 |

**이 수치를 glot의 lindera baseline으로 삼는다**(release gate, 아래 §재정의).

## A1: 품질 정교화 실험 (measure-first — 가설 세우고 측정하고 기각)

research Phase7 BM25(MeCab 기반, NDCG 0.6385)와의 ko 갭을 메우려는 가설들을 동일
데이터로 측정했다. **조사 결과 research `korean` config는 우리와 토크나이저부터 다르다**:
MeCab + `korean_stem`(한국어 어간) + ASCII `english_stem`. 우리는 lindera ko-dic + `simple`.

| 가설 | 변경 | ko Δ NDCG | 판정 |
|---|---|---|---|
| POS 필터(기능어 색인 제외) | `is_content_pos` | 0.6058 → 0.5983 (**−0.75%p**) | **기각**(IDF와 중복, 무이득) |
| english_stem(ASCII 스테밍) | asciiword→english_stem | 0.6058 → 0.6057 (**무변**) | **기각**(ko corpus ASCII 2.8%) |

- **POS 필터는 ko에서 무효**(오히려 미세 하락) — BM25 IDF가 흔한 기능어를 이미 누른다.
  단 **ja는 recall +1.6%p**(助詞/助動詞 제거가 도움) → ja만 유지. zh는 cc-cedict가 POS
  미제공이라 적용 불가.
- **english_stem은 정상 작동**(`Running servers`→`run server`)하나 ko corpus의 ASCII 비율이
  **2.8%**라 NDCG 영향이 없다. 정합성(올바른 ASCII 처리)·혼합 텍스트 도메인엔 유효해 유지.

### 결론: 갭의 주원인은 토크나이저
POS·스테밍 둘 다 ko 갭을 못 메웠다. 남은 차이는 **① 토크나이저(MeCab ≠ lindera,
bake-off Jaccard 0.97 = 3% 토큰 불일치) ② korean_stem(한국어 어간 정규화)** 이며, 둘 다
우리 정체성인 **lindera(순수 Rust 임베드)로는 본질적으로 재현 어렵다**. MeCab과의 정확한
NDCG parity는 비현실적 목표다.

## release gate 재정의 (DESIGN §5.5)

"research 0.6385 재현"이 아니라 **"lindera baseline(ko 0.606) + 회귀 없음 + 측정 가능한
개선"** 으로 재정의한다. 향후 분석기/사전/정규화 변경은 이 baseline 대비 **상대 측정**으로
판정한다(절대 parity는 토크나이저가 다른 한 불가). Kiwi opt-in(품질 1위 분석기)이 도입되면
별도 baseline으로 비교한다.

## 한계 (반드시 함께 읽을 것)

1. **dev passages subset** — MIRACL dev의 positive+negative passages(~수천~1만)이며 MIRACL
   **full corpus(ja ~700만/zh ~500만)가 아니다**. 공식 리더보드와 **직접 비교 불가**.
2. **BM25만** — dense/RRF(`glot.hybrid`)는 임베딩(BGE-M3 등) 확보 후 측정. 여기는 lexical 단독.
3. **유의성 미검정** — bootstrap CI/per-query 분석 없음(±0.5%p 수준 차이는 noise일 수 있음).
4. **POS 필터는 ja만 활성**, ko 비활성(측정 근거), zh는 POS 미제공.

## 재현

```bash
python3 bench/fetch_miracl.py                         # → bench/data/{ja,zh}/ (ko는 research 데이터)
DSN="host=localhost port=5433 user=postgres password=pw dbname=postgres"
python3 bench/eval_glot.py --lang korean   --corpus bench/data/ko/corpus.json --queries bench/data/ko/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang japanese --corpus bench/data/ja/corpus.json --queries bench/data/ja/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang chinese  --corpus bench/data/zh/corpus.json --queries bench/data/zh/queries.json --dsn "$DSN"
```

데이터는 라이선스/용량상 커밋하지 않는다(재생성). MIRACL = Apache-2.0 + Wikipedia
CC-BY-SA. docs/DESIGN.md §7.
