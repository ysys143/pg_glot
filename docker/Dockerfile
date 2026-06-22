# pg_textsearch_ko — 원클릭 이미지.
# postgres:17 베이스에 전체 스택(pgvector + pg_textsearch + pg_tsvector_ko +
# pg_textsearch_ko)을 구워넣고 pg_textsearch를 preload한다.
#
#   docker build -f docker/Dockerfile -t pg_textsearch_ko:17 .
#   docker run -d -e POSTGRES_PASSWORD=pw -p 5432:5432 pg_textsearch_ko:17
#   psql ... -c "CREATE EXTENSION pg_textsearch_ko CASCADE;"   # 첫 init 시 자동 실행됨
#
# 설계 노트(중요):
#   - 빌드/런타임 스테이지의 PostgreSQL minor를 동일하게 맞춘다(ABI 일치). 빌드한
#     확장 .so는 PG ABI에 민감해 minor가 어긋나면 로드 시 segfault.
#   - 런타임에는 *확장 .so만* 선택 복사한다. PG 코어 라이브러리 디렉토리를 통째로
#     덮으면 코어가 손상돼 crash한다.
#   - pgvector는 공식 apt 패키지(검증된 빌드)를 쓴다. pg_textsearch는 apt에 없어 핀
#     커밋으로 소스 빌드한다. 라이선스: THIRD_PARTY_LICENSES/.
# syntax=docker/dockerfile:1
ARG PG_MAJOR=17

# ── build stage ──────────────────────────────────────────────────────────────
FROM postgres:${PG_MAJOR} AS build
ARG PG_MAJOR
ARG PGTS_REF=76ea737a5e9a3ae79f4ea8b2028163f8e80e9406
ENV DEBIAN_FRONTEND=noninteractive

# 빌드 도구 + server-dev. postgresql-${PG_MAJOR}을 함께 명시해 런타임과 같은 최신
# minor로 맞춘다(같은 build invocation = 같은 pgdg 스냅샷).
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential git ca-certificates curl pkg-config clang libclang-dev \
        postgresql-${PG_MAJOR} postgresql-server-dev-${PG_MAJOR} \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo install cargo-pgrx --version 0.18.0 --locked
RUN cargo pgrx init --pg${PG_MAJOR} "$(which pg_config)"

# pg_textsearch (BM25 engine, 핀 커밋).
RUN git clone https://github.com/timescale/pg_textsearch /tmp/pgts \
    && git -C /tmp/pgts checkout --quiet ${PGTS_REF} \
    && make -C /tmp/pgts install

# 우리 확장 2개 (pgrx). lindera가 ko-dic을 임베드하므로 빌드가 무겁다.
WORKDIR /src
COPY . /src
RUN cd extensions/pg_tsvector_ko && cargo pgrx install --release --pg-config "$(which pg_config)" \
 && cd /src/extensions/pg_textsearch_ko && cargo pgrx install --release --pg-config "$(which pg_config)"

# ── runtime stage ────────────────────────────────────────────────────────────
FROM postgres:${PG_MAJOR}
ARG PG_MAJOR
ENV DEBIAN_FRONTEND=noninteractive

# 런타임 PG를 빌드와 동일 minor로 + pgvector 공식 패키지(dense leg).
RUN apt-get update && apt-get install -y --no-install-recommends \
        postgresql-${PG_MAJOR} postgresql-${PG_MAJOR}-pgvector \
    && rm -rf /var/lib/apt/lists/*

# 확장 산출물만 선택 복사 (PG 코어 라이브러리는 절대 덮지 않는다).
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_textsearch.so     /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_tsvector_ko.so    /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/lib/postgresql/${PG_MAJOR}/lib/pg_textsearch_ko.so  /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_textsearch*    /usr/share/postgresql/${PG_MAJOR}/extension/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_tsvector_ko*   /usr/share/postgresql/${PG_MAJOR}/extension/
COPY --from=build /usr/share/postgresql/${PG_MAJOR}/extension/pg_textsearch_ko* /usr/share/postgresql/${PG_MAJOR}/extension/

# 새 클러스터마다 pg_textsearch를 preload(initdb가 conf.sample을 복사).
RUN echo "shared_preload_libraries = 'pg_textsearch'" \
        >> /usr/share/postgresql/${PG_MAJOR}/postgresql.conf.sample

# 번들 배포 고지(THIRD_PARTY_LICENSES + NOTICE/LICENSE).
COPY THIRD_PARTY_LICENSES/ /usr/share/doc/pg_textsearch_ko/THIRD_PARTY_LICENSES/
COPY NOTICE LICENSE        /usr/share/doc/pg_textsearch_ko/

# 첫 init 시 전체 스택을 자동 생성(편의, IF NOT EXISTS).
COPY docker/initdb/10-create-extension.sql /docker-entrypoint-initdb.d/
