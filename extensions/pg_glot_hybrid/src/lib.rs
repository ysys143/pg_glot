//! pg_glot_hybrid — Layer B: 한국어 BM25 + RRF 하이브리드 검색.
//!
//! Layer A(`pg_glot`)의 `korean` text search config 위에서 BM25 leg
//! (`pg_textsearch`, `text_config='public.korean'`, `content <@> query`)와 dense leg
//! (`pgvector`, `embedding <=> query`)를 DB-side RRF로 융합한다. 설계: docs/DESIGN.md (D6/D7).
//!
//! 융합 API는 2층(D6):
//!   - `rrf(...)`            — 백엔드/스키마 무관 fusion 프리미티브(`1/(k+rank)` 합산)
//!   - `hybrid(...)`  — pgvector 편의 어댑터(BM25+dense leg 실행 후 rrf 융합)
//!
//! 핵심 제약(스모크로 검증됨):
//!   - BM25 인덱스/질의의 `text_config`는 **스키마 한정**(`public.korean`)이어야 한다.
//!     빌드가 별도 워커에서 일어나 search_path가 다르기 때문.
//!   - `pg_textsearch`는 `shared_preload_libraries` 등록 필요 → 테스트는 아래
//!     `pg_test::postgresql_conf_options()`로 주입한다.

use pgrx::prelude::*;

::pgrx::pg_module_magic!(name, version);

// glot.rrf 융합 프리미티브는 Layer A(pg_glot)가 `glot` 스키마와 함께 소유한다
// (rrf는 언어/백엔드 무관 공통 유틸). 이 확장은 그 위에 glot.hybrid를 더한다.

// hybrid — pgvector 편의 어댑터 (D6).
//
// BM25 leg(`text_col <@> q_text`)와 dense leg(`vec_col <=> q_vec`)를 각각 인덱스
// top-k로 뽑아 거리순 id 배열을 만들고, rrf로 융합해 상위 n을 돌려준다.
// 동적 SQL이지만 식별자는 `format('%I')`, 테이블은 `regclass`(검증된 OID)로만
// 들어가므로 SQL injection 면역. 권한은 invoker(SECURITY DEFINER 회피, §10).
//
// 순위 보존(BLOCKER 회귀 가드): `ORDER BY <@>/<=> ... LIMIT` 스캔의 산출 순서는
// `row_number() OVER ()`(빈 윈도)로 그냥 들어가면 **보존이 보장되지 않는다**
// (subquery의 ORDER BY는 ordering을 소비하지 않는 부모로 살아남지 않으며, ORDER BY
// 없는 WindowAgg는 그 소비자가 아니다). 그래서 각 leg의 LIMIT된 정렬 스캔을
// `WITH t AS MATERIALIZED (...)`로 고정해 윈도 입력 순서를 핀한다. BM25 leg는
// `row_number() OVER (ORDER BY <@>)`로 바꾸면 별도 WindowAgg 정렬이 생겨
// pg_textsearch planner hook(plain ORDER BY+LIMIT+리터럴 질의에서만 인덱스 매칭)을
// 무력화할 수 있으므로 절대 그렇게 하지 않는다.
//
// search_path(§10): `@extschema@.rrf`로 한정해 user의 `public.rrf` shadowing을
// 차단하고, BM25 `<@>`/dense `<=>` 연산자는 `OPERATOR(public.<@>)`/`OPERATOR(public.<=>)`로
// 스키마 한정해 search_path에서 public을 제거(`pg_catalog, pg_temp`)한다.
extension_sql!(
    r#"
CREATE FUNCTION glot.hybrid(
    rel       regclass,
    key_col   text,
    text_col  text,
    vec_col   text,
    q_text    text,
    q_vec     vector,
    k         integer DEFAULT 60,
    per_leg   integer DEFAULT 60,
    n         integer DEFAULT 10
) RETURNS TABLE(id bigint, score double precision)
LANGUAGE plpgsql
STABLE
PARALLEL RESTRICTED
SET search_path = pg_catalog, pg_temp
AS $func$
DECLARE
    bm25_ids  bigint[];
    dense_ids bigint[];
BEGIN
    -- BM25 leg: korean config로 색인된 text_col을 q_text로 스코어, 인덱스 top-k.
    -- pg_textsearch `<@>`는 plain ORDER BY+LIMIT(인덱스 스캔)에서만 동작하고,
    -- planner hook은 질의가 **리터럴**일 때만 인덱스를 매칭하므로 q_text는 %L
    -- (quote_literal, injection 안전)로 인라인한다. 순위는 정렬 결과의 순번.
    -- MATERIALIZED CTE가 LIMIT된 정렬 스캔을 고정해 윈도 입력 순서를 보존한다.
    EXECUTE format(
        'WITH t AS MATERIALIZED ('
        '  SELECT (%1$I)::bigint AS k FROM %2$s'
        '  ORDER BY (%3$I OPERATOR(public.<@>) %4$L) LIMIT %5$L)'
        'SELECT array_agg(k ORDER BY ord) FROM ('
        '  SELECT k, row_number() OVER () AS ord FROM t) u',
        key_col, rel::text, text_col, q_text, per_leg
    ) INTO bm25_ids;

    -- dense leg: pgvector 거리, 인덱스 top-k (동일 순위화 패턴).
    EXECUTE format(
        'WITH t AS MATERIALIZED ('
        '  SELECT (%1$I)::bigint AS k FROM %2$s'
        '  ORDER BY (%3$I OPERATOR(public.<=>) $1) LIMIT $2)'
        'SELECT array_agg(k ORDER BY ord) FROM ('
        '  SELECT k, row_number() OVER () AS ord FROM t) u',
        key_col, rel::text, vec_col
    ) INTO dense_ids USING q_vec, per_leg;

    RETURN QUERY
        SELECT r.id, r.score
        FROM glot.rrf(
                 COALESCE(bm25_ids,  ARRAY[]::bigint[]),
                 COALESCE(dense_ids, ARRAY[]::bigint[]),
                 k) AS r
        LIMIT n;
END;
$func$;
COMMENT ON FUNCTION glot.hybrid(regclass,text,text,text,text,vector,integer,integer,integer)
    IS 'Korean hybrid search: BM25(<@>, korean config) + dense(<=>) fused by rrf';
"#,
    name = "hybrid",
);

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// 스캐폴딩 스모크: Layer B 확장과 requires 의존 체인이 모두 로드된다.
    #[pg_test]
    fn extension_and_deps_load() {
        let n = Spi::get_one::<i64>(
            "SELECT count(*) FROM pg_extension \
             WHERE extname IN ('pg_glot_hybrid','pg_glot','pg_textsearch','vector')",
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

    // ── rrf: 백엔드/스키마 무관 fusion 프리미티브 (D6) ──────────────────

    /// 두 leg 모두에 등장하는 id는 양쪽 기여(1/(k+rank))로 최상위 점수를 받는다.
    #[pg_test]
    fn rrf_ranks_shared_id_highest() {
        // bm25 순위: 10,20,30 / dense 순위: 20,40 → id 20만 양쪽 등장
        let top = Spi::get_one::<i64>(
            "SELECT id FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60) \
             ORDER BY score DESC, id LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 20, "양쪽 leg에 등장한 id 20이 최상위여야 함");
    }

    /// 단일 leg, rank=1 → score = 1/(k+1).
    #[pg_test]
    fn rrf_score_formula() {
        let s = Spi::get_one::<f64>(
            "SELECT score FROM glot.rrf(ARRAY[10]::bigint[], ARRAY[]::bigint[], 60)",
        )
        .expect("spi")
        .expect("null");
        assert!((s - 1.0 / 61.0).abs() < 1e-9, "1/(60+1) 기대, 실제 {s}");
    }

    /// 누락 id는 그 leg에 기여하지 않는다(합산 정확성): bm25 rank2 + dense rank1.
    #[pg_test]
    fn rrf_sums_per_leg_ranks() {
        // id 20: bm25 rank2(=1/62) + dense rank1(=1/61)
        let s = Spi::get_one::<f64>(
            "SELECT score FROM glot.rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60) \
             WHERE id = 20",
        )
        .expect("spi")
        .expect("null");
        let expected = 1.0 / 62.0 + 1.0 / 61.0;
        assert!((s - expected).abs() < 1e-9, "기대 {expected}, 실제 {s}");
    }

    /// 빈 leg 둘 → 0 row.
    #[pg_test]
    fn rrf_empty_legs_yield_no_rows() {
        let n = Spi::get_one::<i64>(
            "SELECT count(*) FROM glot.rrf(ARRAY[]::bigint[], ARRAY[]::bigint[], 60)",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(n, 0);
    }

    /// k 생략 시 기본값 60.
    #[pg_test]
    fn rrf_default_k_is_60() {
        let s = Spi::get_one::<f64>(
            "SELECT score FROM glot.rrf(ARRAY[10]::bigint[], ARRAY[]::bigint[])",
        )
        .expect("spi")
        .expect("null");
        assert!(
            (s - 1.0 / 61.0).abs() < 1e-9,
            "기본 k=60 기대, 실제 score {s}"
        );
    }

    // ── hybrid: pgvector 편의 어댑터 (D6) ────────────────────────

    /// 테스트용 한국어 문서 테이블 + BM25 인덱스 + 벡터.
    fn setup_hybrid_table() {
        Spi::run(
            "CREATE TEMP TABLE hdocs(id bigint primary key, body text, emb vector(3)); \
             INSERT INTO hdocs VALUES \
               (1,'한국어 형태소 분석 테스트','[1,0,0]'), \
               (2,'서울 맛집 추천 정보','[0,1,0]'), \
               (3,'형태소 분석기 비교 연구','[0,0,1]'); \
             CREATE INDEX hdocs_bm25 ON hdocs \
                 USING bm25(body) WITH (text_config='public.korean');",
        )
        .expect("하이브리드 테이블 셋업 실패");
    }

    /// BM25 leg('형태소' → 1,3)와 dense leg([1,0,0] → 1 최근접)를 융합하면
    /// 양쪽에서 강한 doc 1이 최상위가 된다(end-to-end BM25 라운드트립 + 융합).
    #[pg_test]
    fn hybrid_fuses_bm25_and_dense() {
        setup_hybrid_table();
        let top = Spi::get_one::<i64>(
            "SELECT id FROM glot.hybrid('hdocs','id','body','emb','형태소','[1,0,0]'::vector) \
             LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 1, "BM25+dense 양쪽 강한 doc 1이 최상위여야 함");
    }

    /// 융합 결과의 **순위 순서**가 BM25 rank 순서를 정확히 보존한다(집합 멤버십이 아님).
    ///
    /// 회귀 가드(BLOCKER): BM25 leg의 `ORDER BY <@> ... LIMIT` 산출 순서가
    /// `row_number() OVER ()`(빈 윈도)로 들어갈 때 보존되지 않으면 RRF rank가
    /// 뒤섞인다. dense leg를 BM25와 **동일 순서**(doc1>doc3>doc4>doc5)로 정렬되도록
    /// 거리를 단조 분리해 dense가 순서를 강화만 하게 하고(타이 없음), 두 leg가
    /// 일관되게 같은 순서를 산출할 때만 최종이 [1,3,4,5]가 되게 설계한다. 어느 한
    /// leg라도 row_number 스크램블이 생기면 순서가 뒤집힌다.
    #[pg_test]
    fn hybrid_preserves_bm25_rank_order() {
        // '형태소' 빈도: doc1=4, doc3=3, doc4=2, doc5=1 → BM25 rank 1,2,3,4 결정적.
        // doc2: '형태소' 무관(BM25 미히트, dense도 직교로 최하위).
        // dense 거리(query [1,0,0] 기준): doc1<doc3<doc4<doc5<doc2 단조 분리(타이 없음).
        Spi::run(
            "CREATE TEMP TABLE rdocs(id bigint primary key, body text, emb vector(3)); \
             INSERT INTO rdocs VALUES \
               (1,'형태소 형태소 형태소 형태소 분석','[1,0,0]'), \
               (2,'서울 맛집 추천 정보','[0,1,0]'), \
               (3,'형태소 형태소 형태소 자료','[1,0.2,0]'), \
               (4,'형태소 형태소 연구','[1,0.5,0]'), \
               (5,'형태소 검토','[1,0.9,0]'); \
             CREATE INDEX rdocs_bm25 ON rdocs \
                 USING bm25(body) WITH (text_config='public.korean');",
        )
        .expect("회귀 테이블 셋업 실패");
        // 상위 4개를 순서대로 수집. 두 leg가 일관되면 [1,3,4,5].
        // row_number 스크램블이 발생하면 순서가 뒤집히거나 doc2가 끼어든다.
        let ordered = Spi::get_one::<Vec<i64>>(
            "SELECT array_agg(id ORDER BY ord) FROM ( \
               SELECT id, row_number() OVER (ORDER BY score DESC, id) AS ord \
               FROM glot.hybrid('rdocs','id','body','emb','형태소','[1,0,0]'::vector, \
                                      60, 60, 4)) s",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(
            ordered,
            vec![1, 3, 4, 5],
            "융합 상위 순서는 BM25 rank를 보존해 [1,3,4,5]여야 함(실제 {ordered:?})"
        );
    }

    /// 최종 반환 수 n을 존중한다.
    #[pg_test]
    fn hybrid_respects_limit() {
        setup_hybrid_table();
        let cnt = Spi::get_one::<i64>(
            "SELECT count(*) FROM glot.hybrid('hdocs','id','body','emb','형태소 분석', \
             '[1,0,0]'::vector, 60, 60, 2)",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(cnt, 2, "limit n=2를 존중해야 함");
    }

    // ── index/query 일관성: BM25 incremental maintenance + 파서 동일성 (마일스톤 ④) ──

    /// 인덱스 생성 후 INSERT한 행이 BM25 검색에 반영된다.
    #[pg_test]
    fn insert_reflected_in_bm25_search() {
        Spi::run(
            "CREATE TEMP TABLE c(id bigint primary key, body text); \
             INSERT INTO c VALUES (1,'기존 문서 자료'); \
             CREATE INDEX c_bm25 ON c USING bm25(body) WITH (text_config='public.korean'); \
             INSERT INTO c VALUES (2,'신규 검색 항목');",
        )
        .expect("셋업");
        let hit = Spi::get_one::<i64>("SELECT id FROM c ORDER BY body <@> '검색' LIMIT 1")
            .expect("spi")
            .expect("null");
        assert_eq!(hit, 2, "인덱스 생성 후 INSERT한 행이 검색돼야 함");
    }

    /// UPDATE 후 새 토큰으로 검색된다.
    #[pg_test]
    fn update_reflected_in_bm25_search() {
        Spi::run(
            "CREATE TEMP TABLE c(id bigint primary key, body text); \
             INSERT INTO c VALUES (1,'원본 자료 항목'),(2,'무관 문서 내용'); \
             CREATE INDEX c_bm25 ON c USING bm25(body) WITH (text_config='public.korean'); \
             UPDATE c SET body='수정된 검색 품질' WHERE id=1;",
        )
        .expect("셋업");
        let hit = Spi::get_one::<i64>("SELECT id FROM c ORDER BY body <@> '품질' LIMIT 1")
            .expect("spi")
            .expect("null");
        assert_eq!(hit, 1, "UPDATE 후 새 토큰 '품질'로 행 1이 검색돼야 함");
    }

    /// DELETE된 행은 검색 결과에서 빠진다.
    #[pg_test]
    fn delete_excluded_from_bm25_search() {
        Spi::run(
            "CREATE TEMP TABLE c(id bigint primary key, body text); \
             INSERT INTO c VALUES (1,'형태소 분석 자료'),(2,'형태소 검색 항목'); \
             CREATE INDEX c_bm25 ON c USING bm25(body) WITH (text_config='public.korean'); \
             DELETE FROM c WHERE id=1;",
        )
        .expect("셋업");
        let remaining = Spi::get_one::<i64>(
            "SELECT count(*) FROM (SELECT id FROM c ORDER BY body <@> '형태소' LIMIT 10) s \
             WHERE id = 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(remaining, 0, "DELETE된 행은 검색 결과에서 빠져야 함");
    }

    /// 색인(index-time)과 질의(query-time)가 동일한 korean 파서를 쓴다:
    /// 색인 텍스트의 조사 분리 토큰('한국어를'→'한국어')이 질의 '한국어'와 매칭.
    #[pg_test]
    fn index_query_parser_consistency() {
        Spi::run(
            "CREATE TEMP TABLE c(id bigint primary key, body text); \
             INSERT INTO c VALUES (1,'한국어를 분석한다'),(2,'영어 텍스트 문서'); \
             CREATE INDEX c_bm25 ON c USING bm25(body) WITH (text_config='public.korean');",
        )
        .expect("셋업");
        let hit = Spi::get_one::<i64>("SELECT id FROM c ORDER BY body <@> '한국어' LIMIT 1")
            .expect("spi")
            .expect("null");
        assert_eq!(
            hit, 1,
            "조사 분리된 '한국어를'이 질의 '한국어'와 매칭되어야 함(파서 일관성)"
        );
    }

    // ── REINDEX (마일스톤 ⑤) ──────────────────────────────────────────────

    /// REINDEX 후에도 BM25 검색이 유지된다.
    #[pg_test]
    fn reindex_preserves_search() {
        Spi::run(
            "CREATE TEMP TABLE c(id bigint primary key, body text); \
             INSERT INTO c VALUES (1,'형태소 분석 자료'),(2,'서울 맛집 추천'); \
             CREATE INDEX c_bm25 ON c USING bm25(body) WITH (text_config='public.korean'); \
             REINDEX INDEX c_bm25;",
        )
        .expect("REINDEX 실패");
        let hit = Spi::get_one::<i64>("SELECT id FROM c ORDER BY body <@> '형태소' LIMIT 1")
            .expect("spi")
            .expect("null");
        assert_eq!(hit, 1, "REINDEX 후에도 검색이 유지돼야 함");
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
