# pg_glot_hybrid — 원클릭 이미지.
# postgres:17 베이스에 전체 스택(pgvector + pg_textsearch + pg_glot +
# pg_glot_hybrid)을 구워넣고 pg_textsearch를 preload한다.
#
#   docker build -f docker/Dockerfile -t pg_glot_hybrid:17 .
#   docker run -d -e POSTGRES_PASSWORD=pw -p 5432:5432 pg_glot_hybrid:17
#   psql ... -c "CREATE EXTENSION pg_glot_hybrid CASCADE;"   # 첫 init 시 자동 실행됨
#
# 재현성/공급망(리뷰 반영):
#   - 베이스 이미지를 **digest로 핀** → 빌드/런타임 스테이지가 동일한 PG(17.10)를
#     공유하므로 확장 .so의 ABI가 항상 일치(예전 segfault의 근본 차단).
#   - apt 패키지(server-dev, pgvector)와 Rust toolchain을 **버전 핀**.
#   - 의존성 소스는 핀(커밋/태그/버전). 라이선스 고지: THIRD_PARTY_LICENSES/.
# syntax=docker/dockerfile:1
ARG PG_MAJOR=17
# postgres:17 (manifest digest, =17.10-1.pgdg13+1). 갱신 시 PG_APT_VER도 함께 변경.
ARG PG_DIGEST=sha256:2203e6282d9e7de7c24d7da234e2a744fb325df366a3fd8ed940e8abbee39527

# ── build stage ──────────────────────────────────────────────────────────────
FROM postgres:${PG_MAJOR}@${PG_DIGEST} AS build
ARG PG_MAJOR
ARG PGTS_REF=76ea737a5e9a3ae79f4ea8b2028163f8e80e9406
ARG PG_APT_VER=17.10-1.pgdg13+1
ARG RUST_VER=1.95.0
ENV DEBIAN_FRONTEND=noninteractive

# 빌드 도구 + server-dev(베이스 PG와 동일 버전으로 핀 → ABI 일치).
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential git ca-certificates curl pkg-config clang libclang-dev \
        postgresql-server-dev-${PG_MAJOR}=${PG_APT_VER} \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain(핀) + cargo-pgrx(핀).
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain ${RUST_VER} --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo install cargo-pgrx --version 0.18.0 --locked
RUN cargo pgrx init --pg${PG_MAJOR} "$(which pg_config)"

# pg_textsearch (BM25 engine, 핀 커밋). pgvector는 런타임 apt 공식 패키지.
RUN git clone https://github.com/timescale/pg_textsearch /tmp/pgts \
    && git -C /tmp/pgts checkout --quiet ${PGTS_REF} \
    && make -C /tmp/pgts install

# 우리 확장 2개 (pgrx). lindera가 ko-dic을 임베드하므로 빌드가 무겁다.
WORKDIR /src
COPY . /src
RUN cd extensions/pg_glot && cargo pgrx install --release --pg-config "$(which pg_config)" \
 && cd /src/extensions/pg_glot_hybrid && cargo pgrx install --release --pg-config "$(which pg_config)"

# ── runtime stage ────────────────────────────────────────────────────────────
FROM postgres:${PG_MAJOR}@${PG_DIGEST}
ARG PG_MAJOR
ARG PGVECTOR_APT_VER=0.8.2-1.pgdg13+1
ENV DEBIAN_FRONTEND=noninteractive

# pgvector(dense leg) — 공식 apt 패키지, 버전 핀. 베이스 PG는 digest로 17.10 고정.
RUN apt-get update && apt-get install -y --no-install-recommends \
        postgresql-${PG_MAJOR}-pgvector=${PGVECTOR_APT_VER} \
    && rm -rf /var/lib/apt/lists/*

# 확장 산출물만 선택 복사 (PG 코어 라이브러리는 절대 덮지 않는다).
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_textsearch.so     /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_glot.so    /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_glot_hybrid.so  /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_textsearch*    /usr/share/postgresql/${PG_MAJOR}/extension/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_glot*   /usr/share/postgresql/${PG_MAJOR}/extension/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_glot_hybrid* /usr/share/postgresql/${PG_MAJOR}/extension/

# 새 클러스터마다 pg_textsearch를 preload(initdb가 conf.sample을 복사).
RUN echo "shared_preload_libraries = 'pg_textsearch'" \
        >> /usr/share/postgresql/${PG_MAJOR}/postgresql.conf.sample

# 번들 배포 고지(THIRD_PARTY_LICENSES + NOTICE/LICENSE). RUST_CRATES.html 포함.
COPY THIRD_PARTY_LICENSES/ /usr/share/doc/pg_glot_hybrid/THIRD_PARTY_LICENSES/
COPY NOTICE LICENSE        /usr/share/doc/pg_glot_hybrid/

# 첫 init 시 *기본 init DB*에만 전체 스택을 자동 생성(편의). 추가 DB는 수동 생성 필요.
COPY docker/initdb/10-create-extension.sql /docker-entrypoint-initdb.d/
