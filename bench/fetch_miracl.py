#!/usr/bin/env python3
"""MIRACL ja/zh dev → bench/data/{ja,zh}/{corpus,queries}.json (ko와 동일 형식).

full corpus(수 GB) 대신 dev split만 받는다: dev의 positive/negative passages를 모아
corpus(dedup)를, positive docids를 relevant_ids로 한 queries를 만든다. relevant 문서는
반드시 corpus에 포함(누락 시 NDCG 왜곡)되고, 이후 negative로 max_docs까지 채운다.

데이터: MIRACL (Apache-2.0 + Wikipedia CC-BY-SA). docs/DESIGN.md §7.
"""
from __future__ import annotations

import json
import os
import sys

from datasets import load_dataset

LANGS = {"ja": "japanese", "zh": "chinese"}


def fetch(hf_code: str, out_dir: str, max_docs: int = 10000) -> None:
    # MIRACL은 loading script(miracl.py) 기반 → datasets<3 + trust_remote_code 필요.
    ds = load_dataset("miracl/miracl", hf_code, split="dev", trust_remote_code=True)
    corpus: dict[str, str] = {}
    queries: list[dict] = []
    for row in ds:
        rels = []
        for p in row["positive_passages"]:
            corpus[p["docid"]] = p["text"]
            rels.append(p["docid"])
        for p in row["negative_passages"]:
            corpus.setdefault(p["docid"], p["text"])
        queries.append(
            {"query_id": str(row["query_id"]), "text": row["query"], "relevant_ids": rels}
        )

    rel_ids = {d for q in queries for d in q["relevant_ids"]}
    items = [{"id": k, "text": v} for k, v in corpus.items() if k in rel_ids]
    for k, v in corpus.items():
        if len(items) >= max_docs:
            break
        if k not in rel_ids:
            items.append({"id": k, "text": v})

    os.makedirs(out_dir, exist_ok=True)
    with open(f"{out_dir}/corpus.json", "w", encoding="utf-8") as f:
        json.dump(items, f, ensure_ascii=False)
    with open(f"{out_dir}/queries.json", "w", encoding="utf-8") as f:
        json.dump(queries, f, ensure_ascii=False)
    print(
        f"{hf_code}: {len(items)} docs ({len(rel_ids)} relevant), {len(queries)} queries → {out_dir}"
    )


if __name__ == "__main__":
    codes = sys.argv[1:] or list(LANGS.keys())
    for code in codes:
        fetch(code, f"bench/data/{code}")
