//! glot-tokenizer — lindera 기반 CJK(한·중·일) 형태소 토크나이저 (순수 Rust).
//!
//! `pg_glot`(Layer A)가 이 크레이트로 PostgreSQL `korean`/`japanese`/`chinese`
//! text search configuration(커스텀 TS parser)을 구성한다. lindera 사전은 백엔드
//! 프로세스당 1회 로드해 불변 공유한다(`tokenize`가 `&self`).
//!
//! 엔진=lindera(MIT, kuromoji-rs 계보). 사전: ko-dic(ko, Apache-2.0) / IPADIC(ja) /
//! CC-CEDICT(zh). feature(`korean`/`japanese`/`chinese`, default 셋 다)로 임베드 선택.
//! 실제 검증·출하는 korean; ja/zh는 구조 지원(품질 미검증).

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer;

/// 분절에 영향을 주는 lindera 엔진 버전. Cargo.toml의 exact 핀(`lindera = "=3.0.7"`)과
/// 반드시 일치해야 한다 — 핀과 이 상수는 함께 움직인다(`lindera_constant_matches_cargo_lock`
/// 테스트가 Cargo.lock 해석 버전과의 드리프트를 하드 실패로 막는다). 버전을 올리려면
/// 먼저 분절을 재검증한 뒤 핀과 이 상수를 동시에 갱신할 것.
pub const LINDERA_VERSION: &str = "3.0.7";

/// 임베드된 ko-dic 사전 버전(lindera-ko-dic 3.0.7 `build.rs`/`NOTICE.txt`로 확인).
#[cfg(feature = "korean")]
pub const KO_DIC_VERSION: &str = "mecab-ko-dic-2.1.1-20180720";
/// 임베드된 IPADIC 사전 버전(lindera-ipadic 3.0.7).
#[cfg(feature = "japanese")]
pub const IPADIC_VERSION: &str = "mecab-ipadic-2.7.0-20070801";
/// 임베드된 CC-CEDICT 사전(lindera-cc-cedict 3.0.7).
#[cfg(feature = "chinese")]
pub const CC_CEDICT_VERSION: &str = "CC-CEDICT (lindera-cc-cedict 3.0.7)";

/// 지원 언어(=임베드 사전). feature로 게이트된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    #[cfg(feature = "korean")]
    Korean,
    #[cfg(feature = "japanese")]
    Japanese,
    #[cfg(feature = "chinese")]
    Chinese,
}

impl Lang {
    fn embedded_uri(self) -> &'static str {
        match self {
            #[cfg(feature = "korean")]
            Lang::Korean => "embedded://ko-dic",
            #[cfg(feature = "japanese")]
            Lang::Japanese => "embedded://ipadic",
            #[cfg(feature = "chinese")]
            Lang::Chinese => "embedded://cc-cedict",
        }
    }

    /// PostgreSQL TS config 이름(`korean`/`japanese`/`chinese`).
    #[must_use]
    pub fn config_name(self) -> &'static str {
        match self {
            #[cfg(feature = "korean")]
            Lang::Korean => "korean",
            #[cfg(feature = "japanese")]
            Lang::Japanese => "japanese",
            #[cfg(feature = "chinese")]
            Lang::Chinese => "chinese",
        }
    }

    /// 임베드 사전 + 엔진 버전 식별자. 사전(또는 분절에 영향을 주는 엔진 버전)이
    /// 바뀌면 기존 tsvector/BM25 인덱스는 stale → REINDEX(사전은 인덱스 정의의 일부).
    #[must_use]
    pub fn dictionary_version(self) -> String {
        let (dic, feat) = match self {
            #[cfg(feature = "korean")]
            Lang::Korean => (KO_DIC_VERSION, "embed-ko-dic"),
            #[cfg(feature = "japanese")]
            Lang::Japanese => (IPADIC_VERSION, "embed-ipadic"),
            #[cfg(feature = "chinese")]
            Lang::Chinese => (CC_CEDICT_VERSION, "embed-cc-cedict"),
        };
        format!("{dic} (lindera {LINDERA_VERSION}, {feat})")
    }

    /// POS 태그가 색인 대상(내용어)인가. 기능어(조사/어미/조동사/기호)는 false → 색인
    /// 제외(BM25 노이즈 감소). POS가 없으면(zh cc-cedict는 품사 미제공) 필터 불가 →
    /// 보수적으로 모두 내용어 취급(true).
    #[must_use]
    pub fn is_content_pos(self, pos: Option<&str>) -> bool {
        let pos = match pos {
            Some(p) => p,
            None => return true,
        };
        match self {
            // ko: MeCab accept-list(내용어 POS)로 색인. lindera + 이 필터 = NDCG 0.636(MIRACL
            // 측정) → MeCab(0.633)/research(0.6385) 동급. 정체성(순수 Rust 임베드)을 지키며
            // 품질 확보. (A1의 넓은 allowlist N*/V*/MA*는 무효였으나, 정확한 accept-list가 +3%p.
            // 토크나이저 분절은 lindera≈MeCab이고 진짜 레버는 이 POS 필터였다.)
            // 복합 태그(예: 'VV+EC')는 첫 형태소(VV) 기준으로 판정.
            #[cfg(feature = "korean")]
            Lang::Korean => {
                const ACCEPT: &[&str] = &[
                    "NNG", "NNP", "NNB", "NNBC", "NR", "VV", "VA", "MM", "MAG", "XSN", "XR", "SH",
                    "SL",
                ];
                ACCEPT.contains(&pos.split('+').next().unwrap_or(""))
            }
            // ja(ipadic denylist): 助詞(조사)/助動詞(조동사)/記号(기호)/フィラー/感動詞/接続詞
            // 제외 → recall +1.5%p(측정). accept-list(名詞/動詞/…)와 성능 동등(0.5658≈0.5647,
            // bench/RESULTS.md §ja/zh)이라 더 단순한 denylist를 채택. ko가 accept-list인 것과
            // 대비되는 의도된 비대칭(ja는 ipadic POS 체계가 넓어 denylist가 자연스럽다).
            #[cfg(feature = "japanese")]
            Lang::Japanese => !matches!(
                pos,
                "助詞" | "助動詞" | "記号" | "フィラー" | "感動詞" | "接続詞"
            ),
            // zh(cc-cedict): 사전이 POS를 제공하지 않아('*') 내용어/기능어 구분이 불가 → 필터
            // 없이 전부 색인(보수적). stopword(的/了/…)도 측정상 무효(−0.22%p, bench/RESULTS.md
            // §ja/zh)라 미적용. ko(accept)/ja(deny)와 대비되는 의도된 비대칭(사전 한계).
            #[cfg(feature = "chinese")]
            Lang::Chinese => true,
        }
    }
}

/// 하위호환: 인자 없는 사전버전 = korean. `pg_glot`이 `glot.dictionary_version()`으로 노출.
#[cfg(feature = "korean")]
#[must_use]
pub fn dictionary_version() -> String {
    Lang::Korean.dictionary_version()
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

/// lindera 기반 분석기(언어=`Lang`). 사전 로드는 무겁다 — 프로세스당 1회 권장.
pub struct LinderaAnalyzer {
    tokenizer: Tokenizer,
    lang: Lang,
}

impl LinderaAnalyzer {
    /// 지정 언어의 임베드 사전으로 분석기 생성.
    pub fn new(lang: Lang) -> lindera::LinderaResult<Self> {
        let dictionary = load_dictionary(lang.embedded_uri())?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        Ok(Self {
            tokenizer: Tokenizer::new(segmenter),
            lang,
        })
    }

    /// 이 분석기의 언어(POS 필터 등 언어별 처리에 사용).
    #[must_use]
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// 하위호환: ko-dic 분석기 = `new(Lang::Korean)`.
    #[cfg(feature = "korean")]
    pub fn new_ko_dic() -> lindera::LinderaResult<Self> {
        Self::new(Lang::Korean)
    }
}

/// 가타카나 중점(`・` U+30FB, 반각 `･` U+FF65)으로 surface를 하위 토큰으로 분할한다.
/// lindera/ipadic은 외국 인명 "トーマス・エジソン"을 **단일 토큰**으로 emit하는데, 그러면
/// "エジソン" 쿼리가 매칭되지 않는다(MIRACL ja 측정: recall −4.4%p / NDCG −2.6%p). PG 기본
/// 파서가 ・에서 분할하는 것과 동일하게 쪼갠다. 반환 오프셋은 원문 기준(`base`를 더함)이라
/// `text[start..end] == surface` 불변식을 그대로 유지한다. ・가 없으면 통째로(빠른 경로).
fn split_middle_dot(surface: &str, base: usize) -> Vec<(&str, usize, usize)> {
    if !surface.contains(['・', '･']) {
        return vec![(surface, base, base + surface.len())];
    }
    let mut out = Vec::new();
    let mut seg = 0; // surface 내 세그먼트 시작 바이트
    for (i, c) in surface.char_indices() {
        if c == '・' || c == '･' {
            if i > seg {
                out.push((&surface[seg..i], base + seg, base + i));
            }
            seg = i + c.len_utf8();
        }
    }
    if surface.len() > seg {
        out.push((&surface[seg..], base + seg, base + surface.len()));
    }
    out
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
            // ・-결합 토큰을 하위 토큰으로 분할(없으면 1개 그대로). 오프셋은 원문 기준 유지.
            for (sub, start, end) in split_middle_dot(&tok.surface, tok.byte_start) {
                out.push(Token {
                    surface: sub.to_string(),
                    byte_start: start,
                    byte_end: end,
                    pos: pos.clone(),
                });
            }
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

    /// 워크스페이스 `Cargo.lock`에서 실제로 해석된 lindera 패키지 버전을 추출.
    /// `[[package]]` 블록 중 `name = "lindera"`인 것의 `version = "..."`를 반환한다.
    fn locked_lindera_version() -> String {
        // CARGO_MANIFEST_DIR = crates/glot-tokenizer → ../../ = 워크스페이스 루트.
        let lock_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock");
        let lock = std::fs::read_to_string(lock_path)
            .unwrap_or_else(|e| panic!("Cargo.lock 읽기 실패({lock_path}): {e}"));

        let mut in_lindera = false;
        for line in lock.lines() {
            let trimmed = line.trim();
            if trimmed == "[[package]]" {
                in_lindera = false; // 새 패키지 블록 시작 → 플래그 리셋.
            } else if trimmed == r#"name = "lindera""# {
                in_lindera = true;
            } else if in_lindera {
                if let Some(rest) = trimmed.strip_prefix("version = \"") {
                    if let Some(ver) = rest.strip_suffix('"') {
                        return ver.to_string();
                    }
                }
            }
        }
        panic!("Cargo.lock에서 lindera 패키지의 version을 찾지 못함");
    }

    /// 손으로 유지하는 `LINDERA_VERSION` 상수가 Cargo.lock의 실제 해석 버전과
    /// 드리프트하면 큰 소리로 실패시킨다 — "의존성 올리고 상수 깜빡"을 하드 실패로.
    /// lindera는 Cargo.toml에서 `=3.0.7`로 핀(exact)되어 있고, 핀과 이 상수는
    /// 반드시 함께 움직여야 한다(버전을 올리면 분절 재검증 후 상수도 갱신).
    #[test]
    fn lindera_constant_matches_cargo_lock() {
        let locked = locked_lindera_version();
        assert_eq!(
            locked, LINDERA_VERSION,
            "LINDERA_VERSION 상수({LINDERA_VERSION})와 Cargo.lock 해석 버전({locked})이 \
             불일치 — lindera 핀을 올렸다면 LINDERA_VERSION 상수도 갱신하고 분절을 재검증할 것"
        );
    }

    // ── ja/zh: CJK 확장. byte-offset 불변식(text[start..end]==surface)은 언어 불문 —
    //    PostgreSQL TS parser 정확성의 핵심이라 모든 언어에서 검증한다. ──

    #[cfg(feature = "japanese")]
    #[test]
    fn tokenizes_japanese_offsets_map_to_surface() {
        let a = LinderaAnalyzer::new(Lang::Japanese).expect("ipadic 로드 실패");
        let text = "東京都に住んでいます";
        let toks = a.tokenize(text);
        assert!(!toks.is_empty(), "일본어 토큰이 비어있음");
        for t in &toks {
            assert_eq!(
                text.get(t.byte_start..t.byte_end),
                Some(t.surface.as_str()),
                "offset 슬라이스 != surface (ja)"
            );
        }
    }

    // 가타카나 중점(・)으로 이어진 외국 인명은 하위 토큰으로 분리되어야 한다. lindera/ipadic은
    // "トーマス・エジソン"을 단일 토큰으로 emit하는데, 그러면 "エジソン" 쿼리가 매칭되지 않는다
    // (MIRACL ja 측정: recall −4.4%p / NDCG −2.6%p). PG 기본 파서가 ・에서 분할하는 것과 동일.
    // split_middle_dot: 결정적 단위 테스트(lindera 무관). 오프셋은 base 기준이며 ・(3바이트)를
    // 건너뛴다. 빈 세그먼트(선두/말미/연속 ・)는 버린다.
    #[test]
    fn split_middle_dot_offsets_and_segments() {
        // 단순: ・ 없음 → 통째로.
        assert_eq!(split_middle_dot("東京", 0), vec![("東京", 0, 6)]);
        // 분할 + 원문 기준 오프셋(base=10).
        let s = "トーマス・エジソン";
        let parts = split_middle_dot(s, 10);
        let surfs: Vec<&str> = parts.iter().map(|p| p.0).collect();
        assert_eq!(surfs, vec!["トーマス", "エジソン"]);
        // base를 빼면 원래 surface 슬라이스와 일치.
        for (sub, start, end) in parts {
            assert_eq!(&s[start - 10..end - 10], sub);
        }
        // 반각 ･ + 선두/연속/말미 ・는 빈 세그먼트로 버려짐.
        assert_eq!(
            split_middle_dot("・A･･B・", 0)
                .iter()
                .map(|p| p.0)
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        // 전부 ・ → 빈 결과.
        assert!(split_middle_dot("・･", 0).is_empty());
    }

    #[cfg(feature = "japanese")]
    #[test]
    fn japanese_splits_foreign_name_on_middle_dot() {
        let a = LinderaAnalyzer::new(Lang::Japanese).expect("ipadic 로드 실패");
        let text = "トーマス・エジソン";
        let toks = a.tokenize(text);
        let surfaces: Vec<&str> = toks.iter().map(|t| t.surface.as_str()).collect();
        assert!(
            surfaces.contains(&"エジソン"),
            "・ 뒤 'エジソン'이 별도 토큰이어야: {surfaces:?}"
        );
        assert!(
            surfaces.contains(&"トーマス"),
            "・ 앞 'トーマス'이 별도 토큰이어야: {surfaces:?}"
        );
        assert!(
            !surfaces.iter().any(|s| s.contains('・')),
            "・가 토큰 안에 남으면 안 됨: {surfaces:?}"
        );
        // byte-offset 불변식: 분리 후에도 text[start..end]==surface.
        for t in &toks {
            assert_eq!(text.get(t.byte_start..t.byte_end), Some(t.surface.as_str()));
        }
    }

    #[cfg(feature = "chinese")]
    #[test]
    fn tokenizes_chinese_offsets_map_to_surface() {
        let a = LinderaAnalyzer::new(Lang::Chinese).expect("cc-cedict 로드 실패");
        let text = "我喜欢自然语言处理";
        let toks = a.tokenize(text);
        assert!(!toks.is_empty(), "중국어 토큰이 비어있음");
        for t in &toks {
            assert_eq!(
                text.get(t.byte_start..t.byte_end),
                Some(t.surface.as_str()),
                "offset 슬라이스 != surface (zh)"
            );
        }
    }

    /// 언어별 사전버전 식별자가 해당 사전을 가리킨다.
    #[test]
    fn dictionary_version_per_lang() {
        #[cfg(feature = "korean")]
        assert!(Lang::Korean.dictionary_version().contains("ko-dic"));
        #[cfg(feature = "japanese")]
        assert!(Lang::Japanese.dictionary_version().contains("ipadic"));
        #[cfg(feature = "chinese")]
        assert!(Lang::Chinese.dictionary_version().contains("CC-CEDICT"));
    }

    #[test]
    fn is_content_pos_filters_functional_words() {
        // ko: MeCab accept-list — 명사/용언 등 내용어는 색인, 조사/어미는 제외.
        #[cfg(feature = "korean")]
        {
            assert!(Lang::Korean.is_content_pos(Some("NNG")));
            assert!(Lang::Korean.is_content_pos(Some("VV+EC"))); // 복합 → 첫 형태소 VV
            assert!(!Lang::Korean.is_content_pos(Some("JKO"))); // 조사
            assert!(!Lang::Korean.is_content_pos(Some("EC"))); // 어미
        }
        #[cfg(feature = "japanese")]
        {
            assert!(Lang::Japanese.is_content_pos(Some("名詞")), "명사=내용어");
            assert!(Lang::Japanese.is_content_pos(Some("動詞")), "동사=내용어");
            assert!(!Lang::Japanese.is_content_pos(Some("助詞")), "조사=기능어");
            assert!(
                !Lang::Japanese.is_content_pos(Some("助動詞")),
                "조동사=기능어"
            );
        }
        // zh: cc-cedict POS 미제공('*') → 필터 불가, 전부 색인.
        #[cfg(feature = "chinese")]
        assert!(Lang::Chinese.is_content_pos(Some("*")));
        // POS None → 보수적 색인.
        #[cfg(feature = "korean")]
        assert!(Lang::Korean.is_content_pos(None));
    }

    #[test]
    fn analyzer_exposes_lang() {
        #[cfg(feature = "korean")]
        assert_eq!(
            LinderaAnalyzer::new(Lang::Korean).unwrap().lang(),
            Lang::Korean
        );
        #[cfg(feature = "japanese")]
        assert_eq!(
            LinderaAnalyzer::new(Lang::Japanese).unwrap().lang(),
            Lang::Japanese
        );
    }
}
