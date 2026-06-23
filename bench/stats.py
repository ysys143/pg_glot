#!/usr/bin/env python3
"""통계 엄밀성 — per-query NDCG의 bootstrap 95% CI + paired bootstrap 유의성 검정.

ko POS accept-list가 무필터 대비 유의한 개선인지(같은 213 query) 검정한다. 두 변형을
lindera(ko-dic)로 사전 토큰화해 PG simple BM25로 색인·검색하고, per-query NDCG@10에
bootstrap을 적용한다. 공개 데이터(MIRACL-ko), 실제 pg_textsearch 엔진. 시드 고정(재현).

사용: python3 bench/stats.py   (Docker 컨테이너 port 5433 가정)
"""
from __future__ import annotations

import json
import os
import random
import re
import subprocess
import sys

import psycopg

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_glot import evaluate  # noqa: E402

DSN = "host=localhost port=5433 user=postgres password=pw dbname=postgres"
LINDERA = os.path.expanduser("~/.cargo/bin/lindera")
ACCEPT = {"NNG", "NNP", "NNB", "NNBC", "NR", "VV", "VA", "MM", "MAG", "XSN", "XR", "SH", "SL"}
BOOT = 5000
random.seed(42)


def norm(t: str) -> str:
    t = re.sub(r"\s+", " ", t).strip()
    return t or "x"


def lindera_build(in_dir: str, accept_filter: bool) -> dict:
    """ko-dic로 corpus/queries 사전 토큰화(accept_filter면 내용어 POS만)."""
    res = {}
    for split in ["corpus", "queries"]:
        data = json.load(open(f"{in_dir}/{split}.json", encoding="utf-8"))
        texts = [norm(d["text"]) for d in data]
        out = subprocess.run(
            [LINDERA, "tokenize", "-d", "embedded://ko-dic"],
            input="\n".join(texts),
            capture_output=True,
            text=True,
        ).stdout
        docs, cur = [], []
        for line in out.split("\n"):
            if line == "EOS":
                docs.append(cur)
                cur = []
            elif "\t" in line:
                s, rest = line.split("\t", 1)
                pos = rest.split(",")[0].split("+")[0]
                if (not accept_filter) or (pos in ACCEPT):
                    cur.append(s)
        assert len(docs) == len(data), f"{in_dir}/{split}: {len(docs)} != {len(data)}"
        res[split] = [dict(d, text=" ".join(t)) for d, t in zip(data, docs)]
    return res


def boot_ci(vals: list[float], n: int = BOOT) -> tuple[float, float]:
    N = len(vals)
    means = sorted(sum(vals[random.randrange(N)] for _ in range(N)) / N for _ in range(n))
    return means[int(0.025 * n)], means[int(0.975 * n)]


def paired_bootstrap(a: list[float], b: list[float], n: int = BOOT):
    d = [x - y for x, y in zip(a, b)]
    N = len(d)
    means = sorted(sum(d[random.randrange(N)] for _ in range(N)) / N for _ in range(n))
    obs = sum(d) / N
    lo, hi = means[int(0.025 * n)], means[int(0.975 * n)]
    # two-sided bootstrap p: 부호가 반대편을 넘는 비율
    p = 2.0 * min(sum(1 for m in means if m <= 0), sum(1 for m in means if m >= 0)) / n
    return obs, lo, hi, min(p, 1.0)


def per_q(conn, data: dict, cfg: str) -> tuple[list[float], float]:
    r = evaluate(conn, "x", data["corpus"], data["queries"], 10, cfg)
    return [x["ndcg"] for x in r["per_query"]], r["ndcg@10"]


def main() -> None:
    acc = lindera_build("bench/data/ko", True)
    nof = lindera_build("bench/data/ko", False)
    with psycopg.connect(DSN) as conn:
        a_q, a_m = per_q(conn, acc, "pg_catalog.simple")
        n_q, n_m = per_q(conn, nof, "pg_catalog.simple")
    alo, ahi = boot_ci(a_q)
    nlo, nhi = boot_ci(n_q)
    obs, dlo, dhi, p = paired_bootstrap(a_q, n_q)
    print(f"n_queries = {len(a_q)},  bootstrap = {BOOT}, seed = 42")
    print(f"ko accept-list  NDCG@10 = {a_m:.4f}  95%CI = [{alo:.4f}, {ahi:.4f}]")
    print(f"ko no-filter    NDCG@10 = {n_m:.4f}  95%CI = [{nlo:.4f}, {nhi:.4f}]")
    print(f"paired Δ(accept−nofilter) = {obs:+.4f}  95%CI = [{dlo:+.4f}, {dhi:+.4f}]  p = {p:.4f}")
    print("→ 유의(95% CI가 0을 포함하지 않음)" if dlo > 0 else "→ 비유의(CI가 0 포함)")


if __name__ == "__main__":
    main()
