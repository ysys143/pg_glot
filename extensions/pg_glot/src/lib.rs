//! pg_glot — PostgreSQL `korean` text search configuration (Layer A).
//!
//! lindera(ko-dic) 기반 커스텀 TS parser를 등록한다. `to_tsvector('korean', …)`,
//! `ts_debug('korean', …)`, 그리고 (Layer B) pg_textsearch BM25(`text_config='korean'`)가
//! 이 config를 소비한다. 설계: docs/DESIGN.md.
//!
//! C-ABI 안전(Codex 리뷰 반영):
//! - 파서 상태는 `Internal::new`로 현재 memory context에 palloc → context 삭제 시 자동 drop
//!   (에러/longjmp 시에도 누수 없음).
//! - 토큰 바이트는 ParserState(컨텍스트 수명) 안의 String을 가리키며, PG가 즉시 복사한다.
//! - 입력은 UTF-8 검증 후 처리(비-UTF8 DB 인코딩이면 ereport).
//! - panic은 pgrx `#[pg_guard]`(매크로가 자동 부착)가 ereport로 변환.

use glot_tokenizer::{Analyzer, LinderaAnalyzer};
use pgrx::prelude::*;
use pgrx::Internal;
use std::os::raw::{c_char, c_int};
use std::sync::OnceLock;

::pgrx::pg_module_magic!(name, version);

/// PostgreSQL 기본 파서(prsd_lextype) 토큰 타입 ID.
const WORD: usize = 2; // 한국어 형태소 → "word"
const BLANK: usize = 12; // 공백 → "blank"

/// lindera 분석기: 백엔드 프로세스당 1회 로드(불변 공유). `tokenize`가 `&self`.
static ANALYZER: OnceLock<LinderaAnalyzer> = OnceLock::new();

fn analyzer() -> &'static LinderaAnalyzer {
    ANALYZER.get_or_init(|| {
        LinderaAnalyzer::new_ko_dic()
            .unwrap_or_else(|e| pgrx::error!("pg_glot: lindera ko-dic 로드 실패: {e:?}"))
    })
}

/// 파서 상태: prsstart에서 전체 텍스트를 1회 토큰화해 저장, gettoken이 순회.
struct ParserState {
    tokens: Vec<OutTok>,
    idx: usize,
}

struct OutTok {
    surface: String,
    lextype: usize,
}

/// `prsstart(internal input, int4 len) -> internal`
#[pg_extern(immutable, parallel_safe)]
fn ko_prs_start(input: Internal, len: i32) -> Internal {
    // SAFETY: PG가 (char* input, int len)로 정규 호출. input datum은 입력 텍스트 포인터.
    let state = unsafe {
        let datum = input.unwrap().expect("pg_glot: null parser input");
        let ptr = datum.cast_mut_ptr::<u8>();
        let bytes = std::slice::from_raw_parts(ptr, len.max(0) as usize);
        let text = match std::str::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                pgrx::error!("pg_glot: 입력이 유효한 UTF-8이 아님 (DB 인코딩은 UTF8이어야 함)")
            }
        };
        let tokens = analyzer()
            .tokenize(text)
            .into_iter()
            .map(|t| {
                // 영숫자/한글/한자가 1자 이상이면 색인 대상(word), 순수 기호/공백은 blank.
                // (char::is_alphanumeric: 한글/한자=alphabetic, 숫자=numeric → true)
                let lextype = if t.surface.chars().any(char::is_alphanumeric) {
                    WORD
                } else {
                    BLANK
                };
                OutTok {
                    surface: t.surface,
                    lextype,
                }
            })
            .collect();
        ParserState { tokens, idx: 0 }
    };
    Internal::new(state)
}

/// `prsgettoken(internal state, internal **t, internal *tlen) -> internal` (반환 datum = int 토큰타입)
#[pg_extern(immutable, parallel_safe)]
fn ko_prs_nexttoken(mut state: Internal, t: Internal, tlen: Internal) -> Internal {
    let lextype: usize = unsafe {
        match state.get_mut::<ParserState>() {
            Some(st) if st.idx < st.tokens.len() => {
                let idx = st.idx;
                st.idx += 1;
                let tok = &st.tokens[idx];
                let t_out = t
                    .unwrap()
                    .expect("pg_glot: null token out-ptr")
                    .cast_mut_ptr::<*mut c_char>();
                let tlen_out = tlen
                    .unwrap()
                    .expect("pg_glot: null tlen out-ptr")
                    .cast_mut_ptr::<c_int>();
                // PG가 이 포인터의 바이트를 즉시 복사한다. surface는 ParserState 안에 살아있음.
                *t_out = tok.surface.as_ptr() as *mut c_char;
                *tlen_out = tok.surface.len() as c_int;
                tok.lextype
            }
            // 상태 없음 또는 토큰 소진 → 0 (파싱 종료)
            _ => 0,
        }
    };
    Internal::from(Some(pg_sys::Datum::from(lextype)))
}

/// `prsend(internal state) -> void`. 상태는 memory context 삭제 시 자동 정리.
#[pg_extern(immutable, parallel_safe)]
fn ko_prs_end(_state: Internal) {}

/// `glot` 네임스페이스 — 언어무관 공통 함수. 이 확장(pg_glot)이 `glot` 스키마를
/// 소유하고, pg_glot_hybrid(Layer B)가 그 위에 `glot.hybrid`를 더한다(스키마 공유).
#[pg_schema]
mod glot {
    use pgrx::prelude::*;
    use std::collections::HashMap;

    /// RRF(Reciprocal Rank Fusion) 프리미티브 — 백엔드/스키마/언어 무관 (D6).
    ///
    /// 각 leg는 **순위순 id 배열**(앞이 1위). 융합 점수 = `Σ_legs 1/(k + rank)`.
    /// (id, rank)만 받으므로 BM25/dense 외 임의 랭커·커스텀 스키마·외부 파이프라인에서
    /// 재사용된다. NULL 불기여. 점수 내림차순(동점 id 오름차순) 결정적 정렬.
    #[pg_extern(immutable, parallel_safe)]
    fn rrf(
        bm25: Vec<Option<i64>>,
        dense: Vec<Option<i64>>,
        k: default!(i32, 60),
    ) -> TableIterator<'static, (name!(id, i64), name!(score, f64))> {
        if k <= 0 {
            error!("rrf: k must be positive (got {k})");
        }
        let kf = f64::from(k);
        let mut scores: HashMap<i64, f64> = HashMap::new();
        let mut accumulate = |leg: &[Option<i64>]| {
            for (i, id) in leg.iter().enumerate() {
                if let Some(id) = id {
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

    /// 임베드된 사전/엔진 버전(`glot.dictionary_version()`). 사전(또는 분절에 영향을
    /// 주는 엔진 버전)이 바뀌면 기존 `korean` tsvector와 BM25 인덱스는 stale이 된다 —
    /// **사전은 인덱스 정의의 일부**이므로 REINDEX가 필요하다(재현성 가드, §10).
    #[pg_extern(immutable, parallel_safe)]
    fn dictionary_version() -> String {
        ::glot_tokenizer::dictionary_version()
    }
}

extension_sql!(
    r#"
CREATE TEXT SEARCH PARSER korean (
    START    = ko_prs_start,
    GETTOKEN = ko_prs_nexttoken,
    END      = ko_prs_end,
    LEXTYPES = pg_catalog.prsd_lextype,
    HEADLINE = pg_catalog.prsd_headline
);
COMMENT ON TEXT SEARCH PARSER korean IS 'Korean morphological parser (lindera + ko-dic)';

CREATE TEXT SEARCH CONFIGURATION korean (PARSER = korean);
COMMENT ON TEXT SEARCH CONFIGURATION korean IS 'Korean text search configuration (lindera)';

-- 형태소/숫자/ASCII 토큰을 simple 사전으로 매핑(소문자화+중복제거, 별도 스테밍 없음).
-- (후속: ASCII는 english_stem, POS 기반 정교화)
ALTER TEXT SEARCH CONFIGURATION korean
    ADD MAPPING FOR
        word, hword, hword_part,
        numword, numhword, hword_numpart,
        asciiword, asciihword, hword_asciipart
    WITH simple;
"#,
    name = "korean_ts_config",
    requires = [ko_prs_start, ko_prs_nexttoken, ko_prs_end],
);

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn lexeme_count(text: &str) -> Option<i32> {
        Spi::get_one::<i32>(&format!(
            "SELECT array_length(tsvector_to_array(to_tsvector('korean', {})), 1)",
            quote_literal(text)
        ))
        .expect("spi 실행 실패")
    }

    fn quote_literal(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    // 'korean' config가 형태소 토큰을 생성해야 한다.
    #[pg_test]
    fn korean_config_tokenizes_morphemes() {
        let n = lexeme_count("한국어 형태소 분석").expect("결과 null");
        assert!(n >= 3, "기대: 형태소 3개 이상, 실제 {n}");
    }

    // 빈 입력 → 빈 tsvector (패닉 없이).
    #[pg_test]
    fn empty_text_yields_empty_tsvector() {
        assert!(lexeme_count("").is_none(), "빈 입력은 빈 tsvector여야 함");
    }

    // ts_debug가 한국어 형태소를 'word' 타입으로 라벨해야 한다.
    #[pg_test]
    fn ts_debug_labels_korean_as_word() {
        let alias =
            Spi::get_one::<String>("SELECT alias FROM ts_debug('korean', '형태소') LIMIT 1")
                .expect("spi")
                .expect("null");
        assert_eq!(alias, "word", "한국어 형태소는 'word' 타입이어야 함");
    }

    // 색인/질의 파서 일관성: to_tsvector @@ to_tsquery 매칭.
    #[pg_test]
    fn tsvector_matches_tsquery() {
        let m = Spi::get_one::<bool>(
            "SELECT to_tsvector('korean', '한국어 형태소 분석 결과') @@ to_tsquery('korean', '형태소')",
        )
        .expect("spi")
        .expect("null");
        assert!(m, "색인/질의 파서 일관성: @@ 매칭되어야 함");
    }

    // 구두점만 → 인덱싱되는 lexeme 없어야 함 (기호는 word가 아님).
    #[pg_test]
    fn punctuation_only_not_indexed() {
        let n = lexeme_count("!!! ??? ... , ; :");
        assert!(n.is_none(), "구두점만 → lexeme 없어야 함, 실제 {n:?}");
    }

    // 장문(반복) — 패닉/오프셋 오류 없이 처리.
    #[pg_test]
    fn large_input_no_panic() {
        let n = Spi::get_one::<i32>(
            "SELECT array_length(tsvector_to_array(to_tsvector('korean', repeat('한국어 형태소 분석 결과. ', 1000))), 1)",
        )
        .expect("spi")
        .expect("null");
        assert!(n > 0, "장문 처리 실패");
    }

    // 같은 세션 반복 호출 — 파서 재진입/상태정리 안전.
    #[pg_test]
    fn repeated_calls_same_session() {
        for _ in 0..5 {
            let n = lexeme_count("검색 품질 평가").expect("null");
            assert!(n >= 1);
        }
    }

    // 사전 버전 노출: 사전 변경 시 REINDEX 판단 근거(재현성, §10).
    #[pg_test]
    fn dictionary_version_exposed() {
        let v = Spi::get_one::<String>("SELECT glot.dictionary_version()")
            .expect("spi")
            .expect("null");
        assert!(v.contains("mecab-ko-dic"), "ko-dic 사전 식별: {v}");
        assert!(v.contains("2.1.1-20180720"), "사전 버전: {v}");
    }
}

/// `cargo pgrx test`가 요구하는 모듈.
#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
