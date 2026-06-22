#!/usr/bin/env bash
# pg_textsearch_ko 통합 설치 — 의존성(pgvector, pg_textsearch)과 우리 확장 2개
# (pg_tsvector_ko, pg_textsearch_ko)를 기존 PostgreSQL에 한 번에 빌드/설치한다.
#
# pg_textsearch/pgvector 소스는 각자의 공식 레포에서 *빌드 시 다운로드*하며
# (재배포 아님, 버전 핀), 우리 확장은 cargo pgrx로 설치한다. 라이선스: docs/DESIGN.md §6.
#
# 사용:  PG_CONFIG=/opt/homebrew/opt/postgresql@17/bin/pg_config ./scripts/install-all.sh
# 변수:  PG_CONFIG(필수), PGTS_REF, PGVECTOR_REF, REINSTALL=1(의존성 강제 재설치)
set -euo pipefail

PG_CONFIG="${PG_CONFIG:-pg_config}"
PGTS_REPO="${PGTS_REPO:-https://github.com/timescale/pg_textsearch}"
PGTS_REF="${PGTS_REF:-76ea737a5e9a3ae79f4ea8b2028163f8e80e9406}"
PGVECTOR_REPO="${PGVECTOR_REPO:-https://github.com/pgvector/pgvector}"
# Docker 이미지의 apt pgvector(0.8.2)와 마이너 일치. (소스 빌드 vs apt 빌드 차이만 존재)
PGVECTOR_REF="${PGVECTOR_REF:-v0.8.2}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log()  { printf '\n[install-all] %s\n' "$*"; }
die()  { printf '\n[install-all][ERROR] %s\n' "$*" >&2; exit 1; }

# ── 사전 점검 ────────────────────────────────────────────────────────────────
command -v "$PG_CONFIG" >/dev/null 2>&1 || die "pg_config not found — set PG_CONFIG=/path/to/pg_config"
command -v git  >/dev/null 2>&1 || die "git required"
command -v make >/dev/null 2>&1 || die "make required"
command -v cc   >/dev/null 2>&1 || command -v clang >/dev/null 2>&1 || die "a C compiler (cc/clang) required"
command -v cargo >/dev/null 2>&1 || die "cargo (Rust toolchain) required"
cargo pgrx --version >/dev/null 2>&1 || die \
  "cargo-pgrx required: cargo install cargo-pgrx --version 0.18.0 && cargo pgrx init --pg17 \"\$($PG_CONFIG)\""

SHAREDIR="$("$PG_CONFIG" --sharedir)"
log "target:   $("$PG_CONFIG" --version)"
log "sharedir: $SHAREDIR"

BUILD="$(mktemp -d)"
trap 'rm -rf "$BUILD"' EXIT

# ── 1) pgvector (dense leg) ──────────────────────────────────────────────────
if [ -f "$SHAREDIR/extension/vector.control" ] && [ "${REINSTALL:-0}" != "1" ]; then
  log "pgvector already installed — skip (설치된 버전이 핀 $PGVECTOR_REF와 다를 수 있음; REINSTALL=1로 강제)"
else
  log "building pgvector $PGVECTOR_REF"
  git clone --depth 1 --branch "$PGVECTOR_REF" "$PGVECTOR_REPO" "$BUILD/pgvector"
  make -C "$BUILD/pgvector" install PG_CONFIG="$PG_CONFIG"
fi

# ── 2) pg_textsearch (BM25 engine, 핀 커밋) ──────────────────────────────────
if [ -f "$SHAREDIR/extension/pg_textsearch.control" ] && [ "${REINSTALL:-0}" != "1" ]; then
  log "pg_textsearch already installed — skip (설치본이 핀 ${PGTS_REF:0:12}와 다를 수 있음; REINSTALL=1로 강제)"
else
  log "building pg_textsearch @ ${PGTS_REF:0:12}"
  git clone "$PGTS_REPO" "$BUILD/pgts"
  git -C "$BUILD/pgts" checkout --quiet "$PGTS_REF"
  make -C "$BUILD/pgts" install PG_CONFIG="$PG_CONFIG"
fi

# ── 3) 우리 확장 (pgrx) ──────────────────────────────────────────────────────
for ext in pg_tsvector_ko pg_textsearch_ko; do
  log "installing $ext (cargo pgrx install --release)"
  ( cd "$REPO_ROOT/extensions/$ext" && cargo pgrx install --release --pg-config "$PG_CONFIG" )
done

# ── 마무리 안내 (preload는 서버 재시작이 필요해 수동) ────────────────────────
log "설치 완료. 남은 1회 단계 (수동):"
cat <<'EOF'
  1) postgresql.conf 에 추가 후 PostgreSQL 재시작
       shared_preload_libraries = 'pg_textsearch'
     (기존 preload가 있으면 콤마로 이어붙일 것)
  2) psql 에서 한 줄로 전체 스택 생성
       CREATE EXTENSION pg_textsearch_ko CASCADE;
     -> pg_tsvector_ko + pg_textsearch + vector 가 함께 생성됩니다.
EOF
