#!/usr/bin/env python3
"""lindera CLI로 corpus/queries를 사전 토큰화 → 공백 join → PG `simple` config로 색인하기
위한 데이터 생성(측정 전용). 내장 `japanese`/`korean` parser와 **같은 lindera 3.0.7 + 사전 +
Mode::Normal**를 쓰므로(분절 동일), production(custom parser) vs pretokenized+simple 갭에서
"분절"을 제거하고 나머지(필터 등) 차이만 분리하기 위한 장치.

내장 POS 필터와 동일한 규칙을 적용한다(crates/glot-tokenizer/src/lib.rs is_content_pos):
- ja(ipadic): denylist = 助詞 助動詞 記号 フィラー 感動詞 接続詞 제외
- ko(ko-dic): accept-list = NNG NNP NNB NNBC NR VV VA MM MAG XSN XR SH SL
복합 태그는 첫 형태소 기준.

추가로, **denylist는 통과하지만 영숫자 글자가 0개인 토큰**을 집계한다 — 이것이 custom
parser의 `surface.chars().any(char::is_alphanumeric)`(extensions/pg_glot/src/lib.rs:90)가
pretokenized 경로 대비 **추가로** blank 처리하는 토큰이다(갭 진단용).

주의: 측정 전용. 제품 코드 아님(제품 토큰화는 내장 lindera). 사용:
  python3 bench/pretokenize_lindera.py ja bench/data/ja bench/data/ja_pretok
"""
from __future__ import annotations

import json
import subprocess
import sys

DICT = {"ja": "embedded://ipadic", "ko": "embedded://ko-dic", "zh": "embedded://cc-cedict"}
JA_DENY = {"助詞", "助動詞", "記号", "フィラー", "感動詞", "接続詞"}
KO_ACCEPT = {
    "NNG", "NNP", "NNB", "NNBC", "NR", "VV", "VA", "MM", "MAG", "XSN", "XR", "SH", "SL",
}


def keep(lang: str, pos: str) -> bool:
    """내장 is_content_pos와 동일 판정. pos 없음(UNK/공란)은 content로 취급(None=>true)."""
    if lang == "ja":
        return pos not in JA_DENY
    if lang == "ko":
        return pos.split("+")[0] in KO_ACCEPT
    return True  # zh: cc-cedict POS 미제공 → 전부 색인


def tokenize_all(lang: str, texts: list[str]) -> list[list[tuple[str, str]]]:
    """lindera CLI 1회 호출로 전체 토큰화. 반환: 문서별 [(surface, pos_major)]."""
    out = subprocess.run(
        ["lindera", "tokenize", "-d", DICT[lang]],
        input="\n".join(t.replace("\n", " ") for t in texts),
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    docs: list[list[tuple[str, str]]] = []
    cur: list[tuple[str, str]] = []
    for line in out.split("\n"):
        if line == "EOS":
            docs.append(cur)
            cur = []
        elif "\t" in line:
            s, rest = line.split("\t", 1)
            pos = rest.split(",")[0]  # 첫 detail = 대분류 POS (UNK면 그대로)
            cur.append((s, pos))
    return docs


def main() -> None:
    lang = sys.argv[1]
    in_dir = sys.argv[2]
    out_dir = sys.argv[3]
    import os

    os.makedirs(out_dir, exist_ok=True)

    n_kept = 0
    n_nonalnum = 0  # denylist 통과 + 영숫자 0개 (= custom parser가 추가로 떨구는 것)
    nonalnum_surf: dict[str, int] = {}

    for split, key in (("corpus", "text"), ("queries", "text")):
        data = json.load(open(f"{in_dir}/{split}.json", encoding="utf-8"))
        docs = tokenize_all(lang, [d[key] for d in data])
        assert len(docs) == len(data), f"{split}: {len(docs)} != {len(data)}"
        for d, toks in zip(data, docs):
            kept = []
            for s, pos in toks:
                if not keep(lang, pos):
                    continue
                kept.append(s)
                n_kept += 1
                if not any(c.isalnum() for c in s):
                    n_nonalnum += 1
                    nonalnum_surf[s] = nonalnum_surf.get(s, 0) + 1
            d[key] = " ".join(kept)
        json.dump(data, open(f"{out_dir}/{split}.json", "w", encoding="utf-8"), ensure_ascii=False)
        print(f"{split}: {len(data)} docs/queries")

    print(f"\n[diag] denylist-kept tokens: {n_kept}")
    print(f"[diag] of which non-alphanumeric (custom parser가 추가 blank): {n_nonalnum} "
          f"({100 * n_nonalnum / max(n_kept, 1):.3f}%)")
    top = sorted(nonalnum_surf.items(), key=lambda x: -x[1])[:25]
    print(f"[diag] distinct non-alnum surfaces: {len(nonalnum_surf)}  top: {top}")


if __name__ == "__main__":
    main()
