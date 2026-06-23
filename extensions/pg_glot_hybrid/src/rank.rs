use pgrx::prelude::*;

extension_sql!(
    r#"
CREATE FUNCTION glot.rank(
    text_col text,
    vec_col vector,
    q_text text,
    q_vec vector,
    k integer DEFAULT 60,
    per_leg integer DEFAULT 60
) RETURNS double precision
LANGUAGE plpgsql
STABLE
STRICT
PARALLEL RESTRICTED
SET search_path = pg_catalog, pg_temp
AS $func$
DECLARE
    lexical_score double precision;
    dense_score double precision;
BEGIN
    IF k <= 0 THEN
        RAISE EXCEPTION 'glot.rank: k must be positive (got %)', k;
    END IF;
    IF per_leg <= 0 THEN
        RAISE EXCEPTION 'glot.rank: per_leg must be positive (got %)', per_leg;
    END IF;

    lexical_score := ts_rank_cd(
        to_tsvector('public.korean'::regconfig, text_col),
        plainto_tsquery('public.korean'::regconfig, q_text)
    );
    dense_score := 1.0 / (1.0 + (vec_col OPERATOR(public.<=>) q_vec));

    RETURN lexical_score + dense_score;
END;
$func$;

COMMENT ON FUNCTION glot.rank(text,vector,text,vector,integer,integer)
    IS 'Hybrid rank marker for ORDER BY ... DESC LIMIT; CustomScan uses BM25+dense RRF for supported shapes, with a SQL fallback for ordinary expression evaluation';
"#,
    name = "rank",
);

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn setup_rank_table() {
        Spi::run(
            "CREATE TEMP TABLE rank_docs(id bigint primary key, body text NOT NULL, emb vector(3) NOT NULL); \
             INSERT INTO rank_docs VALUES \
               (1,'한국어 형태소 분석 테스트','[1,0,0]'), \
               (2,'서울 맛집 추천 정보','[0,1,0]'), \
               (3,'형태소 분석기 비교 연구','[0.7,0.3,0]'); \
             CREATE INDEX rank_docs_bm25 ON rank_docs \
                 USING bm25(body) WITH (text_config='public.korean');",
        )
        .expect("rank 테스트 테이블 셋업 실패");
    }

    #[pg_test]
    fn rank_orders_by_text_and_vector_relevance() {
        setup_rank_table();
        let top = Spi::get_one::<i64>(
            "SELECT id FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC, id \
             LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 1, "텍스트와 벡터가 모두 강한 doc 1이 최상위여야 함");
    }

    #[pg_test]
    fn rank_order_by_uses_custom_scan() {
        setup_rank_table();
        let plan = Spi::explain(
            "SELECT id, body FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC \
             LIMIT 3",
        )
        .expect("explain")
        .0
        .to_string();

        assert!(
            plan.contains("\"Node Type\":\"Custom Scan\"")
                && plan.contains("\"Custom Plan Provider\":\"GlotHybrid\""),
            "plan did not use GlotHybrid CustomScan: {plan}"
        );
    }

    #[pg_test]
    fn rank_custom_scan_matches_hybrid_order() {
        setup_rank_table();
        let rank_ids = collect_ids(
            "SELECT id FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC \
             LIMIT 3",
        );
        let hybrid_ids = collect_ids(
            "SELECT id \
             FROM glot.hybrid('rank_docs'::regclass, 'id', 'body', 'emb', \
                              '형태소', '[1,0,0]'::vector, 60, 60, 3) \
             ORDER BY score DESC, id",
        );

        assert_eq!(rank_ids, hybrid_ids);
    }

    /// 폴백 가드: CustomScan이 적용되지 않는 쿼리형태(LIMIT 없는 ORDER BY)에서는
    /// `try_build_path_config`가 `None`을 반환해 일반 planner 경로로 빠지고,
    /// `glot.rank`가 plpgsql 마커로 평가되어도 순위가 정상이어야 한다. preload
    /// hook이 없을 때의 사일런트 폴백과 동일한 비-RRF 평가 경로다.
    #[pg_test]
    fn rank_without_limit_falls_back_safely() {
        setup_rank_table();
        let plan = Spi::explain(
            "SELECT id FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC, id",
        )
        .expect("explain")
        .0
        .to_string();
        assert!(
            !plan.contains("\"Custom Plan Provider\":\"GlotHybrid\""),
            "LIMIT 없는 쿼리는 GlotHybrid CustomScan을 쓰지 않아야 함: {plan}"
        );

        let top = Spi::get_one::<i64>(
            "SELECT id FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC, id \
             LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 1, "폴백 평가 경로에서도 doc 1이 최상위여야 함");
    }

    /// `ExplainCustomScan`: EXPLAIN이 GlotHybrid 스캔의 내부 후보 SQL(BM25+dense
    /// 융합을 수행하는 `glot.hybrid` 호출)을 `Hybrid Query` 속성으로 노출한다.
    #[pg_test]
    fn rank_custom_scan_explains_hybrid_query() {
        setup_rank_table();
        let plan = Spi::explain(
            "SELECT id, body FROM rank_docs \
             ORDER BY glot.rank(body, emb, '형태소', '[1,0,0]'::vector) DESC \
             LIMIT 3",
        )
        .expect("explain")
        .0
        .to_string();
        assert!(
            plan.contains("Hybrid Query") && plan.contains("glot.hybrid"),
            "EXPLAIN이 custom scan의 hybrid 후보 SQL을 노출해야 함: {plan}"
        );
    }

    fn collect_ids(sql: &str) -> Vec<i64> {
        Spi::connect(|client| {
            client
                .select(sql, None, &[])
                .expect("select")
                .map(|row| {
                    row.get_by_name::<i64, _>("id")
                        .expect("id")
                        .expect("id null")
                })
                .collect()
        })
    }
}
