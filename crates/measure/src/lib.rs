//! 측정 엔진 추상화.
//!
//! - [`Measurer`]: 검증기가 쓰는 측정 인터페이스. 렌더링 검증기는 이 트레이트만 안다.
//! - [`EngineDll`]: 자체 엔진 측정 DLL(`engine/include/tms_measure.h`)을 불러 쓰는 구현.
//!
//! 엔진 DLL이 아직 없을 때는 `crates/engine-stub`이 만드는 가짜 DLL로 개발한다.

mod ffi;

pub use ffi::EngineDll;

use serde::{Deserialize, Serialize};

/// 측정 스타일. `tms_style`과 같은 의미다.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub size_px: f32,
    pub letter_spacing: f32,
    /// 0이면 엔진 기본값.
    pub line_height: f32,
    pub wrap: WrapMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapMode {
    /// 줄바꿈 없이 한 줄.
    None,
    /// 엔진 기본 줄바꿈.
    Engine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// UTF-8 바이트 위치(포함).
    pub start: usize,
    /// UTF-8 바이트 위치(제외).
    pub end: usize,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// 가장 넓은 줄의 폭.
    pub width: f32,
    pub height: f32,
    pub lines: Vec<Line>,
    pub missing_glyphs: u32,
    /// 단어 하나가 박스보다 넓어 넘쳤는지.
    pub word_overflow: bool,
}

impl Layout {
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error("엔진 DLL을 불러올 수 없음: {0}")]
    Load(String),
    #[error("엔진 ABI 버전 불일치: TMS는 {expected}, DLL은 {found}")]
    AbiMismatch { expected: u32, found: u32 },
    #[error("폰트를 불러올 수 없음")]
    FontLoad,
    #[error("문구에 NUL 문자가 있음")]
    NulInText,
    #[error("엔진 오류 코드 {0}")]
    Engine(i32),
}

/// 엔진이 불러 둔 폰트를 가리키는 번호. 실제 핸들은 Measurer 구현이 가진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub usize);

/// 검증기가 쓰는 측정 인터페이스.
///
/// 구현은 한 스레드에서만 호출된다고 가정한다(엔진이 전역 상태를 써도 되도록).
pub trait Measurer {
    /// 결과에 함께 기록할 엔진 버전 문자열.
    fn engine_version(&self) -> String;

    /// .ttf 바이트를 불러 폰트 번호를 돌려준다.
    fn load_font(&mut self, ttf: &[u8]) -> Result<FontId, MeasureError>;

    /// `fonts`는 폴백 순서. `box_width`가 0이면 한 줄로 측정한다.
    fn layout(
        &mut self,
        fonts: &[FontId],
        style: &Style,
        text: &str,
        box_width: f32,
    ) -> Result<Layout, MeasureError>;
}
