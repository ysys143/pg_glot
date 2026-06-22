# pg_textsearch_ko — dev targets (pgrx). 설계: docs/DESIGN.md
# 타깃 PG: pg17 (pgrx-managed). 변경: make PG_VER=pgXX ...
PG_VER ?= pg17
EXT    ?= pg_tsvector_ko
EXT_DIR := extensions/$(EXT)

.PHONY: unit run test schema package fmt lint help

help:
	@echo "unit    - 순수 Rust 토크나이저 유닛테스트 (PG 불필요, 최속 피드백)"
	@echo "run     - cargo pgrx run $(PG_VER) (psql 진입)         [EXT=$(EXT)]"
	@echo "test    - cargo pgrx test $(PG_VER) (pg_regress+pg_test) [EXT=$(EXT)]"
	@echo "schema  - cargo pgrx schema (생성 SQL 확인)"
	@echo "package - cargo pgrx package (.so+control+sql 산출)"
	@echo "fmt/lint- cargo fmt / clippy"

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
