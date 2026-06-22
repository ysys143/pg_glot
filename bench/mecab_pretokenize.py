#!/usr/bin/env python3
"""MeCab-ko(python-mecab-ko)로 corpus/queries를 사전 토큰화 — lindera vs MeCab 갭을 같은
BM25 하니스에서 직접 측정하기 위함. research textsearch_ko의 accept POS만 남겨 공백 join
→ PG `simple` config로 색인(공백 분리 = mecab 토큰이 곧 lexeme).

research accept(ts_mecab_ko.c accept_parts_of_speech):
  NNG NNP NNB NNBC NR VV VA MM MAG XSN XR SH SL
복합 태그(예: 'VV+EC')는 첫 형태소(VV) 기준으로 판정.

주의: 이건 측정 전용(MeCab 토크나이저 효과 정량화)이지 제품 코드가 아니다. pg_glot의
정체성은 lindera(순수 Rust 임베드). MeCab은 C 라이브러리+외부 사전 의존이라 도입한다면
opt-in이어야 하고, 한국어 품질은 Kiwi가 더 적합하다.
"""
from __future__ import annotations

import json
import os
import sys

from mecab import MeCab

ACCEPT = {"NNG", "NNP", "NNB", "NNBC", "NR", "VV", "VA", "MM", "MAG", "XSN", "XR", "SH", "SL"}


def wakati(m: MeCab, text: str) -> str:
    out = []
    for surface, tag in m.pos(text):
        if tag.split("+")[0] in ACCEPT:
            out.append(surface)
    return " ".join(out)


def convert(m: MeCab, in_dir: str, out_dir: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    with open(f"{in_dir}/corpus.json", encoding="utf-8") as f:
        corpus = json.load(f)
    with open(f"{in_dir}/queries.json", encoding="utf-8") as f:
        queries = json.load(f)
    for d in corpus:
        d["text"] = wakati(m, d["text"])
    for q in queries:
        q["text"] = wakati(m, q["text"])
    with open(f"{out_dir}/corpus.json", "w", encoding="utf-8") as f:
        json.dump(corpus, f, ensure_ascii=False)
    with open(f"{out_dir}/queries.json", "w", encoding="utf-8") as f:
        json.dump(queries, f, ensure_ascii=False)
    print(f"{in_dir} → {out_dir}: {len(corpus)} docs, {len(queries)} queries")


if __name__ == "__main__":
    src = sys.argv[1] if len(sys.argv) > 1 else "bench/data/ko"
    dst = sys.argv[2] if len(sys.argv) > 2 else "bench/data/ko_mecab"
    convert(MeCab(), src, dst)
