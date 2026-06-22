# pg_textsearch_ko — dev targets (pgrx). 설계: docs/DESIGN.md
# 타깃 PG: pg17 (pgrx-managed). 변경: make PG_VER=pgXX ...
PG_VER    ?= pg17
EXT       ?= pg_tsvector_ko
EXT_DIR   := extensions/$(EXT)
# 통합 설치 대상 PostgreSQL (install-all/docker). pgrx-managed가 아닌 실제 클러스터.
PG_CONFIG ?= pg_config
IMAGE     ?= pg_textsearch_ko:17

.PHONY: unit run test schema package fmt lint help install-all docker-build licenses

help:
	@echo "unit        - 순수 Rust 토크나이저 유닛테스트 (PG 불필요, 최속 피드백)"
	@echo "run         - cargo pgrx run $(PG_VER) (psql 진입)         [EXT=$(EXT)]"
	@echo "test        - cargo pgrx test $(PG_VER) (pg_regress+pg_test) [EXT=$(EXT)]"
	@echo "schema      - cargo pgrx schema (생성 SQL 확인)"
	@echo "package     - cargo pgrx package (.so+control+sql 산출)"
	@echo "fmt/lint    - cargo fmt / clippy"
	@echo "install-all - 의존성(pgvector,pg_textsearch)+확장 2개를 기존 PG에 일괄 설치"
	@echo "              [PG_CONFIG=/path/to/pg_config]"
	@echo "docker-build- 원클릭 Docker 이미지 빌드 (postgres:17 + 전체 스택)  [IMAGE=$(IMAGE)]"
	@echo "licenses    - 정적 링크 Rust 크레이트 라이선스 재생성 + cargo-deny 게이트"

unit:
	cargo test -p korean-tokenizer

run:
	cd $(EXT_DIR) && cargo pgrx run $(PG_VER)

test:
	cd $(EXT_DIR) && cargo pgrx test $(PG_VER)

schema:
	cd $(EXT_DIR) && cargo pgrx schema

package:
	cd $(EXT_DIR) && cargo pgrx package

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets --all-features -- -D warnings

# 기존 PostgreSQL에 전체 스택을 일괄 빌드/설치 (의존성은 공식 레포에서 빌드 시 fetch).
install-all:
	PG_CONFIG="$(PG_CONFIG)" bash scripts/install-all.sh

# 진짜 원클릭: postgres:17 베이스에 전체 스택을 구워넣은 이미지.
docker-build:
	docker build -f docker/Dockerfile -t "$(IMAGE)" .

# 번들(.so)에 정적 링크되는 Rust 크레이트 전체의 라이선스/저작권을 박제 + 게이트.
# 의존성 변경 시 재실행해 THIRD_PARTY_LICENSES/RUST_CRATES.html을 갱신할 것.
licenses:
	cargo about generate about.hbs > THIRD_PARTY_LICENSES/RUST_CRATES.html
	cargo deny check licenses
