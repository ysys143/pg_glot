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

use pgrx::prelude::*;
use std::collections::HashMap;

::pgrx::pg_module_magic!(name, version);

/// RRF(Reciprocal Rank Fusion) 프리미티브 — 백엔드/스키마/언어 무관 (D6).
///
/// 각 leg는 **순위순 id 배열**(앞이 1위)이다. 융합 점수 = `Σ_legs 1/(k + rank)`.
/// (id, rank) 리스트만 받으므로 BM25/dense 외 임의의 랭커, 커스텀 스키마, 외부
/// 파이프라인(pg_aidb)에서도 그대로 재사용된다. NULL 요소는 기여하지 않는다.
///
/// 결과는 점수 내림차순(동점은 id 오름차순)으로 결정적 정렬된다.
#[pg_extern(immutable, parallel_safe)]
fn ko_rrf(
    bm25: Vec<Option<i64>>,
    dense: Vec<Option<i64>>,
    k: default!(i32, 60),
) -> TableIterator<'static, (name!(id, i64), name!(score, f64))> {
    if k <= 0 {
        error!("ko_rrf: k must be positive (got {k})");
    }
    let kf = f64::from(k);
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let mut accumulate = |leg: &[Option<i64>]| {
        for (i, id) in leg.iter().enumerate() {
            if let Some(id) = id {
                // rank는 1-based(배열 위치). NULL도 위치는 차지하나 점수엔 불기여.
                *scores.entry(*id).or_insert(0.0) += 1.0 / (kf + (i as f64 + 1.0));
            }
        }
    };
    accumulate(&bm25);
    accumulate(&dense);

    let mut rows: Vec<(i64, f64)> = scores.into_iter().collect();
    rows.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    TableIterator::new(rows)
}

// ko_search_hybrid — pgvector 편의 어댑터 (D6).
//
// BM25 leg(`text_col <@> q_text`)와 dense leg(`vec_col <=> q_vec`)를 각각 인덱스
// top-k로 뽑아 거리순 id 배열을 만들고, ko_rrf로 융합해 상위 n을 돌려준다.
// 동적 SQL이지만 식별자는 `format('%I')`, 테이블은 `regclass`(검증된 OID)로만
// 들어가므로 SQL injection 면역. 권한은 invoker(SECURITY DEFINER 회피, §10).
// `<@>`/`<=>`/`ko_rrf` 해석을 위해 search_path에 public을 포함하되 고정한다.
extension_sql!(
    r#"
CREATE FUNCTION ko_search_hybrid(
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
PARALLEL SAFE
SET search_path = pg_catalog, public, pg_temp
AS $func$
DECLARE
    bm25_ids  bigint[];
    dense_ids bigint[];
BEGIN
    -- BM25 leg: korean config로 색인된 text_col을 q_text로 스코어, 인덱스 top-k.
    -- pg_textsearch `<@>`는 plain ORDER BY+LIMIT(인덱스 스캔)에서만 동작하고,
    -- planner hook은 질의가 **리터럴**일 때만 인덱스를 매칭하므로 q_text는 %L
    -- (quote_literal, injection 안전)로 인라인한다. 순위는 정렬 결과의 순번.
    EXECUTE format(
        'SELECT array_agg(k ORDER BY ord) FROM ('
        '  SELECT k, row_number() OVER () AS ord FROM ('
        '    SELECT (%1$I)::bigint AS k FROM %2$s'
        '    ORDER BY (%3$I <@> %4$L) LIMIT %5$L) t) u',
        key_col, rel::text, text_col, q_text, per_leg
    ) INTO bm25_ids;

    -- dense leg: pgvector 거리, 인덱스 top-k (동일 순위화 패턴).
    EXECUTE format(
        'SELECT array_agg(k ORDER BY ord) FROM ('
        '  SELECT k, row_number() OVER () AS ord FROM ('
        '    SELECT (%1$I)::bigint AS k FROM %2$s'
        '    ORDER BY (%3$I <=> $1) LIMIT $2) t) u',
        key_col, rel::text, vec_col
    ) INTO dense_ids USING q_vec, per_leg;

    RETURN QUERY
        SELECT r.id, r.score
        FROM ko_rrf(
                 COALESCE(bm25_ids,  ARRAY[]::bigint[]),
                 COALESCE(dense_ids, ARRAY[]::bigint[]),
                 k) AS r
        LIMIT n;
END;
$func$;
COMMENT ON FUNCTION ko_search_hybrid(regclass,text,text,text,text,vector,integer,integer,integer)
    IS 'Korean hybrid search: BM25(<@>, korean config) + dense(<=>) fused by ko_rrf';
"#,
    name = "ko_search_hybrid",
    requires = [ko_rrf],
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

    // ── ko_rrf: 백엔드/스키마 무관 fusion 프리미티브 (D6) ──────────────────

    /// 두 leg 모두에 등장하는 id는 양쪽 기여(1/(k+rank))로 최상위 점수를 받는다.
    #[pg_test]
    fn ko_rrf_ranks_shared_id_highest() {
        // bm25 순위: 10,20,30 / dense 순위: 20,40 → id 20만 양쪽 등장
        let top = Spi::get_one::<i64>(
            "SELECT id FROM ko_rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60) \
             ORDER BY score DESC, id LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 20, "양쪽 leg에 등장한 id 20이 최상위여야 함");
    }

    /// 단일 leg, rank=1 → score = 1/(k+1).
    #[pg_test]
    fn ko_rrf_score_formula() {
        let s = Spi::get_one::<f64>(
            "SELECT score FROM ko_rrf(ARRAY[10]::bigint[], ARRAY[]::bigint[], 60)",
        )
        .expect("spi")
        .expect("null");
        assert!((s - 1.0 / 61.0).abs() < 1e-9, "1/(60+1) 기대, 실제 {s}");
    }

    /// 누락 id는 그 leg에 기여하지 않는다(합산 정확성): bm25 rank2 + dense rank1.
    #[pg_test]
    fn ko_rrf_sums_per_leg_ranks() {
        // id 20: bm25 rank2(=1/62) + dense rank1(=1/61)
        let s = Spi::get_one::<f64>(
            "SELECT score FROM ko_rrf(ARRAY[10,20,30]::bigint[], ARRAY[20,40]::bigint[], 60) \
             WHERE id = 20",
        )
        .expect("spi")
        .expect("null");
        let expected = 1.0 / 62.0 + 1.0 / 61.0;
        assert!((s - expected).abs() < 1e-9, "기대 {expected}, 실제 {s}");
    }

    /// 빈 leg 둘 → 0 row.
    #[pg_test]
    fn ko_rrf_empty_legs_yield_no_rows() {
        let n = Spi::get_one::<i64>(
            "SELECT count(*) FROM ko_rrf(ARRAY[]::bigint[], ARRAY[]::bigint[], 60)",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(n, 0);
    }

    /// k 생략 시 기본값 60.
    #[pg_test]
    fn ko_rrf_default_k_is_60() {
        let s =
            Spi::get_one::<f64>("SELECT score FROM ko_rrf(ARRAY[10]::bigint[], ARRAY[]::bigint[])")
                .expect("spi")
                .expect("null");
        assert!(
            (s - 1.0 / 61.0).abs() < 1e-9,
            "기본 k=60 기대, 실제 score {s}"
        );
    }

    // ── ko_search_hybrid: pgvector 편의 어댑터 (D6) ────────────────────────

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
    fn ko_search_hybrid_fuses_bm25_and_dense() {
        setup_hybrid_table();
        let top = Spi::get_one::<i64>(
            "SELECT id FROM ko_search_hybrid('hdocs','id','body','emb','형태소','[1,0,0]'::vector) \
             LIMIT 1",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(top, 1, "BM25+dense 양쪽 강한 doc 1이 최상위여야 함");
    }

    /// 최종 반환 수 n을 존중한다.
    #[pg_test]
    fn ko_search_hybrid_respects_limit() {
        setup_hybrid_table();
        let cnt = Spi::get_one::<i64>(
            "SELECT count(*) FROM ko_search_hybrid('hdocs','id','body','emb','형태소 분석', \
             '[1,0,0]'::vector, 60, 60, 2)",
        )
        .expect("spi")
        .expect("null");
        assert_eq!(cnt, 2, "limit n=2를 존중해야 함");
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
