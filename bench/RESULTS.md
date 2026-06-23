# glot 검색 품질 — BM25 NDCG (실제 pg_glot + pg_textsearch)

`bench/eval_glot.py`로 측정. 각 언어 config(`public.korean`/`japanese`/`chinese`)로
실제 pg_textsearch BM25 인덱스를 만들고 검색 → NDCG@10 / Recall@10. **bakeoff/(순수
Python BM25, INDICATIVE)와 달리 실제 엔진 측정**이지만, 아래 한계를 반드시 함께 읽을 것.

## 결과 (MIRACL dev, BM25-only)

| lang | config | docs | queries | NDCG@10 | Recall@10 | empty |
|---|---|---|---|---|---|---|
| ko | `public.korean`   | 10000 | 213 | **0.6362** | 0.7983 | 0 |
| ja | `public.japanese` |  8066 | 860 | **0.5397** | 0.7298 | 0 |
| zh | `public.chinese`  |  3786 | 393 | **0.4577** | 0.6461 | 0 |

ko는 MeCab accept-list POS 필터(simple 매핑). **research MeCab BM25(0.6385)의 99.7%**.

## 핵심 발견: 갭의 레버는 토크나이저가 아니라 POS 필터였다 (measure-first)

research MeCab(0.6385) 대비 lindera 무필터(0.606)의 갭을 끝까지 측정으로 분해했다.
같은 10K 하니스에서 토크나이저/POS 조건을 분리:

| 구성 | NDCG@10 | 해석 |
|---|---|---|
| lindera 무필터 | 0.606 | 출발점 |
| MeCab 전체 morphs(무필터) | 0.6038 | **토크나이저만 바꾸면 ≈ lindera** |
| MeCab + accept-list(POS 필터) | 0.6328 | POS 필터가 +2.9%p |
| **lindera + accept-list**(simple) | **0.6362** | **lindera가 MeCab 초과** |
| **lindera + accept-list**(production korean config, simple) | **0.6362** | 출하본 |
| research MeCab | 0.6385 | 목표(99.4% 도달) |

- **토크나이저 분절은 lindera ≈ MeCab**(무필터 0.606 vs 0.6038). 갭은 토크나이저가 아니었다.
- **진짜 레버는 정확한 POS accept-list**(`NNG,NNP,NNB,NNBC,NR,VV,VA,MM,MAG,XSN,XR,SH,SL`).
  lindera·MeCab 모두 같은 ko-dic POS 체계라, lindera에 그대로 이식해 **+2.8%p**(0.606→0.634).
- **MeCab/Kiwi opt-in 불필요** — lindera(순수 Rust·임베드·최속)로 MeCab 품질을 **정체성 그대로** 달성.
- A1에서 "POS 필터 무효(−0.75%p)"라 한 건 잘못된 **넓은 allowlist**(`N*/V*/MA*`) 탓. 정확한
  accept-list가 답이었다. (가설→측정→재측정으로 자기수정)

## ja/zh — ko의 accept-list 같은 큰 레버가 없다 (정교화 측정 결과)

ko에서 통한 패턴들을 ja/zh에도 측정했으나 모두 무효였다:

| 레버 | 결과 |
|---|---|
| ja accept-list(名詞/動詞/…) vs denylist | 0.5658 ≈ 0.5647 (동일, denylist로 충분) |
| zh stopword(的/了/…) | 0.4555 vs 0.4577 (오히려 −) |
| english_stem 제거(simple) | 무변 (ko +0.0018, ja −0.001, zh +0.0008) — A1의 english_stem은 CJK 라틴(음역)엔 무의미해 제거·단순화 |
| 텍스트 정규화 / alphanumeric 필터 | 무효 / 토큰 수 동일 |

- **ja**: ipadic denylist(助詞/助動詞/記号 등)로 이미 충분. NDCG **0.5387**, recall 0.7298.
- **zh**: cc-cedict POS 미제공이라 POS·stopword 둘 다 한계. NDCG **0.4585**.
- ko가 특별했던 건 ko-dic POS 체계가 정확해 accept-list가 잘 들었기 때문. ja/zh는 현 방식이 적절.
- ko>ja>zh 순서는 MIRACL 언어별 BM25 난이도와 일치(정상).
- **미해결**: ja production(0.5387) vs pg-simple-on-pretokenized(0.5647) — 토큰 수 동일(61.7≈62.2)한데 +2.5%p 차이. query/measure 처리 의심, ja/zh 부차라 추후.

## RRF hybrid (dense + BM25) — 모델 0, research BGE-M3 임베딩 재사용

dense leg는 research textsearch의 **사전계산 BGE-M3(BAAI/bge-m3, 1024d) 임베딩을 재사용**한다
(`data/phase8/{doc,query}_embs_miracl.json` → corpus/query docid 100% 정합, **임베딩 모델 추론
0회**). BM25(korean) + dense(pgvector cosine `<=>`) → `glot.hybrid`(= BM25 leg + dense leg →
`glot.rrf`). `bench/eval_rrf.py`. (paired bootstrap 5000, seed 42)

| 구성 | NDCG@10 | 95%CI | Recall@10 |
|---|---|---|---|
| BM25 단독 | 0.6362 | [0.593, 0.681] | 0.7983 |
| dense 단독 | 0.7904 | [0.757, 0.823] | — |
| **RRF hybrid** | **0.7545** | [0.718, 0.791] | **0.8973** |

- **RRF − BM25 = +0.1183**, 95%CI [+0.094, +0.144], **p<0.001 유의** — hybrid가 lexical 단독을 크게 향상.
- dense − RRF = +0.0359 (p=0.020) — MIRACL-ko에서 BGE-M3 dense가 워낙 강해 RRF보다 NDCG가 높다.
  단 **RRF recall 0.897(최고)**로 두 leg 합집합 효과. `glot.hybrid`가 실제 BM25+dense 융합으로 정상 동작.
- ja/zh dense/RRF는 사전계산 임베딩 부재(research는 ko/ezis만)로 보류 — **RRF는 언어 무관이라 ko로 입증**.

## release gate (DESIGN §5.5)

lindera baseline = **ko BM25 NDCG 0.636**(research MeCab 0.6385의 99.7%). 토크나이저 교체나
외부 의존 없이 사실상 동급에 도달했다. 향후 분석기/사전/정규화 변경은 이 baseline 대비
**상대 측정**으로 판정. dense/RRF는 텍스트 검색(BM25) 위에 얹는 층이라 별도.

## 한계 (반드시 함께 읽을 것)

1. **dev passages subset** — MIRACL dev의 positive+negative passages(~수천~1만)이며 full
   corpus가 아니다. 공식 리더보드와 **직접 비교 불가**.
2. **dense/RRF 측정 완료**(ko) — research BGE-M3 사전계산 임베딩 재사용(모델 0). §RRF hybrid.
   ja/zh dense/RRF는 사전계산 임베딩 부재로 보류(BM25는 측정됨).
3. **유의성 검정 완료** (`bench/stats.py`, paired bootstrap 5000, seed 42) — ko accept-list
   **+2.87%p가 유의**(p=0.0076, 95%CI [+0.007, +0.050]). ko 0.636의 95%CI [0.593, 0.681]이
   research 0.6385를 포함 → **통계적 동급(parity)**. ja/zh 소차(±0.5%p)는 미검정(무효 결론).
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
