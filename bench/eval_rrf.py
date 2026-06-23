#!/usr/bin/env python3
"""RRF 평가 — BM25 단독 vs dense 단독 vs glot.hybrid(RRF) NDCG@10 + paired bootstrap.

lang(ko/ja/zh) 인자. 임베딩은 bench/data/{lang}/{doc,query}_embs.json:
- ko: research textsearch 사전계산 BGE-M3 재사용(모델 0).
- ja/zh: 사전계산본이 없어 BGE-M3 1회 추론(bench/embed_cjk.py).
docid가 비-숫자(MIRACL ja/zh '12345#6')일 수 있어 corpus row index로 매핑한다(glot.rrf는 i64 id).
BM25(korean/japanese/chinese) + dense(pgvector cosine) → glot.hybrid(= BM25 leg + dense leg → glot.rrf).
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
CFG = {"ko": "korean", "ja": "japanese", "zh": "chinese"}


def vec_literal(e: list[float]) -> str:
    return "[" + ",".join(map(str, e)) + "]"


def main() -> None:
    lang = sys.argv[1] if len(sys.argv) > 1 else "ko"
    data = f"bench/data/{lang}"
    cfg = f"public.{CFG[lang]}"
    corpus = json.load(open(f"{data}/corpus.json", encoding="utf-8"))
    queries = json.load(open(f"{data}/queries.json", encoding="utf-8"))
    doc_emb = json.load(open(f"{data}/doc_embs.json", encoding="utf-8"))
    q_emb = json.load(open(f"{data}/query_embs.json", encoding="utf-8"))
    dim = len(next(iter(doc_emb.values())))
    d2i = {str(d["id"]): i + 1 for i, d in enumerate(corpus)}  # docid → i64 row index

    with psycopg.connect(DSN) as conn, conn.cursor() as cur:
        cur.execute("DROP TABLE IF EXISTS rdocs")
        cur.execute(f"CREATE TABLE rdocs(id bigint primary key, body text, emb vector({dim}))")
        with cur.copy("COPY rdocs(id, body, emb) FROM STDIN") as cp:
            for d in corpus:
                cp.write_row((d2i[str(d["id"])], d["text"], vec_literal(doc_emb[str(d["id"])])))
        cur.execute(f"CREATE INDEX ON rdocs USING bm25(body) WITH (text_config='{cfg}')")
        cur.execute("CREATE INDEX ON rdocs USING hnsw(emb vector_cosine_ops)")
        conn.commit()

        bm, dn, rr, rbm, rrr = [], [], [], [], []
        for q in queries:
            rel = [str(d2i[r]) for r in q["relevant_ids"] if r in d2i]
            qv = vec_literal(q_emb[str(q["query_id"])])
            cur.execute(
                sql.SQL("SELECT id FROM rdocs ORDER BY body OPERATOR(public.<@>) {} LIMIT 10").format(
                    sql.Literal(q["text"])
                )
            )
            bm_ids = [str(r[0]) for r in cur.fetchall()]
            cur.execute(
                sql.SQL("SELECT id FROM rdocs ORDER BY emb OPERATOR(public.<=>) {}::vector LIMIT 10").format(
                    sql.Literal(qv)
                )
            )
            dn_ids = [str(r[0]) for r in cur.fetchall()]
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

    print(f"[{lang}] n_queries={len(queries)}, docs={len(corpus)}, emb dim={dim}")
    print(f"  BM25  NDCG@10 = {statistics.mean(bm):.4f}  R@10 = {statistics.mean(rbm):.4f}")
    print(f"  dense NDCG@10 = {statistics.mean(dn):.4f}")
    print(f"  RRF   NDCG@10 = {statistics.mean(rr):.4f}  R@10 = {statistics.mean(rrr):.4f}")
    from stats import paired_bootstrap

    o, lo, hi, p = paired_bootstrap(rr, bm)
    sig = "유의" if lo > 0 else "비유의"
    print(f"  RRF − BM25  Δ = {o:+.4f}  95%CI = [{lo:+.4f}, {hi:+.4f}]  p = {p:.4f}  → {sig}")
    o, lo, hi, p = paired_bootstrap(dn, rr)
    print(f"  dense − RRF Δ = {o:+.4f}  95%CI = [{lo:+.4f}, {hi:+.4f}]  p = {p:.4f}")


if __name__ == "__main__":
    main()
