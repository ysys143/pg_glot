#!/usr/bin/env python3
"""RRF 평가 — BM25(korean) 단독 vs dense(pgvector) 단독 vs glot.hybrid(RRF).

임베딩은 **모델을 돌리지 않고** 공개 사전계산본을 재사용한다: research textsearch의
BGE-M3(BAAI/bge-m3, 1024d) 임베딩(`data/phase8/{doc,query}_embs_miracl.json`)이 우리
corpus_10k/queries와 docid·query_id 100% 정합(공개 MIRACL-ko 기반). dense leg는
pgvector `<=>`(cosine), 융합은 `glot.hybrid`(= BM25 leg + dense leg → glot.rrf).

사용: python3 bench/eval_rrf.py   (Docker 컨테이너 port 5433 가정)
"""
from __future__ import annotations

import json
import os
import statistics
import sys

import psycopg
from psycopg import sql

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_glot import ndcg_at_k, recall_at_k  # noqa: E402

DSN = "host=localhost port=5433 user=postgres password=pw dbname=postgres"
DATA = "bench/data/ko"


def vec_literal(e: list[float]) -> str:
    return "[" + ",".join(map(str, e)) + "]"


def main() -> None:
    corpus = json.load(open(f"{DATA}/corpus.json", encoding="utf-8"))
    queries = json.load(open(f"{DATA}/queries.json", encoding="utf-8"))
    doc_emb = json.load(open(f"{DATA}/doc_embs.json", encoding="utf-8"))
    q_emb = json.load(open(f"{DATA}/query_embs.json", encoding="utf-8"))

    with psycopg.connect(DSN) as conn, conn.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS rdocs")
        cur.execute("CREATE TABLE rdocs(id bigint primary key, body text, emb vector(1024))")
        with cur.copy("COPY rdocs(id, body, emb) FROM STDIN") as cp:
            for d in corpus:
                cp.write_row((int(d["id"]), d["text"], vec_literal(doc_emb[str(d["id"])])))
        cur.execute(
            "CREATE INDEX ON rdocs USING bm25(body) WITH (text_config='public.korean')"
        )
        cur.execute("CREATE INDEX ON rdocs USING hnsw(emb vector_cosine_ops)")
        conn.commit()

        bm, dn, rr = [], [], []
        rbm, rrr = [], []
        for q in queries:
            rel = q["relevant_ids"]
            qv = vec_literal(q_emb[str(q["query_id"])])
            # BM25 단독 (리터럴 인라인 — planner hook)
            cur.execute(
                sql.SQL("SELECT id FROM rdocs ORDER BY body OPERATOR(public.<@>) {} LIMIT 10").format(
                    sql.Literal(q["text"])
                )
            )
            bm_ids = [str(r[0]) for r in cur.fetchall()]
            # dense 단독 (cosine)
            cur.execute(
                sql.SQL("SELECT id FROM rdocs ORDER BY emb OPERATOR(public.<=>) {}::vector LIMIT 10").format(
                    sql.Literal(qv)
                )
            )
            dn_ids = [str(r[0]) for r in cur.fetchall()]
            # RRF (glot.hybrid: BM25 + dense → glot.rrf)
            cur.execute(
                sql.SQL(
                    "SELECT id FROM glot.hybrid('rdocs','id','body','emb',{},{}::vector,60,60,10)"
                ).format(sql.Literal(q["text"]), sql.Literal(qv))
            )
            rr_ids = [str(r[0]) for r in cur.fetchall()]

            bm.append(ndcg_at_k(rel, bm_ids, 10))
            dn.append(ndcg_at_k(rel, dn_ids, 10))
            rr.append(ndcg_at_k(rel, rr_ids, 10))
            rbm.append(recall_at_k(rel, bm_ids, 10))
            rrr.append(recall_at_k(rel, rr_ids, 10))

    print(f"n_queries = {len(queries)}, docs = {len(corpus)}, emb = BGE-M3 1024d (사전계산 재사용)")
    print(f"BM25  단독  NDCG@10 = {statistics.mean(bm):.4f}  Recall@10 = {statistics.mean(rbm):.4f}")
    print(f"dense 단독  NDCG@10 = {statistics.mean(dn):.4f}")
    print(f"RRF hybrid  NDCG@10 = {statistics.mean(rr):.4f}  Recall@10 = {statistics.mean(rrr):.4f}")
    lift = statistics.mean(rr) - statistics.mean(bm)
    print(f"→ RRF − BM25 = {lift:+.4f}")

    # 통계 엄밀성: bootstrap 95% CI + paired bootstrap (seed 고정은 stats.py에서)
    from stats import boot_ci, paired_bootstrap

    print("--- 통계 (paired bootstrap 5000, seed 42) ---")
    for name, vals in [("BM25", bm), ("dense", dn), ("RRF", rr)]:
        lo, hi = boot_ci(vals)
        print(f"  {name:5} NDCG 95%CI = [{lo:.4f}, {hi:.4f}]")
    o, lo, hi, p = paired_bootstrap(rr, bm)
    sig = "유의" if lo > 0 else "비유의"
    print(f"  RRF − BM25  Δ = {o:+.4f}  95%CI = [{lo:+.4f}, {hi:+.4f}]  p = {p:.4f}  → {sig}")
    o, lo, hi, p = paired_bootstrap(dn, rr)
    print(f"  dense − RRF Δ = {o:+.4f}  95%CI = [{lo:+.4f}, {hi:+.4f}]  p = {p:.4f}")


if __name__ == "__main__":
    main()
