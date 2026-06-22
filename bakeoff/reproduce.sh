#!/usr/bin/env bash
# bakeoff/reproduce.sh
# 한국어 형태소 분석기 bake-off 전체 재현 스크립트.
#
# 전제 (이 머신에서 이미 충족된 환경):
#   - venv python: /Users/jaesolshin/Documents/GitHub/textsearch/.venv/bin/python
#     (python-mecab-ko + mecab_ko_dic 설치됨)
#   - Rust/cargo 1.95
#   - hephaex clone: /tmp/hephaex_mecab_ko  (rust 워크스페이스: 그 안 rust/)
#   - MIRACL 데이터: /Users/jaesolshin/Documents/GitHub/textsearch/data/miracl/
#   - MDN 코퍼스: /tmp/mdn_clone/files/ko/**/index.md
set -euo pipefail

VENV_PY=/Users/jaesolshin/Documents/GitHub/textsearch/.venv/bin/python
HEPHAEX=/tmp/hephaex_mecab_ko
BAKEOFF_DIR=/Users/jaesolshin/Documents/GitHub/pg_textsearch_ko/bakeoff

echo "==> [1/4] lindera-cli 설치 (ko-dic 임베딩)"
# lindera 3.x: feature 이름은 embed-ko-dic (구버전 ko-dic 아님)
if [ ! -x "$HOME/.cargo/bin/lindera" ]; then
  cargo install lindera-cli --no-default-features --features=embed-ko-dic
fi
echo "한국어 형태소 분석 테스트입니다" | "$HOME/.cargo/bin/lindera" tokenize -d embedded://ko-dic -o wakati

echo "==> [2/4] hephaex CLI 빌드"
( cd "$HEPHAEX/rust" && cargo build --release -p mecab-ko-cli )

echo "==> [3/4] hephaex 용 mecab-ko-dic 바이너리 사전 빌드"
# python-mecab-ko의 컴파일된 sys.dic은 원조 MeCab(C) 포맷이라 hephaex(yada/rkyv)와 비호환.
# 따라서 동일 계열 원본(mecab-ko-dic-2.1.1-20180720)을 hephaex builder로 재컴파일한다.
if [ ! -d "$HEPHAEX/data/mecab-ko-dic-2.1.1-20180720" ]; then
  ( cd "$HEPHAEX/data" && \
    curl -sLO https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz && \
    tar xzf mecab-ko-dic-2.1.1-20180720.tar.gz )
fi
if [ ! -f "$HEPHAEX/data/dict-output/sys.dic.zst" ]; then
  ( cd "$HEPHAEX/rust" && \
    cargo run --release -p mecab-ko-dict-builder -- build \
      --input "$HEPHAEX/data/mecab-ko-dic-2.1.1-20180720" \
      --output "$HEPHAEX/data/dict-output" )
fi
echo "한국어 형태소 분석 테스트입니다" | \
  "$HEPHAEX/rust/target/release/mecab" -d "$HEPHAEX/data/dict-output" -O wakati -q

echo "==> [4/4] bake-off 측정 실행"
cd "$BAKEOFF_DIR"
"$VENV_PY" run_bakeoff.py

echo "==> 완료. 결과: $BAKEOFF_DIR/bakeoff_results.json, REPORT.md"
