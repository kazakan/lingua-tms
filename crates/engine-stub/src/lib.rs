//! 개발용 가짜 엔진 측정 DLL.
//!
//! 실제 엔진 DLL이 나오기 전까지 TMS 개발과 FFI 테스트에 쓴다.
//! 폰트 파일 내용은 보지 않고 아래 규칙으로 폭을 계산한다. 실제 엔진과 결과가 다르다.
//!
//! - 한글·한자·전각 문자: 글자 크기 × 1.0
//! - 공백: 글자 크기 × 0.3
//! - 그 밖의 문자: 글자 크기 × 0.55
//! - 자간: 줄 안의 글자 사이마다 더한다
//! - 줄바꿈: 공백에서만 끊는다(한국어 keep-all과 같은 효과). 박스보다 긴 단어는 넘침으로 보고한다.

use std::ffi::{c_char, CStr};
use std::ptr;

#[repr(C)]
pub struct TmsStyle {
    pub size_px: f32,
    pub letter_spacing: f32,
    pub line_height: f32,
    pub wrap_mode: i32,
}

#[repr(C)]
pub struct TmsLine {
    pub start: i32,
    pub end: i32,
    pub width: f32,
}

#[repr(C)]
pub struct TmsResult {
    pub width: f32,
    pub height: f32,
    pub line_count: i32,
    pub missing_glyphs: i32,
    pub word_overflow: i32,
}

pub struct TmsFont {
    _len: usize,
}

static VERSION: &[u8] = b"stub-0.1.0\0";

#[no_mangle]
pub extern "C" fn tms_abi_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn tms_engine_version() -> *const c_char {
    VERSION.as_ptr() as *const c_char
}

/// # Safety
/// `ttf`는 `len` 바이트를 읽을 수 있는 포인터여야 한다.
#[no_mangle]
pub unsafe extern "C" fn tms_font_load(ttf: *const u8, len: usize) -> *mut TmsFont {
    if ttf.is_null() || len == 0 {
        return ptr::null_mut();
    }
    Box::into_raw(Box::new(TmsFont { _len: len }))
}

/// # Safety
/// `tms_font_load`가 돌려준 포인터를 한 번만 넘겨야 한다.
#[no_mangle]
pub unsafe extern "C" fn tms_font_free(font: *mut TmsFont) {
    if !font.is_null() {
        drop(Box::from_raw(font));
    }
}

fn advance(c: char, size: f32) -> f32 {
    let wide = matches!(c as u32,
        0x1100..=0x11FF | 0x3000..=0x303F | 0x3130..=0x318F | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFF00..=0xFFEF);
    if c == ' ' {
        size * 0.3
    } else if wide {
        size
    } else {
        size * 0.55
    }
}

/// `text[start..end]`의 폭. 자간은 글자 사이에만 더한다.
fn width_of(text: &str, style: &TmsStyle) -> f32 {
    let mut w = 0.0;
    let mut n = 0;
    for c in text.chars() {
        w += advance(c, style.size_px);
        n += 1;
    }
    if n > 1 {
        w += style.letter_spacing * (n - 1) as f32;
    }
    w
}

/// 반환: (줄 목록(start, end, width), 단어 넘침 여부)
pub fn layout_text(text: &str, style: &TmsStyle, box_width: f32) -> (Vec<(usize, usize, f32)>, bool) {
    if style.wrap_mode == 0 || box_width <= 0.0 {
        return (vec![(0, text.len(), width_of(text, style))], false);
    }
    // 단어 = 공백이 아닌 연속 구간
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c == ' ' {
            if let Some(s) = start.take() {
                words.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        words.push((s, text.len()));
    }
    if words.is_empty() {
        return (vec![(0, text.len(), 0.0)], false);
    }

    let mut lines = Vec::new();
    let mut overflow = false;
    let mut line_start = words[0].0;
    let mut line_end = words[0].1;
    if width_of(&text[words[0].0..words[0].1], style) > box_width {
        overflow = true;
    }
    for &(ws, we) in &words[1..] {
        let candidate = width_of(&text[line_start..we], style);
        if candidate <= box_width {
            line_end = we;
        } else {
            lines.push((line_start, line_end, width_of(&text[line_start..line_end], style)));
            line_start = ws;
            line_end = we;
            if width_of(&text[ws..we], style) > box_width {
                overflow = true;
            }
        }
    }
    lines.push((line_start, line_end, width_of(&text[line_start..line_end], style)));
    (lines, overflow)
}

/// # Safety
/// 포인터 인자는 tms_measure.h 규약을 따라야 한다.
#[no_mangle]
pub unsafe extern "C" fn tms_layout(
    fonts: *const *const TmsFont,
    font_count: usize,
    style: *const TmsStyle,
    text_utf8: *const c_char,
    box_width: f32,
    lines: *mut TmsLine,
    max_lines: usize,
    out: *mut TmsResult,
) -> i32 {
    if style.is_null() || text_utf8.is_null() || out.is_null() {
        return 1;
    }
    if fonts.is_null() || font_count == 0 {
        return 2;
    }
    let style = &*style;
    let Ok(text) = CStr::from_ptr(text_utf8).to_str() else {
        return 3;
    };

    let (result_lines, overflow) = layout_text(text, style, box_width);
    if !lines.is_null() {
        for (i, (s, e, w)) in result_lines.iter().take(max_lines).enumerate() {
            *lines.add(i) = TmsLine {
                start: *s as i32,
                end: *e as i32,
                width: *w,
            };
        }
    }
    let line_height = if style.line_height > 0.0 {
        style.line_height
    } else {
        style.size_px * 1.2
    };
    *out = TmsResult {
        width: result_lines.iter().map(|l| l.2).fold(0.0, f32::max),
        height: line_height * result_lines.len() as f32,
        line_count: result_lines.len() as i32,
        missing_glyphs: 0,
        word_overflow: overflow as i32,
    };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TmsStyle {
        TmsStyle {
            size_px: 20.0,
            letter_spacing: 0.0,
            line_height: 0.0,
            wrap_mode: 1,
        }
    }

    #[test]
    fn wraps_at_spaces() {
        let (lines, overflow) = layout_text("확인 취소", &style(), 50.0);
        assert_eq!(lines.len(), 2);
        assert!(!overflow);
    }

    #[test]
    fn long_word_overflows() {
        let (_, overflow) = layout_text("가나다라마", &style(), 50.0);
        assert!(overflow);
    }
}
