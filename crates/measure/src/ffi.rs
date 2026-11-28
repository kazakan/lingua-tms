//! `engine/include/tms_measure.h` 바인딩.
//!
//! 헤더의 구조체·함수와 1:1로 맞춘다. 헤더를 바꾸면 이 파일과 `ABI_VERSION`도 함께 바꾼다.

use std::ffi::{c_char, CStr, CString};
use std::path::Path;

use libloading::Library;

use crate::{FontId, Layout, Line, MeasureError, Measurer, Style, WrapMode};

/// TMS가 기대하는 ABI 버전(`TMS_MEASURE_ABI_VERSION`).
pub const ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct RawStyle {
    size_px: f32,
    letter_spacing: f32,
    line_height: f32,
    wrap_mode: i32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct RawLine {
    start: i32,
    end: i32,
    width: f32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct RawResult {
    width: f32,
    height: f32,
    line_count: i32,
    missing_glyphs: i32,
    word_overflow: i32,
}

/// 엔진이 소유하는 불투명 폰트 타입.
#[repr(C)]
struct RawFont {
    _private: [u8; 0],
}

type FnAbiVersion = unsafe extern "C" fn() -> u32;
type FnEngineVersion = unsafe extern "C" fn() -> *const c_char;
type FnFontLoad = unsafe extern "C" fn(*const u8, usize) -> *mut RawFont;
type FnFontFree = unsafe extern "C" fn(*mut RawFont);
type FnLayout = unsafe extern "C" fn(
    *const *const RawFont,
    usize,
    *const RawStyle,
    *const c_char,
    f32,
    *mut RawLine,
    usize,
    *mut RawResult,
) -> i32;

/// DLL 을 연다. Windows 에서 절대 경로면 DLL 이 의존하는 다른 DLL 도 그 DLL 의 폴더에서 먼저 찾는다
/// (엔진 DLL 옆에 둔 런타임 DLL 을 찾게). SAFETY: 신뢰하는 엔진 DLL만 불러온다는 전제. 초기화 루틴이 있어도 감수한다.
fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    #[cfg(windows)]
    if path.is_absolute() {
        use libloading::os::windows::{Library as WinLibrary, LOAD_WITH_ALTERED_SEARCH_PATH};
        return unsafe { WinLibrary::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH) }.map(Library::from);
    }
    unsafe { Library::new(path) }
}

/// 엔진 측정 DLL. 한 스레드에서만 순서대로 호출한다(앱에서는 Mutex 안에 둔다).
pub struct EngineDll {
    font_load: FnFontLoad,
    font_free: FnFontFree,
    layout: FnLayout,
    version: String,
    fonts: Vec<*mut RawFont>,
    // 함수 포인터와 폰트가 모두 정리된 뒤에 내려가야 하므로 마지막 필드로 둔다.
    _lib: Library,
}

// 원시 포인터 때문에 자동으로 Send가 아니다. 호출은 Mutex로 직렬화한다는 전제로 옮기는 것만 허용한다.
unsafe impl Send for EngineDll {}

impl EngineDll {
    /// DLL을 불러오고 ABI 버전을 확인한다.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, MeasureError> {
        let lib = open_library(path.as_ref()).map_err(|e| MeasureError::Load(e.to_string()))?;

        // SAFETY: 심볼 시그니처는 tms_measure.h와 같다.
        unsafe {
            let abi: FnAbiVersion = *lib
                .get::<FnAbiVersion>(b"tms_abi_version\0")
                .map_err(|e| MeasureError::Load(e.to_string()))?;
            let found = abi();
            if found != ABI_VERSION {
                return Err(MeasureError::AbiMismatch {
                    expected: ABI_VERSION,
                    found,
                });
            }
            let engine_version: FnEngineVersion = *lib
                .get::<FnEngineVersion>(b"tms_engine_version\0")
                .map_err(|e| MeasureError::Load(e.to_string()))?;
            let ptr = engine_version();
            let version = if ptr.is_null() {
                String::from("unknown")
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            };
            let font_load: FnFontLoad = *lib
                .get::<FnFontLoad>(b"tms_font_load\0")
                .map_err(|e| MeasureError::Load(e.to_string()))?;
            let font_free: FnFontFree = *lib
                .get::<FnFontFree>(b"tms_font_free\0")
                .map_err(|e| MeasureError::Load(e.to_string()))?;
            let layout: FnLayout = *lib
                .get::<FnLayout>(b"tms_layout\0")
                .map_err(|e| MeasureError::Load(e.to_string()))?;

            Ok(Self {
                font_load,
                font_free,
                layout,
                version,
                fonts: Vec::new(),
                _lib: lib,
            })
        }
    }
}

impl Measurer for EngineDll {
    fn engine_version(&self) -> String {
        self.version.clone()
    }

    fn load_font(&mut self, ttf: &[u8]) -> Result<FontId, MeasureError> {
        // SAFETY: 엔진은 호출 동안만 버퍼를 읽는다(필요하면 내부에 복사한다).
        let font = unsafe { (self.font_load)(ttf.as_ptr(), ttf.len()) };
        if font.is_null() {
            return Err(MeasureError::FontLoad);
        }
        self.fonts.push(font);
        Ok(FontId(self.fonts.len() - 1))
    }

    fn layout(
        &mut self,
        fonts: &[FontId],
        style: &Style,
        text: &str,
        box_width: f32,
    ) -> Result<Layout, MeasureError> {
        let font_ptrs = fonts
            .iter()
            .map(|id| {
                self.fonts
                    .get(id.0)
                    .map(|p| *p as *const RawFont)
                    .ok_or(MeasureError::FontLoad)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let c_text = CString::new(text).map_err(|_| MeasureError::NulInText)?;
        let raw_style = RawStyle {
            size_px: style.size_px,
            letter_spacing: style.letter_spacing,
            line_height: style.line_height,
            wrap_mode: match style.wrap {
                WrapMode::None => 0,
                WrapMode::Engine => 1,
            },
        };

        let mut capacity = 16usize;
        loop {
            let mut lines = vec![RawLine::default(); capacity];
            let mut out = RawResult::default();
            // SAFETY: 모든 포인터는 호출 동안 유효하고, lines 버퍼 길이는 capacity다.
            let rc = unsafe {
                (self.layout)(
                    font_ptrs.as_ptr(),
                    font_ptrs.len(),
                    &raw_style,
                    c_text.as_ptr(),
                    box_width,
                    lines.as_mut_ptr(),
                    capacity,
                    &mut out,
                )
            };
            if rc != 0 {
                return Err(MeasureError::Engine(rc));
            }
            let count = out.line_count.max(0) as usize;
            if count > capacity {
                capacity = count;
                continue;
            }
            lines.truncate(count);
            return Ok(Layout {
                width: out.width,
                height: out.height,
                lines: lines
                    .into_iter()
                    .map(|l| Line {
                        start: l.start.max(0) as usize,
                        end: l.end.max(0) as usize,
                        width: l.width,
                    })
                    .collect(),
                missing_glyphs: out.missing_glyphs.max(0) as u32,
                word_overflow: out.word_overflow != 0,
            });
        }
    }
}

impl Drop for EngineDll {
    fn drop(&mut self) {
        for font in self.fonts.drain(..) {
            // SAFETY: load_font가 돌려받은 포인터이고 한 번만 해제한다.
            unsafe { (self.font_free)(font) };
        }
    }
}
