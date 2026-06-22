# glot 검색 품질 — BM25 NDCG (실제 pg_glot + pg_textsearch)

`bench/eval_glot.py`로 측정. 각 언어 config(`public.korean`/`japanese`/`chinese`)로
실제 pg_textsearch BM25 인덱스를 만들고 검색 → NDCG@10 / Recall@10. **bakeoff/(순수
Python BM25, INDICATIVE)와 달리 실제 엔진 측정**이지만, 아래 한계를 반드시 함께 읽을 것.

## 결과 (MIRACL dev)

| lang | config | docs | queries | NDCG@10 | Recall@10 | empty |
|---|---|---|---|---|---|---|
| ko | `public.korean`   | 10000 | 213 | **0.6058** | 0.7672 | 0 |
| ja | `public.japanese` |  8066 | 860 | **0.5328** | 0.7134 | 0 |
| zh | `public.chinese`  |  3786 | 393 | **0.4585** | 0.6461 | 0 |

## 해석

- **empty_results 0 (전 언어)** — 모든 query가 BM25 인덱스 매칭. 엔진 + config + planner
  hook(리터럴 ORDER BY+LIMIT)이 ko/ja/zh 모두 정상 동작함을 확인.
- **ko vs research Phase7 BM25 (0.6385)** — 0.6058, **−5%**. 갭의 주원인 추정 = **POS
  필터 부재**(우리는 표층형 전체를 색인 → 조사/어미가 노이즈; research는 내용어 추출
  가능성). 품질 정교화(josa/eomi POS 필터, ASCII→english_stem)의 개선 여지.
- **ja/zh가 ko보다 낮은 것은 정상** — MIRACL 언어별 BM25 난이도 자체가 ja<ko, zh<ja
  (공식 baseline 경향과 일치). 동일 POS 갭 + 언어 난이도이며, ballpark는 MIRACL BM25
  baseline 수준 → ja/zh config가 **합리적으로 동작**.

## 한계 (반드시 함께 읽을 것)

1. **dev passages subset** — MIRACL dev의 positive+negative passages(~수천~1만)이며 MIRACL
   **full corpus(ja ~700만/zh ~500만)가 아니다**. 따라서 MIRACL 공식 리더보드 수치와
   **직접 비교 불가**. 상대·정성 지표로만 읽을 것.
2. **BM25만** — dense/RRF(`glot.hybrid`) 풀스택은 임베딩(BGE-M3 등) 확보 후 측정. 여기 수치는
   lexical leg 단독.
3. **POS 필터 없음** — 기능어(조사/어미/的/了 등)까지 색인. 정교화 전 baseline.
4. **유의성 미검정** — bootstrap CI/per-query 분석 없음.
5. **release gate(ko)** — research parity는 **아직 미달**(0.6058 < 0.6385). DESIGN §5.5의
   "NDCG parity 홍보 금지"는 **유지**한다. 정교화 후 재측정이 다음 수순.

## 재현

```bash
# 데이터: ko는 research 레포(MIRACL-ko) 재사용; ja/zh는 MIRACL dev에서 추출.
python3 bench/fetch_miracl.py                         # → bench/data/{ja,zh}/

# 평가(Docker 컨테이너 = 자기완결 pg_glot 스택, port 5433 노출 가정)
DSN="host=localhost port=5433 user=postgres password=pw dbname=postgres"
python3 bench/eval_glot.py --lang korean   --corpus bench/data/ko/corpus.json --queries bench/data/ko/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang japanese --corpus bench/data/ja/corpus.json --queries bench/data/ja/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang chinese  --corpus bench/data/zh/corpus.json --queries bench/data/zh/queries.json --dsn "$DSN"
```

데이터는 라이선스/용량상 커밋하지 않는다(재생성). MIRACL = Apache-2.0 + Wikipedia
CC-BY-SA. docs/DESIGN.md §7.
