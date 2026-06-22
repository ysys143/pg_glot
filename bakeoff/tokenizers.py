"""
bakeoff/tokenizers.py

3개 한국어 형태소 분석기를 동일한 인터페이스(text -> List[str], 표층형 시퀀스)로 노출한다.

  1. python-mecab-ko (baseline, golden)  : `from mecab import MeCab`
  2. hephaex/mecab-ko (pure Rust)        : CLI subprocess (mecab -O wakati)
  3. lindera (pure Rust, ko-dic)         : CLI subprocess (lindera tokenize -o wakati)

Rust 토크나이저는 배치(여러 문서를 한 번에 stdin으로) 호출해 subprocess 오버헤드를 줄인다.
각 문서는 개행을 공백으로 치환하여 "한 문서 = 한 라인"으로 매핑한다(line-by-line 모드).

토큰 정의 통일: 비교는 표층형(surface) 시퀀스 기준. 품사 필터는 적용하지 않는다
(python-mecab-ko의 .morphs()가 표층형 전체를 반환하므로 이에 맞춘다).
"""
from __future__ import annotations

import subprocess
from pathlib import Path
from typing import List

# --- 경로 상수 ---
HEPHAEX_BIN = "/tmp/hephaex_mecab_ko/rust/target/release/mecab"
HEPHAEX_DICDIR = "/tmp/hephaex_mecab_ko/data/dict-output"  # builder 출력 (full ko-dic)
LINDERA_BIN = str(Path.home() / ".cargo/bin/lindera")
LINDERA_DICT = "embedded://ko-dic"


def _normalize_line(text: str) -> str:
    """문서를 한 라인으로: 개행/탭/연속 공백을 단일 공백으로."""
    return " ".join(text.split())


# ---------------------------------------------------------------------------
# 1. python-mecab-ko (baseline)
# ---------------------------------------------------------------------------

class MecabKoBaseline:
    name = "python-mecab-ko"

    def __init__(self):
        from mecab import MeCab
        self._m = MeCab()

    def tokenize(self, text: str) -> List[str]:
        return self._m.morphs(text)

    def tokenize_batch(self, texts: List[str]) -> List[List[str]]:
        return [self._m.morphs(t) for t in texts]


# ---------------------------------------------------------------------------
# 2. hephaex/mecab-ko (Rust CLI)
# ---------------------------------------------------------------------------

class HephaexMecabKo:
    """hephaex CLI 기반. sejong 모드 ON/OFF 선택 가능."""

    def __init__(self, sejong: bool = False, dicdir: str = HEPHAEX_DICDIR):
        self.sejong = sejong
        self.dicdir = dicdir
        self.name = "hephaex-mecab-ko" + ("-sejong" if sejong else "")
        if not Path(HEPHAEX_BIN).exists():
            raise RuntimeError(f"hephaex binary not found: {HEPHAEX_BIN}")

    def _args(self):
        # 주의: --sejong 은 -O wakati 를 무시하고 TSV(surface\tpos)+EOS 형식으로 출력한다(CLI 동작).
        # 따라서 sejong 모드는 EOS 구분 파싱을 사용한다.
        if self.sejong:
            a = [HEPHAEX_BIN, "-q", "--sejong"]
        else:
            a = [HEPHAEX_BIN, "-O", "wakati", "-q"]
        if self.dicdir:
            a += ["-d", self.dicdir]
        return a

    def tokenize_batch(self, texts: List[str]) -> List[List[str]]:
        lines = [_normalize_line(t) for t in texts]
        stdin = "\n".join(lines) + "\n"
        proc = subprocess.run(
            self._args(), input=stdin, capture_output=True, text=True
        )
        if self.sejong:
            return self._parse_eos(proc.stdout, len(lines))
        out_lines = proc.stdout.split("\n")
        if out_lines and out_lines[-1] == "":
            out_lines = out_lines[:-1]
        result = []
        for i in range(len(lines)):
            if i < len(out_lines):
                result.append(out_lines[i].split())
            else:
                result.append([])
        return result

    @staticmethod
    def _parse_eos(stdout: str, n_inputs: int) -> List[List[str]]:
        """TSV(surface\\tpos) 블록을 EOS 기준으로 문서 단위로 묶어 표층형 시퀀스 반환."""
        docs: List[List[str]] = []
        cur: List[str] = []
        for line in stdout.split("\n"):
            line = line.rstrip("\n")
            if line == "EOS":
                docs.append(cur)
                cur = []
            elif line.strip() == "":
                continue
            else:
                surface = line.split("\t", 1)[0]
                if surface:
                    cur.append(surface)
        if cur:
            docs.append(cur)
        # 입력 수에 맞춰 정합
        while len(docs) < n_inputs:
            docs.append([])
        return docs[:n_inputs]

    def tokenize(self, text: str) -> List[str]:
        return self.tokenize_batch([text])[0]


# ---------------------------------------------------------------------------
# 3. lindera (Rust CLI, ko-dic)
# ---------------------------------------------------------------------------

class Lindera:
    name = "lindera-ko-dic"

    def __init__(self):
        if not Path(LINDERA_BIN).exists():
            raise RuntimeError(f"lindera binary not found: {LINDERA_BIN}")

    def tokenize_batch(self, texts: List[str]) -> List[List[str]]:
        lines = [_normalize_line(t) for t in texts]
        stdin = "\n".join(lines) + "\n"
        proc = subprocess.run(
            [LINDERA_BIN, "tokenize", "-d", LINDERA_DICT, "-o", "wakati"],
            input=stdin, capture_output=True, text=True,
        )
        out_lines = proc.stdout.split("\n")
        if out_lines and out_lines[-1] == "":
            out_lines = out_lines[:-1]
        result = []
        for i in range(len(lines)):
            if i < len(out_lines):
                result.append(out_lines[i].split())
            else:
                result.append([])
        return result

    def tokenize(self, text: str) -> List[str]:
        return self.tokenize_batch([text])[0]


if __name__ == "__main__":
    sample = ["한국어 형태소 분석 테스트입니다", "우크라이나는 동유럽의 국가이다."]
    print("[baseline]", MecabKoBaseline().tokenize_batch(sample))
    try:
        print("[hephaex ]", HephaexMecabKo(sejong=False).tokenize_batch(sample))
    except Exception as e:
        print("[hephaex ] FAIL:", e)
    try:
        print("[lindera ]", Lindera().tokenize_batch(sample))
    except Exception as e:
        print("[lindera ] FAIL:", e)
