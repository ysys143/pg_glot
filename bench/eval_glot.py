#!/usr/bin/env python3
"""glot 평가 하니스 — 실제 pg_glot config + pg_textsearch BM25로 NDCG@10/Recall@10.

ko/ja/zh corpus를 각 언어 config(`public.korean`/`japanese`/`chinese`)로 색인·검색해
검색 품질을 측정한다. ko는 공개 release gate(연구 Phase7 BM25 품질 재현), ja/zh는
구조 지원 품질지표(미검증 → 측정).

주의:
- pg_textsearch `<@>`는 plain ORDER BY+LIMIT(인덱스 스캔) + **리터럴 질의**에서만 인덱스
  매칭(planner hook). 그래서 질의는 psycopg `sql.Literal`로 인라인한다(bind 파라미터 X).
- 1차는 BM25만(임베딩 없음). dense/RRF(glot.hybrid)는 임베딩 컬럼이 생기면 추가.

사용:
  python3 bench/eval_glot.py --lang korean \
      --corpus bench/data/ko/corpus.json --queries bench/data/ko/queries.json \
      --dsn "host=localhost port=5432 user=postgres password=pw dbname=postgres"
"""
from __future__ import annotations

import argparse
import json
import math
import statistics
import sys

import psycopg
from psycopg import sql


def ndcg_at_k(relevant: list[str], retrieved: list[str], k: int = 10) -> float:
    rel = set(relevant)
    dcg = sum(1.0 / math.log2(i + 2) for i, d in enumerate(retrieved[:k]) if d in rel)
    ideal = min(len(rel), k)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(ideal))
    return dcg / idcg if idcg else 0.0


def recall_at_k(relevant: list[str], retrieved: list[str], k: int = 10) -> float:
    rel = set(relevant)
    hits = sum(1 for d in retrieved[:k] if d in rel)
    return hits / len(rel) if rel else 0.0


def evaluate(
    conn,
    lang: str,
    corpus: list[dict],
    queries: list[dict],
    k: int = 10,
    config: str | None = None,
) -> dict:
    cfg = config or f"public.{lang}"
    with conn.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS eval_docs")
        cur.execute("CREATE TABLE eval_docs(id text primary key, body text)")
        with cur.copy("COPY eval_docs(id, body) FROM STDIN") as cp:
            for d in corpus:
                cp.write_row((str(d["id"]), d["text"]))
        cur.execute(
            sql.SQL("CREATE INDEX eval_bm25 ON eval_docs USING bm25(body) WITH (text_config={})")
            .format(sql.Literal(cfg))
        )
        conn.commit()

        ndcgs, recalls, empty = [], [], 0
        for q in queries:
            # 리터럴 인라인(planner hook은 Const 질의만 인덱스 매칭).
            stmt = sql.SQL("SELECT id FROM eval_docs ORDER BY body <@> {} LIMIT {}").format(
                sql.Literal(q["text"]), sql.Literal(k)
            )
            cur.execute(stmt)
            retrieved = [r[0] for r in cur.fetchall()]
            if not retrieved:
                empty += 1
            ndcgs.append(ndcg_at_k(q["relevant_ids"], retrieved, k))
            recalls.append(recall_at_k(q["relevant_ids"], retrieved, k))

    return {
        "lang": lang,
        "config": cfg,
        "n_docs": len(corpus),
        "n_queries": len(queries),
        "ndcg@10": round(statistics.mean(ndcgs), 4),
        "recall@10": round(statistics.mean(recalls), 4),
        "empty_results": empty,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lang", required=True, help="korean | japanese | chinese")
    ap.add_argument("--corpus", required=True, help="JSON list of {id, text}")
    ap.add_argument("--queries", required=True, help="JSON list of {query_id, text, relevant_ids}")
    ap.add_argument(
        "--dsn",
        default="host=localhost port=5432 user=postgres password=pw dbname=postgres",
    )
    ap.add_argument("-k", type=int, default=10)
    ap.add_argument("--config", default=None, help="text_config 오버라이드 (기본 public.{lang})")
    a = ap.parse_args()

    with open(a.corpus, encoding="utf-8") as f:
        corpus = json.load(f)
    with open(a.queries, encoding="utf-8") as f:
        queries = json.load(f)

    with psycopg.connect(a.dsn, autocommit=False) as conn:
        res = evaluate(conn, a.lang, corpus, queries, a.k, a.config)
    print(json.dumps(res, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
