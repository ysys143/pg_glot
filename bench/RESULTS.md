# glot 검색 품질 — BM25 NDCG (실제 pg_glot + pg_textsearch)

`bench/eval_glot.py`로 측정. 각 언어 config(`public.korean`/`japanese`/`chinese`)로
실제 pg_textsearch BM25 인덱스를 만들고 검색 → NDCG@10 / Recall@10. **bakeoff/(순수
Python BM25, INDICATIVE)와 달리 실제 엔진 측정**이지만, 아래 한계를 반드시 함께 읽을 것.

## 결과 (MIRACL dev, BM25-only)

| lang | config | docs | queries | NDCG@10 | Recall@10 | empty |
|---|---|---|---|---|---|---|
| ko | `public.korean`   | 10000 | 213 | **0.6344** | 0.7983 | 0 |
| ja | `public.japanese` |  8066 | 860 | **0.5397** | 0.7298 | 0 |
| zh | `public.chinese`  |  3786 | 393 | **0.4577** | 0.6461 | 0 |

ko는 MeCab accept-list POS 필터 + english_stem 반영. **research MeCab BM25(0.6385)의 99.4%**.

## 핵심 발견: 갭의 레버는 토크나이저가 아니라 POS 필터였다 (measure-first)

research MeCab(0.6385) 대비 lindera 무필터(0.606)의 갭을 끝까지 측정으로 분해했다.
같은 10K 하니스에서 토크나이저/POS 조건을 분리:

| 구성 | NDCG@10 | 해석 |
|---|---|---|
| lindera 무필터 | 0.606 | 출발점 |
| MeCab 전체 morphs(무필터) | 0.6038 | **토크나이저만 바꾸면 ≈ lindera** |
| MeCab + accept-list(POS 필터) | 0.6328 | POS 필터가 +2.9%p |
| **lindera + accept-list**(simple) | **0.6362** | **lindera가 MeCab 초과** |
| **lindera + accept-list + english_stem**(production korean config) | **0.6344** | 출하본 |
| research MeCab | 0.6385 | 목표(99.4% 도달) |

- **토크나이저 분절은 lindera ≈ MeCab**(무필터 0.606 vs 0.6038). 갭은 토크나이저가 아니었다.
- **진짜 레버는 정확한 POS accept-list**(`NNG,NNP,NNB,NNBC,NR,VV,VA,MM,MAG,XSN,XR,SH,SL`).
  lindera·MeCab 모두 같은 ko-dic POS 체계라, lindera에 그대로 이식해 **+2.8%p**(0.606→0.634).
- **MeCab/Kiwi opt-in 불필요** — lindera(순수 Rust·임베드·최속)로 MeCab 품질을 **정체성 그대로** 달성.
- A1에서 "POS 필터 무효(−0.75%p)"라 한 건 잘못된 **넓은 allowlist**(`N*/V*/MA*`) 탓. 정확한
  accept-list가 답이었다. (가설→측정→재측정으로 자기수정)

## ja/zh
- **ja**: 助詞/助動詞/記号 등 기능어 제외(denylist) → recall +1.6%p. NDCG 0.5397.
- **zh**: cc-cedict가 POS 미제공('*') → 필터 불가, 전부 색인. NDCG 0.4577.
- ko>ja>zh 순서는 MIRACL 언어별 BM25 난이도와 일치(정상).

## release gate (DESIGN §5.5)

lindera baseline = **ko BM25 NDCG 0.634**(research MeCab 0.6385의 99.4%). 토크나이저 교체나
외부 의존 없이 사실상 동급에 도달했다. 향후 분석기/사전/정규화 변경은 이 baseline 대비
**상대 측정**으로 판정. dense/RRF는 텍스트 검색(BM25) 위에 얹는 층이라 별도.

## 한계 (반드시 함께 읽을 것)

1. **dev passages subset** — MIRACL dev의 positive+negative passages(~수천~1만)이며 full
   corpus가 아니다. 공식 리더보드와 **직접 비교 불가**.
2. **BM25만** — dense/RRF는 임베딩 확보 후. 여기는 lexical 단독.
3. **유의성 미검정** — bootstrap CI/per-query 없음(±0.5%p는 noise일 수 있음).
4. **MeCab 측정은 측정 전용** — `bench/mecab_pretokenize.py`는 갭 분해용이며 제품(순수 Rust)에
   들이지 않는다. 결론은 "lindera로 충분"이다.

## 재현

```bash
python3 bench/fetch_miracl.py                         # → bench/data/{ja,zh}/ (ko는 research 데이터)
DSN="host=localhost port=5433 user=postgres password=pw dbname=postgres"
python3 bench/eval_glot.py --lang korean   --corpus bench/data/ko/corpus.json --queries bench/data/ko/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang japanese --corpus bench/data/ja/corpus.json --queries bench/data/ja/queries.json --dsn "$DSN"
python3 bench/eval_glot.py --lang chinese  --corpus bench/data/zh/corpus.json --queries bench/data/zh/queries.json --dsn "$DSN"
# 갭 분해(측정 전용): bench/mecab_pretokenize.py, bench/eval_bm25_grid.py
```

데이터는 라이선스/용량상 커밋하지 않는다(재생성). MIRACL = Apache-2.0 + Wikipedia
CC-BY-SA. docs/DESIGN.md §7.
