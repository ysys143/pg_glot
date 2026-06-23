#!/usr/bin/env python3
"""ja/zh BGE-M3 임베딩 1회 생성 — ko는 research 사전계산 재사용이지만 ja/zh는 사전계산본이
없어(Cohere ko=404/ja=gated, 나머지 corpus-only) 직접 추론한다. research가 ko에 쓴 것과
**동일하게 FlagEmbedding의 BGEM3FlagModel(dense_vecs)** 사용. GPU(cuda)면 use_fp16.
결과는 bench/data/{lang}/{doc,query}_embs.json.

사용: python3 bench/embed_cjk.py ja   (또는 zh) — GPU VM 권장(CPU는 느림).
"""
from __future__ import annotations

import json
import sys

import torch
from FlagEmbedding import BGEM3FlagModel


def main() -> None:
    lang = sys.argv[1]
    use_cuda = torch.cuda.is_available()
    print(f"device: {'cuda' if use_cuda else 'cpu'}")
    model = BGEM3FlagModel("BAAI/bge-m3", use_fp16=use_cuda)
    corpus = json.load(open(f"bench/data/{lang}/corpus.json", encoding="utf-8"))
    queries = json.load(open(f"bench/data/{lang}/queries.json", encoding="utf-8"))
    bs = 64 if use_cuda else 16
    dv = model.encode([d["text"] for d in corpus], batch_size=bs, max_length=512)["dense_vecs"]
    qv = model.encode([q["text"] for q in queries], batch_size=bs, max_length=512)["dense_vecs"]
    with open(f"bench/data/{lang}/doc_embs.json", "w", encoding="utf-8") as f:
        json.dump({str(d["id"]): v.tolist() for d, v in zip(corpus, dv)}, f)
    with open(f"bench/data/{lang}/query_embs.json", "w", encoding="utf-8") as f:
        json.dump({str(q["query_id"]): v.tolist() for q, v in zip(queries, qv)}, f)
    print(f"{lang}: {len(corpus)} docs, {len(queries)} queries embedded (dim {len(dv[0])})")


if __name__ == "__main__":
    main()
