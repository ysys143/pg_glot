"""
bakeoff/run_bakeoff.py

한국어 형태소 분석기 bake-off:
  M1) 토큰 일치율 (vs python-mecab-ko baseline) — MIRACL + MDN 코퍼스
       (a) 토큰 시퀀스 정확일치율, (b) 토큰 집합 Jaccard
  M2) BM25 NDCG@10 (+Recall@10, MRR) — MIRACL-ko

기존 textsearch 하니스(BM25Embedder의 IDF/scoring, ndcg_at_k 등)를 재사용하고
토크나이저만 3종(+hephaex sejong 변형)으로 갈아끼운다.

사용:
  /Users/.../textsearch/.venv/bin/python run_bakeoff.py
"""
from __future__ import annotations

import json
import math
import re
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Callable, Dict, List

# textsearch 하니스 재사용
TEXTSEARCH_ROOT = Path("/Users/jaesolshin/Documents/GitHub/textsearch")
sys.path.insert(0, str(TEXTSEARCH_ROOT))

from tokenizers import MecabKoBaseline, HephaexMecabKo, Lindera  # noqa: E402

# ---------------------------------------------------------------------------
# 데이터 경로
# ---------------------------------------------------------------------------
MIRACL_DOCS = TEXTSEARCH_ROOT / "data/miracl/docs_ko_miracl.json"
MIRACL_QUERIES = TEXTSEARCH_ROOT / "data/miracl/queries_dev.json"
MDN_ROOT = Path("/tmp/mdn_clone/files/ko")
OUT_DIR = Path("/Users/jaesolshin/Documents/GitHub/pg_textsearch_ko/bakeoff")

N_MATCH_SAMPLE = 800   # M1 토큰 일치율 샘플 수 (코퍼스당)


# ---------------------------------------------------------------------------
# 메트릭 (phase1 하니스에서 차용)
# ---------------------------------------------------------------------------
def ndcg_at_k(ranked_ids: List, relevant_ids: set, k: int = 10) -> float:
    dcg = 0.0
    for rank, doc_id in enumerate(ranked_ids[:k], start=1):
        if doc_id in relevant_ids:
            dcg += 1.0 / math.log2(rank + 1)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(min(len(relevant_ids), k)))
    return dcg / idcg if idcg > 0 else 0.0


def recall_at_k(ranked_ids: List, relevant_ids: set, k: int = 10) -> float:
    hits = sum(1 for d in ranked_ids[:k] if d in relevant_ids)
    return hits / len(relevant_ids) if relevant_ids else 0.0


def mrr(ranked_ids: List, relevant_ids: set) -> float:
    for rank, doc_id in enumerate(ranked_ids, start=1):
        if doc_id in relevant_ids:
            return 1.0 / rank
    return 0.0


# ---------------------------------------------------------------------------
# MDN 코퍼스 로딩 (프론트매터/마크다운 마크업 제거)
# ---------------------------------------------------------------------------
_FRONTMATTER = re.compile(r"^---\n.*?\n---\n", re.DOTALL)
_KUMASCRIPT = re.compile(r"\{\{.*?\}\}")           # {{LandingPageListSubpages}}
_CODEBLOCK = re.compile(r"```.*?```", re.DOTALL)
_INLINECODE = re.compile(r"`[^`]*`")
_MD_LINK = re.compile(r"\[([^\]]*)\]\([^)]*\)")    # [text](url) -> text
_HTML_TAG = re.compile(r"<[^>]+>")
_MD_MARK = re.compile(r"[#>*_\-|=]+")              # 헤더/리스트/표 마커


def clean_mdn(raw: str) -> str:
    t = _FRONTMATTER.sub("", raw)
    t = _CODEBLOCK.sub(" ", t)
    t = _KUMASCRIPT.sub(" ", t)
    t = _INLINECODE.sub(" ", t)
    t = _MD_LINK.sub(r"\1", t)
    t = _HTML_TAG.sub(" ", t)
    t = _MD_MARK.sub(" ", t)
    return " ".join(t.split())


def load_mdn_corpus(limit: int) -> List[str]:
    docs = []
    for p in sorted(MDN_ROOT.rglob("index.md")):
        try:
            body = clean_mdn(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        if len(body) >= 50:          # 너무 짧은(거의 빈) 문서 제외
            docs.append(body)
        if len(docs) >= limit:
            break
    return docs


# ---------------------------------------------------------------------------
# M1: 토큰 일치율
# ---------------------------------------------------------------------------
def token_match_metrics(base_toks: List[List[str]], cand_toks: List[List[str]]) -> Dict:
    """베이스라인 대비 (a) 시퀀스 정확일치율, (b) 토큰 집합 Jaccard 평균."""
    n = len(base_toks)
    exact = 0
    jaccards = []
    # 추가: micro 단위 token-level overlap (집합 기준)
    inter_total = 0
    base_total = 0
    cand_total = 0
    for b, c in zip(base_toks, cand_toks):
        if b == c:
            exact += 1
        bs, cs = set(b), set(c)
        union = bs | cs
        inter = bs & cs
        jaccards.append(len(inter) / len(union) if union else 1.0)
        inter_total += len(inter)
        base_total += len(bs)
        cand_total += len(cs)
    return {
        "n_samples": n,
        "exact_seq_match_rate": round(exact / n, 4) if n else 0.0,
        "mean_jaccard": round(sum(jaccards) / n, 4) if n else 0.0,
        "micro_set_precision": round(inter_total / cand_total, 4) if cand_total else 0.0,
        "micro_set_recall": round(inter_total / base_total, 4) if base_total else 0.0,
    }


# ---------------------------------------------------------------------------
# BM25 (phase1 방식: IDF dict + dot product scoring, 순수 Python)
# ---------------------------------------------------------------------------
class SimpleBM25:
    def __init__(self, tokenize_batch: Callable[[List[str]], List[List[str]]],
                 k: float = 1.2, b: float = 0.75):
        self.tokenize_batch = tokenize_batch
        self.k = k
        self.b = b
        self.idf: Dict[str, float] = {}
        self.avgdl = 0.0

    def fit(self, doc_token_lists: List[List[str]]):
        df = defaultdict(int)
        total_len = 0
        for toks in doc_token_lists:
            total_len += len(toks)
            for t in set(toks):
                df[t] += 1
        n = len(doc_token_lists)
        self.avgdl = total_len / n if n else 0.0
        for t, d in df.items():
            self.idf[t] = math.log((n - d + 0.5) / (d + 0.5) + 1)

    def doc_vector(self, toks: List[str]) -> Dict[str, float]:
        tf = defaultdict(int)
        for t in toks:
            tf[t] += 1
        dl = len(toks)
        vec = {}
        for t, f in tf.items():
            idf = self.idf.get(t, 0.0)
            if idf == 0.0:
                continue
            denom = f + self.k * (1 - self.b + self.b * dl / self.avgdl) if self.avgdl else 1.0
            vec[t] = idf * (f * (self.k + 1)) / denom
        return vec

    def query_terms(self, toks: List[str]) -> set:
        return {t for t in toks if t in self.idf}


def eval_bm25(tokenize_batch, docs: List[Dict], queries: List[Dict], k: int = 10) -> Dict:
    doc_texts = [d["text"] for d in docs]
    doc_ids = [str(d["id"]) for d in docs]

    t0 = time.perf_counter()
    doc_tok = tokenize_batch(doc_texts)
    tok_time = time.perf_counter() - t0

    bm25 = SimpleBM25(tokenize_batch)
    bm25.fit(doc_tok)
    doc_vecs = [bm25.doc_vector(toks) for toks in doc_tok]
    vocab = set()
    for toks in doc_tok:
        vocab.update(toks)

    q_texts = [q["text"] for q in queries]
    q_tok = tokenize_batch(q_texts)

    ndcgs, recalls, mrrs = [], [], []
    for q, qt in zip(queries, q_tok):
        rel = set(str(r) for r in q.get("relevant_ids", []))
        if not rel:
            continue
        qterms = bm25.query_terms(qt)
        scored = []
        for did, dv in zip(doc_ids, doc_vecs):
            s = sum(dv.get(t, 0.0) for t in qterms)
            scored.append((s, did))
        scored.sort(key=lambda x: -x[0])
        ranked = [did for _, did in scored]
        ndcgs.append(ndcg_at_k(ranked, rel, k))
        recalls.append(recall_at_k(ranked, rel, k))
        mrrs.append(mrr(ranked, rel))

    def mean(xs): return sum(xs) / len(xs) if xs else 0.0
    return {
        "n_docs": len(docs),
        "n_queries": len(ndcgs),
        "vocab_size": len(vocab),
        "ndcg_at_10": round(mean(ndcgs), 4),
        "recall_at_10": round(mean(recalls), 4),
        "mrr": round(mean(mrrs), 4),
        "tokenize_time_sec": round(tok_time, 2),
    }


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
def main():
    print("[bakeoff] 토크나이저 초기화...")
    baseline = MecabKoBaseline()
    tokenizers = [("python-mecab-ko (baseline)", baseline)]
    build_status = {"python-mecab-ko (baseline)": "OK (golden)"}

    for label, ctor in [
        ("hephaex-mecab-ko (sejong OFF)", lambda: HephaexMecabKo(sejong=False)),
        ("hephaex-mecab-ko (sejong ON)", lambda: HephaexMecabKo(sejong=True)),
        ("lindera (ko-dic)", lambda: Lindera()),
    ]:
        try:
            tokenizers.append((label, ctor()))
            build_status[label] = "OK"
        except Exception as e:
            build_status[label] = f"FAIL: {e}"
            print(f"  [{label}] init FAIL: {e}")

    # ---- 코퍼스 로딩 ----
    print("[bakeoff] 데이터 로딩...")
    miracl_docs = json.load(open(MIRACL_DOCS))
    miracl_queries = json.load(open(MIRACL_QUERIES))
    print(f"  MIRACL: {len(miracl_docs)} docs, {len(miracl_queries)} queries")

    mdn_docs = load_mdn_corpus(N_MATCH_SAMPLE)
    print(f"  MDN: {len(mdn_docs)} docs (cleaned)")

    # M1 샘플 텍스트
    miracl_sample = [d["text"] for d in miracl_docs[:N_MATCH_SAMPLE]]
    mdn_sample = mdn_docs[:N_MATCH_SAMPLE]

    # baseline 토큰 (M1 기준)
    print("[bakeoff] M1: baseline 토큰화...")
    base_miracl = baseline.tokenize_batch(miracl_sample)
    base_mdn = baseline.tokenize_batch(mdn_sample)

    results = {"build_status": build_status, "tokenizers": {}}

    for label, tok in tokenizers:
        print(f"\n=== {label} ===")
        entry = {"name": label}

        # M1: 토큰 일치율
        if label.startswith("python-mecab-ko"):
            entry["m1_miracl"] = {"exact_seq_match_rate": 1.0, "mean_jaccard": 1.0,
                                  "micro_set_precision": 1.0, "micro_set_recall": 1.0,
                                  "n_samples": len(miracl_sample), "note": "self (baseline)"}
            entry["m1_mdn"] = {"exact_seq_match_rate": 1.0, "mean_jaccard": 1.0,
                               "micro_set_precision": 1.0, "micro_set_recall": 1.0,
                               "n_samples": len(mdn_sample), "note": "self (baseline)"}
        else:
            cand_miracl = tok.tokenize_batch(miracl_sample)
            cand_mdn = tok.tokenize_batch(mdn_sample)
            entry["m1_miracl"] = token_match_metrics(base_miracl, cand_miracl)
            entry["m1_mdn"] = token_match_metrics(base_mdn, cand_mdn)
            print(f"  M1 MIRACL: exact={entry['m1_miracl']['exact_seq_match_rate']:.3f} "
                  f"jaccard={entry['m1_miracl']['mean_jaccard']:.3f}")
            print(f"  M1 MDN   : exact={entry['m1_mdn']['exact_seq_match_rate']:.3f} "
                  f"jaccard={entry['m1_mdn']['mean_jaccard']:.3f}")

        # M2: BM25 NDCG on MIRACL (full 1000 docs, 213 queries)
        print(f"  M2: BM25 NDCG@10 on MIRACL ({len(miracl_docs)} docs)...", flush=True)
        m2 = eval_bm25(tok.tokenize_batch, miracl_docs, miracl_queries, k=10)
        entry["m2_miracl_bm25"] = m2
        print(f"  M2 NDCG@10={m2['ndcg_at_10']:.4f} R@10={m2['recall_at_10']:.4f} "
              f"MRR={m2['mrr']:.4f} vocab={m2['vocab_size']}")

        results["tokenizers"][label] = entry

    # 저장
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    json_path = OUT_DIR / "bakeoff_results.json"
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(results, f, ensure_ascii=False, indent=2)
    print(f"\n[bakeoff] 결과 저장: {json_path}")
    return results


if __name__ == "__main__":
    main()
