//! pg_textsearch_ko — Layer B: 한국어 BM25 + RRF 하이브리드 검색.
//!
//! Layer A(`pg_tsvector_ko`)의 `korean` text search config 위에서 BM25 leg
//! (`pg_textsearch`, `text_config='public.korean'`, `content <@> query`)와 dense leg
//! (`pgvector`, `embedding <=> query`)를 DB-side RRF로 융합한다. 설계: docs/DESIGN.md (D6/D7).
//!
//! 융합 API는 2층(D6):
//!   - `ko_rrf(...)`            — 백엔드/스키마 무관 fusion 프리미티브(`1/(k+rank)` 합산)
//!   - `ko_search_hybrid(...)`  — pgvector 편의 어댑터(BM25+dense leg 실행 후 ko_rrf 융합)
//!
//! 핵심 제약(스모크로 검증됨):
//!   - BM25 인덱스/질의의 `text_config`는 **스키마 한정**(`public.korean`)이어야 한다.
//!     빌드가 별도 워커에서 일어나 search_path가 다르기 때문.
//!   - `pg_textsearch`는 `shared_preload_libraries` 등록 필요 → 테스트는 아래
//!     `pg_test::postgresql_conf_options()`로 주입한다.

::pgrx::pg_module_magic!(name, version);

// 행동(ko_rrf, ko_search_hybrid)은 후속 TDD 커밋에서 추가한다.
// 이 커밋은 구조적 스캐폴딩(확장 로드 + 의존 체인 + 테스트 conf 훅)만 수립한다.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// 스캐폴딩 스모크: Layer B 확장과 requires 의존 체인이 모두 로드된다.
    #[pg_test]
    fn extension_and_deps_load() {
        let n = Spi::get_one::<i64>(
            "SELECT count(*) FROM pg_extension \
             WHERE extname IN ('pg_textsearch_ko','pg_tsvector_ko','pg_textsearch','vector')",
        )
        .expect("spi 실행 실패")
        .expect("결과 null");
        assert_eq!(
            n, 4,
            "4개 확장(B + requires 3종)이 모두 로드되어야 함, 실제 {n}"
        );
    }

    /// 의존 체인이 한국어 BM25를 즉시 사용 가능하게 한다(스키마 한정 config).
    #[pg_test]
    fn korean_bm25_index_builds() {
        Spi::run(
            "CREATE TEMP TABLE smoke_docs(id int primary key, body text); \
             INSERT INTO smoke_docs VALUES (1,'한국어 형태소 분석'),(2,'서울 맛집'); \
             CREATE INDEX smoke_bm25 ON smoke_docs \
                 USING bm25(body) WITH (text_config='public.korean');",
        )
        .expect("korean BM25 인덱스 빌드 실패");
        let hit =
            Spi::get_one::<i32>("SELECT id FROM smoke_docs ORDER BY body <@> '형태소' LIMIT 1")
                .expect("spi")
                .expect("null");
        assert_eq!(hit, 1, "'형태소' 질의는 doc 1을 최상위로 반환해야 함");
    }
}

/// `cargo pgrx test`가 요구하는 모듈.
#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    /// pg_textsearch BM25 access method는 프로세스 시작 시 로드돼야 한다.
    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec!["shared_preload_libraries = 'pg_textsearch'"]
    }
}
