#!/usr/bin/env python3
"""pg_trgm(문자 trigram 유사도) baseline — 형태소 분석 없이 GIN/GiST trigram만으로 검색했을 때의
recall/NDCG. lindera 토크나이저(korean/japanese/chinese config) 대비 "stock PG 확장만 쓰면
CJK 검색이 얼마나 나오나"를 보여주는 비교 baseline(측정 전용).

word_similarity(query, doc) 거리(`<<->`)로 KNN 정렬(GiST gist_trgm_ops). 질의 trigram이 문서에
얼마나 들어있는지의 비대칭 유사도라 짧은 질의 ↔ 긴 문서에 적합하다.

사용: python3 bench/eval_trgm.py ko   (또는 ja/zh)
"""
from __future__ import annotations

import json
import os
import statistics
import sys

import psycopg

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_glot import ndcg_at_k, recall_at_k  # noqa: E402

DSN = "host=localhost port=5433 user=postgres password=pw dbname=postgres"


def main() -> None:
    lang = sys.argv[1] if len(sys.argv) > 1 else "ko"
    data = f"bench/data/{lang}"
    corpus = json.load(open(f"{data}/corpus.json", encoding="utf-8"))
    queries = json.load(open(f"{data}/queries.json", encoding="utf-8"))

    with psycopg.connect(DSN) as conn, conn.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS tg")
        cur.execute("CREATE TABLE tg(id text primary key, body text)")
        with cur.copy("COPY tg(id, body) FROM STDIN") as cp:
            for d in corpus:
                cp.write_row((str(d["id"]), d["text"]))
        # GiST trigram → word_similarity 거리(`<<->`) KNN 정렬 가속.
        cur.execute("CREATE INDEX tg_gist ON tg USING gist (body gist_trgm_ops)")
        conn.commit()

        nd, rc = [], []
        for q in queries:
            # word_similarity 거리(`<<->`)로 KNN 정렬. GiST gist_trgm_ops 인덱스가 가속.
            cur.execute("SELECT id FROM tg ORDER BY body <<-> %s LIMIT 10", (q["text"],))
            ret = [r[0] for r in cur.fetchall()]
            rel = [str(r) for r in q["relevant_ids"]]
            nd.append(ndcg_at_k(rel, ret, 10))
            rc.append(recall_at_k(rel, ret, 10))

    print(f"[{lang}] pg_trgm  docs={len(corpus)}  queries={len(queries)}")
    print(f"  ndcg@10  = {statistics.mean(nd):.4f}")
    print(f"  recall@10 = {statistics.mean(rc):.4f}")


if __name__ == "__main__":
    main()
