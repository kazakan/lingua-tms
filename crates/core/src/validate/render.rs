//! 렌더링 기반 검증기. 폭·줄 수는 모두 엔진(Measurer)이 잰다.

use serde::Deserialize;
use serde_json::{json, Value};
use tms_measure::WrapMode;

use super::{parse_params, Outcome, RenderCtx, TextCtx, ValidateError, Validator};

pub(super) fn all() -> Vec<Box<dyn Validator>> {
    vec![
        Box::new(GlyphCoverage),
        Box::new(MaxWidth),
        Box::new(LineCount),
        Box::new(WordOverflow),
    ]
}

fn need<'a, 'm>(render: Option<&'a mut RenderCtx<'m>>) -> Result<&'a mut RenderCtx<'m>, ValidateError> {
    render.ok_or(ValidateError::NeedsRender)
}

// ── render.glyph_coverage ──────────────────────
struct GlyphCoverage;

impl Validator for GlyphCoverage {
    fn id(&self) -> &'static str {
        "render.glyph_coverage"
    }
    fn description(&self) -> &'static str {
        "폰트(폴백 포함)에 없는 글자 검출"
    }
    fn params_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn needs_render(&self) -> bool {
        true
    }
    fn check(&self, ctx: &TextCtx<'_>, _: &Value, render: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let r = need(render)?;
        let layout = r.measurer.layout(&r.fonts, &r.style, ctx.value, 0.0)?;
        let metrics = json!({ "missing_glyphs": layout.missing_glyphs });
        Ok(if layout.missing_glyphs == 0 {
            Outcome::pass(metrics)
        } else {
            Outcome::fail(format!("폰트에 없는 글자 {}개", layout.missing_glyphs), metrics)
        })
    }
}

// ── render.max_width ───────────────────────────
struct MaxWidth;

#[derive(Deserialize)]
struct MaxWidthParams {
    max_width_px: f32,
}

impl Validator for MaxWidth {
    fn id(&self) -> &'static str {
        "render.max_width"
    }
    fn description(&self) -> &'static str {
        "줄바꿈 없이 한 줄 폭 ≤ N px"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["max_width_px"],
            "properties": { "max_width_px": { "type": "number", "exclusiveMinimum": 0, "title": "최대 폭(px)" } }
        })
    }
    fn needs_render(&self) -> bool {
        true
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, render: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: MaxWidthParams = parse_params(params)?;
        let r = need(render)?;
        let layout = r.measurer.layout(&r.fonts, &r.style, ctx.value, 0.0)?;
        let metrics = json!({ "width_px": layout.width, "max_width_px": p.max_width_px });
        Ok(if layout.width <= p.max_width_px {
            Outcome::pass(metrics)
        } else {
            Outcome::fail(
                format!("폭 {:.1}px > 한도 {:.1}px", layout.width, p.max_width_px),
                metrics,
            )
        })
    }
}

// ── render.line_count ──────────────────────────
struct LineCount;

#[derive(Deserialize)]
struct LineCountParams {
    box_width_px: f32,
    min: Option<usize>,
    max: Option<usize>,
}

impl Validator for LineCount {
    fn id(&self) -> &'static str {
        "render.line_count"
    }
    fn description(&self) -> &'static str {
        "박스 폭에서 자동 줄바꿈 시 줄 수 범위"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["box_width_px"],
            "properties": {
                "box_width_px": { "type": "number", "exclusiveMinimum": 0, "title": "박스 폭(px)" },
                "min": { "type": "integer", "minimum": 1, "title": "최소 줄 수" },
                "max": { "type": "integer", "minimum": 1, "title": "최대 줄 수" }
            }
        })
    }
    fn needs_render(&self) -> bool {
        true
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, render: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: LineCountParams = parse_params(params)?;
        let r = need(render)?;
        let mut style = r.style;
        style.wrap = WrapMode::Engine;
        let layout = r.measurer.layout(&r.fonts, &style, ctx.value, p.box_width_px)?;
        let n = layout.line_count();
        let metrics = json!({
            "lines": n,
            "min": p.min,
            "max": p.max,
            "line_widths": layout.lines.iter().map(|l| l.width).collect::<Vec<_>>(),
        });
        if p.min.is_some_and(|min| n < min) || p.max.is_some_and(|max| n > max) {
            let range = match (p.min, p.max) {
                (Some(a), Some(b)) if a == b => format!("{a}줄"),
                (Some(a), Some(b)) => format!("{a}~{b}줄"),
                (Some(a), None) => format!("{a}줄 이상"),
                (None, Some(b)) => format!("{b}줄 이하"),
                (None, None) => String::new(),
            };
            return Ok(Outcome::fail(format!("{n}줄 (기대: {range})"), metrics));
        }
        Ok(Outcome::pass(metrics))
    }
}

// ── render.word_overflow ───────────────────────
struct WordOverflow;

#[derive(Deserialize)]
struct WordOverflowParams {
    box_width_px: f32,
}

impl Validator for WordOverflow {
    fn id(&self) -> &'static str {
        "render.word_overflow"
    }
    fn description(&self) -> &'static str {
        "단어 하나가 박스보다 넓어 넘침(독일어 복합어 등)"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["box_width_px"],
            "properties": { "box_width_px": { "type": "number", "exclusiveMinimum": 0, "title": "박스 폭(px)" } }
        })
    }
    fn needs_render(&self) -> bool {
        true
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, render: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: WordOverflowParams = parse_params(params)?;
        let r = need(render)?;
        let mut style = r.style;
        style.wrap = WrapMode::Engine;
        let layout = r.measurer.layout(&r.fonts, &style, ctx.value, p.box_width_px)?;
        let metrics = json!({ "word_overflow": layout.word_overflow, "box_width_px": p.box_width_px });
        Ok(if layout.word_overflow {
            Outcome::fail("박스보다 넓은 단어가 있음", metrics)
        } else {
            Outcome::pass(metrics)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::Verdict::{Fail, Pass};
    use super::*;
    use tms_measure::{FontId, Layout, Line, MeasureError, Measurer, Style};

    /// 글자당 10px, 공백에서만 줄바꿈하는 가짜 측정기. '□'는 폰트에 없는 글자로 본다.
    struct Fake;

    impl Measurer for Fake {
        fn engine_version(&self) -> String {
            "fake".into()
        }
        fn load_font(&mut self, _: &[u8]) -> Result<FontId, MeasureError> {
            Ok(FontId(0))
        }
        fn layout(&mut self, _: &[FontId], _: &Style, text: &str, box_width: f32) -> Result<Layout, MeasureError> {
            let w = |s: &str| s.chars().count() as f32 * 10.0;
            let missing = text.chars().filter(|c| *c == '□').count() as u32;
            let mut lines: Vec<Line> = Vec::new();
            let mut overflow = false;
            if box_width <= 0.0 {
                lines.push(Line { start: 0, end: text.len(), width: w(text) });
            } else {
                let mut cur = String::new();
                for word in text.split(' ') {
                    if w(word) > box_width {
                        overflow = true;
                    }
                    let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                    if cur.is_empty() || w(&cand) <= box_width {
                        cur = cand;
                    } else {
                        lines.push(Line { start: 0, end: 0, width: w(&cur) });
                        cur = word.to_string();
                    }
                }
                lines.push(Line { start: 0, end: 0, width: w(&cur) });
            }
            Ok(Layout {
                width: lines.iter().map(|l| l.width).fold(0.0, f32::max),
                height: lines.len() as f32 * 12.0,
                lines,
                missing_glyphs: missing,
                word_overflow: overflow,
            })
        }
    }

    fn run(v: &dyn Validator, text: &str, params: Value) -> super::super::Verdict {
        let mut fake = Fake;
        let mut r = RenderCtx {
            measurer: &mut fake,
            fonts: vec![FontId(0)],
            style: Style { size_px: 10.0, letter_spacing: 0.0, line_height: 0.0, wrap: WrapMode::None },
        };
        let ctx = TextCtx { token_key: "k", locale: "ko-KR", value: text, source_value: None };
        v.check(&ctx, &params, Some(&mut r)).unwrap().verdict
    }

    #[test]
    fn max_width() {
        assert_eq!(run(&MaxWidth, "확인", json!({ "max_width_px": 20 })), Pass);
        assert_eq!(run(&MaxWidth, "확인하기", json!({ "max_width_px": 20 })), Fail);
    }

    #[test]
    fn line_count_exactly_two() {
        let p = json!({ "box_width_px": 40, "min": 2, "max": 2 });
        assert_eq!(run(&LineCount, "확인 취소", p.clone()), Pass);
        assert_eq!(run(&LineCount, "확인", p), Fail);
    }

    #[test]
    fn overflow_and_coverage() {
        assert_eq!(run(&WordOverflow, "가나다라마바", json!({ "box_width_px": 50 })), Fail);
        assert_eq!(run(&GlyphCoverage, "확□인", json!({})), Fail);
    }

    #[test]
    fn render_validators_need_a_profile() {
        let ctx = TextCtx { token_key: "k", locale: "ko-KR", value: "x", source_value: None };
        let err = MaxWidth.check(&ctx, &json!({ "max_width_px": 10 }), None);
        assert!(matches!(err, Err(ValidateError::NeedsRender)));
    }
}
