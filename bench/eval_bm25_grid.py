#!/usr/bin/env python3
"""BM25 k1/b 그리드 — 토크나이저(lindera korean) 고정, 인덱스 파라미터만 스윕.

corpus를 1회만 적재하고 k1/b 조합마다 인덱스를 재생성해 NDCG@10/Recall@10을 측정한다.
k1/b는 토크나이저와 무관한 BM25 랭킹 레버이며 지금까지 pg_textsearch 기본값(1.2/0.75)만
썼다 — 텍스트 검색 품질을 토크나이저 교체 없이 올릴 수 있는지 보는 게 목적.

사용: python3 bench/eval_bm25_grid.py [lang] [corpus.json] [queries.json]
"""
from __future__ import annotations

import itertools
import json
import os
import sys

import psycopg
from psycopg import sql

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_glot import ndcg_at_k, recall_at_k  # noqa: E402

DSN = "host=localhost port=5433 user=postgres password=pw dbname=postgres"
K1_GRID = [0.8, 1.0, 1.2, 1.5, 2.0]
B_GRID = [0.3, 0.5, 0.75, 0.9]


def main() -> None:
    config = sys.argv[1] if len(sys.argv) > 1 else "public.korean"
    corpus_path = sys.argv[2] if len(sys.argv) > 2 else "bench/data/ko/corpus.json"
    queries_path = sys.argv[3] if len(sys.argv) > 3 else "bench/data/ko/queries.json"
    with open(corpus_path, encoding="utf-8") as f:
        corpus = json.load(f)
    with open(queries_path, encoding="utf-8") as f:
        queries = json.load(f)

    with psycopg.connect(DSN) as conn, conn.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS eg")
        cur.execute("CREATE TABLE eg(id text primary key, body text)")
        with cur.copy("COPY eg(id, body) FROM STDIN") as cp:
            for d in corpus:
                cp.write_row((str(d["id"]), d["text"]))
        conn.commit()
        print(f"config={config}  docs={len(corpus)}  queries={len(queries)}")
        print(f"{'k1':>5} {'b':>5} {'ndcg@10':>9} {'recall@10':>10}")
        best = (None, -1.0)
        for k1, b in itertools.product(K1_GRID, B_GRID):
            cur.execute("DROP INDEX IF EXISTS eg_bm25")
            cur.execute(
                sql.SQL(
                    "CREATE INDEX eg_bm25 ON eg USING bm25(body) "
                    "WITH (text_config={}, k1={}, b={})"
                ).format(sql.Literal(config), sql.Literal(k1), sql.Literal(b))
            )
            conn.commit()
            ndcgs, recalls = [], []
            for q in queries:
                cur.execute(
                    sql.SQL("SELECT id FROM eg ORDER BY body <@> {} LIMIT 10").format(
                        sql.Literal(q["text"])
                    )
                )
                ret = [r[0] for r in cur.fetchall()]
                ndcgs.append(ndcg_at_k(q["relevant_ids"], ret, 10))
                recalls.append(recall_at_k(q["relevant_ids"], ret, 10))
            nd = sum(ndcgs) / len(ndcgs)
            rc = sum(recalls) / len(recalls)
            print(f"{k1:>5} {b:>5} {nd:>9.4f} {rc:>10.4f}")
            if nd > best[1]:
                best = ((k1, b), nd)
        print(f"\nbest: k1={best[0][0]} b={best[0][1]} → ndcg@10={best[1]:.4f}  (기본 1.2/0.75 대비)")


if __name__ == "__main__":
    main()
