//! korean-tokenizer — lindera 기반 한국어 형태소 토크나이저 (순수 Rust).
//!
//! `pg_tsvector_ko`(Layer A)가 이 크레이트로 PostgreSQL `korean` text search
//! configuration(커스텀 TS parser)을 구성한다. lindera 사전은 백엔드 프로세스당
//! 1회 로드해 불변 공유한다(`tokenize`가 `&self`).
//!
//! 엔진=lindera(MIT, kuromoji-rs 계보), 사전=ko-dic(mecab-ko-dic 계열, Apache-2.0)
//! `embed-ko-dic` feature로 바이너리에 임베드 → 외부 사전 설치 0.

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer;

/// 임베드된 ko-dic 사전 버전(lindera-ko-dic 3.0.x가 번들한 mecab-ko-dic).
///
/// lindera-ko-dic 3.0.7 `build.rs`/`NOTICE.txt`로 확인된 값. 의존성을 올려
/// 임베드 사전이 바뀌면 이 상수를 함께 갱신할 것.
pub const KO_DIC_VERSION: &str = "mecab-ko-dic-2.1.1-20180720";

/// 분절에 영향을 주는 lindera 엔진 버전(Cargo.toml `lindera = "3.0"`).
pub const LINDERA_VERSION: &str = "3.0";

/// 임베드된 사전 + 엔진의 버전 식별자.
///
/// 사전(또는 분절 정책에 영향을 주는 엔진 버전)이 바뀌면 기존 tsvector/BM25
/// 인덱스는 stale이 된다 — **사전은 인덱스 정의의 일부**이므로 REINDEX가 필요하다.
/// `pg_tsvector_ko`는 이를 `pg_tsvector_ko_dictionary_version()` SQL 함수로 노출한다.
#[must_use]
pub fn dictionary_version() -> String {
    format!("{KO_DIC_VERSION} (lindera {LINDERA_VERSION}, embed-ko-dic)")
}

/// 한 형태소 토큰. `byte_start`/`byte_end`는 입력 UTF-8 바이트 오프셋
/// (PostgreSQL TS parser 토큰 생성에 필요).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub surface: String,
    pub byte_start: usize,
    pub byte_end: usize,
    /// 품사 태그(가능 시). lindera details[0]에서 추출(ko-dic POS).
    pub pos: Option<String>,
}

/// 형태소 분석기 추상화. lindera(기본) / 후속 Kiwi 등이 구현.
/// `Send + Sync`: 백엔드 프로세스에서 불변 공유.
pub trait Analyzer: Send + Sync {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}

/// lindera + 임베드 ko-dic 기반 분석기.
pub struct LinderaAnalyzer {
    tokenizer: Tokenizer,
}

impl LinderaAnalyzer {
    /// 임베드된 ko-dic으로 분석기 생성. 무겁다(사전 로드) — 프로세스당 1회 권장.
    pub fn new_ko_dic() -> lindera::LinderaResult<Self> {
        let dictionary = load_dictionary("embedded://ko-dic")?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        Ok(Self {
            tokenizer: Tokenizer::new(segmenter),
        })
    }
}

impl Analyzer for LinderaAnalyzer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = match self.tokenizer.tokenize(text) {
            Ok(t) => t,
            // 토큰화 실패 시 빈 결과. (PG 측 호출자는 ereport로 승격 가능)
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::with_capacity(tokens.len());
        for tok in tokens.iter_mut() {
            let pos = tok
                .details()
                .first()
                .map(|d| d.to_string())
                .filter(|s| !s.is_empty() && s != "UNK");
            out.push(Token {
                surface: tok.surface.to_string(),
                byte_start: tok.byte_start,
                byte_end: tok.byte_end,
                pos,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer() -> LinderaAnalyzer {
        LinderaAnalyzer::new_ko_dic().expect("ko-dic 로드 실패")
    }

    #[test]
    fn tokenizes_korean_and_offsets_map_to_surface() {
        let text = "한국어 형태소 분석";
        let toks = analyzer().tokenize(text);
        assert!(!toks.is_empty(), "토큰이 비어있음");
        for t in &toks {
            assert!(t.byte_start <= t.byte_end);
            assert!(t.byte_end <= text.len());
            // 오프셋이 char 경계여야 하고 surface와 일치해야 함(Normal 모드)
            let slice = text
                .get(t.byte_start..t.byte_end)
                .expect("byte offset이 char 경계가 아님");
            assert_eq!(slice, t.surface, "offset 슬라이스 != surface");
            assert!(!t.surface.is_empty());
        }
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(analyzer().tokenize("").is_empty());
    }

    #[test]
    fn splits_eojeol_into_morphemes() {
        // "먹었다" → 형태소 분리(먹/었/다 류). 최소한 '먹' 형태소 포함 기대.
        let toks = analyzer().tokenize("먹었다");
        assert!(!toks.is_empty());
        let surfaces: Vec<&str> = toks.iter().map(|t| t.surface.as_str()).collect();
        assert!(
            surfaces.iter().any(|s| s.contains('먹')),
            "기대: '먹' 형태소 포함, 실제: {surfaces:?}"
        );
    }

    #[test]
    fn mixed_korean_english_numeric() {
        let text = "PostgreSQL 16에서 BM25 검색";
        let toks = analyzer().tokenize(text);
        assert!(!toks.is_empty());
        for t in &toks {
            assert_eq!(text.get(t.byte_start..t.byte_end), Some(t.surface.as_str()));
        }
    }

    #[test]
    fn whitespace_and_punctuation_only() {
        let a = analyzer();
        // 공백/구두점만 — 패닉 없이, 토큰이 나오면 오프셋이 유효해야 함.
        for text in ["   ", "!!!", ".,?", "\n\t "] {
            for t in &a.tokenize(text) {
                assert_eq!(text.get(t.byte_start..t.byte_end), Some(t.surface.as_str()));
            }
        }
    }

    #[test]
    fn long_text_offsets_stay_valid() {
        // 장문에서 byte offset 누적이 어긋나지 않는지 (PG TS parser 정확성의 핵심).
        let text = "한국어 형태소 분석 테스트 문장입니다. ".repeat(2000);
        let toks = analyzer().tokenize(&text);
        assert!(!toks.is_empty());
        for t in &toks {
            assert!(t.byte_start <= t.byte_end && t.byte_end <= text.len());
            assert_eq!(text.get(t.byte_start..t.byte_end), Some(t.surface.as_str()));
        }
    }

    #[test]
    fn repeated_calls_are_idempotent() {
        let a = analyzer();
        let text = "검색 품질 평가 재현성";
        assert_eq!(a.tokenize(text), a.tokenize(text));
    }

    // 사전 버전 식별자: 사전은 인덱스 정의의 일부 → 재현성 가드.
    #[test]
    fn dictionary_version_identifies_kodic() {
        let v = dictionary_version();
        assert!(v.contains("mecab-ko-dic"), "ko-dic 사전 식별 포함: {v}");
        assert!(v.contains("2.1.1-20180720"), "사전 버전 포함: {v}");
        assert!(v.contains("lindera"), "엔진 식별 포함: {v}");
    }
}
