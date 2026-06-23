# glot 검색 품질 — BM25 NDCG (실제 pg_glot + pg_textsearch)

`bench/eval_glot.py`로 측정. 각 언어 config(`public.korean`/`japanese`/`chinese`)로
실제 pg_textsearch BM25 인덱스를 만들고 검색 → NDCG@10 / Recall@10. **bakeoff/(순수
Python BM25, INDICATIVE)와 달리 실제 엔진 측정**이지만, 아래 한계를 반드시 함께 읽을 것.

## 결과 (MIRACL dev, BM25-only)

| lang | config | docs | queries | NDCG@10 | Recall@10 | empty |
|---|---|---|---|---|---|---|
| ko | `public.korean`   | 10000 | 213 | **0.6362** | 0.7983 | 0 |
| ja | `public.japanese` |  8066 | 860 | **0.5647** | 0.7727 | 0 |
| zh | `public.chinese`  |  3786 | 393 | **0.4585** | 0.6461 | 0 |

ko는 MeCab accept-list POS 필터(simple 매핑). **research MeCab BM25(0.6385)의 99.7%**.
ja는 가타카나 중점(・) 분할 적용 후 0.5387→**0.5647**(아래 §미해결 해소 참조).

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
| 텍스트 정규화 | 무효 / 토큰 수 동일 |
| alphanumeric 필터(`is_alphanumeric`) | **유익/중립** — denylist 통과한 순수 문장부호(`-`,`(`,`)`,`.` 등 1.6%)만 blank 처리. 갭 원인 아님 |
| **가타카나 중점(・) 분할** | **ja +2.6%p** (0.5387→0.5647, recall +4.4%p) — §미해결 해소 |

- **ja**: ipadic denylist + ・ 분할. NDCG **0.5647**, recall 0.7727.
- **zh**: cc-cedict POS 미제공이라 POS·stopword 둘 다 한계. NDCG **0.4585**.
- ko가 특별했던 건 ko-dic POS 체계가 정확해 accept-list가 잘 들었기 때문. zh는 현 방식이 적절.
- ko>ja>zh 순서는 MIRACL 언어별 BM25 난이도와 일치(정상).

### §미해결 해소 — ja 갭의 원인은 가타카나 중점(・) (measure-first)

ja production(0.5387) vs pg-simple-on-pretokenized(0.5647)의 +2.6%p 갭을 끝까지 측정으로 분해했다.
두 경로는 대칭(lindera 3.0.7 ipadic Normal + denylist + simple)이라 **분절은 동일**(`ts_debug` 대조 확인).
실제 원인은 **tsvector lexeme 집합 차이**였다:

- lindera/ipadic은 외국 인명 `トーマス・エジソン`을 **단일 토큰**으로 emit → custom parser가 통째로
  색인 → 질의 `エジソン`이 매칭 실패. (pg-simple 경로는 PG 기본 파서가 ・에서 분할해 이득을 봤다.)
- 가설 후보였던 `is_alphanumeric` 필터는 **오인** — 그건 순수 문장부호(1.6%)만 떨구는 유익/중립이었다.
- 수정: `glot-tokenizer`가 ・(U+30FB)·반각 ･(U+FF65)에서 surface를 하위 토큰으로 분할(byte-offset
  불변식 유지). **ja 0.5387→0.5647(pg-simple과 동일), recall 0.7283→0.7727. ko 0.6362 / zh 0.4585
  회귀 0**(・는 일본어 부호라 ko/zh 무영향, 측정 확인).
- 가설→측정(재현)→lexeme 진단→ablation→수정→회귀검증으로 자기수정.

## RRF hybrid (dense + BM25) — CJK 전체 (BGE-M3 임베딩)

BM25(korean/japanese/chinese) + dense(pgvector cosine `<=>`) → `glot.hybrid`(= BM25 leg + dense
leg → `glot.rrf`). dense는 BGE-M3(BAAI/bge-m3, 1024d): **ko는 research 사전계산 재사용(모델 0)**,
**ja/zh는 사전계산본이 없어 GPU(T4)에서 FlagEmbedding으로 1회 추론**(research가 ko에 쓴 것과
동일 모델). `bench/eval_rrf.py`, `bench/embed_cjk.py`. (paired bootstrap 5000, seed 42)

| lang | docs/q | BM25 | dense | **RRF** | RRF recall | RRF−BM25 (p) |
|---|---|---|---|---|---|---|
| ko | 10000/213 | 0.6362 | 0.7904 | **0.7545** | 0.8973 | +0.1183 (p<0.001) |
| ja | 8066/860 | 0.5647 | 0.7705 | **0.6910** | 0.8585 | +0.1263 (p<0.001) |
| zh | 3786/393 | 0.4585 | 0.7922 | **0.6245** | 0.8173 | +0.1659 (p<0.001) |

- **3개 언어 모두 RRF가 BM25를 유의하게 +0.12~0.17 향상**(p<0.001) — hybrid의 가치 입증.
- dense > RRF도 모든 언어 유의(BGE-M3가 MIRACL에서 워낙 강함, 정상). 단 **RRF recall이 각 언어 최고**
  (두 leg 합집합). `glot.hybrid`가 CJK 전반에서 실제 BM25+dense 융합으로 정상 동작.
- ko dense는 모델 추론 0(research 재사용); ja/zh는 사전계산본 부재로 GPU 1회 추론(측정 재현용).

## release gate (DESIGN §5.5)

lindera baseline = **ko BM25 NDCG 0.636**(research MeCab 0.6385의 99.7%). 토크나이저 교체나
외부 의존 없이 사실상 동급에 도달했다. 향후 분석기/사전/정규화 변경은 이 baseline 대비
**상대 측정**으로 판정. dense/RRF는 텍스트 검색(BM25) 위에 얹는 층이라 별도.

## 한계 (반드시 함께 읽을 것)

1. **dev passages subset** — MIRACL dev의 positive+negative passages(~수천~1만)이며 full
   corpus가 아니다. 공식 리더보드와 **직접 비교 불가**.
2. **dense/RRF 측정 완료**(ko/ja/zh) — §RRF hybrid. ko는 모델 추론 0(research 재사용),
   ja/zh는 사전계산본 부재로 GPU(T4) 1회 추론(`bench/embed_cjk.py`, FlagEmbedding BGE-M3).
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
