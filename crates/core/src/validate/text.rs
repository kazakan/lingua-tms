//! 텍스트 기반 검증기(폰트 불필요).

use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

use super::{parse_params, Outcome, RenderCtx, TextCtx, ValidateError, Validator};

pub(super) fn all() -> Vec<Box<dyn Validator>> {
    vec![
        Box::new(Required),
        Box::new(Length),
        Box::new(PlaceholderMatch),
        Box::new(Pattern),
        Box::new(Whitespace),
    ]
}

/// 사람이 보는 글자 수(이모지·결합 문자 1자).
pub fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

// ── text.required ──────────────────────────────
struct Required;

impl Validator for Required {
    fn id(&self) -> &'static str {
        "text.required"
    }
    fn description(&self) -> &'static str {
        "빈 값 금지"
    }
    fn params_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn check(&self, ctx: &TextCtx<'_>, _: &Value, _: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        Ok(if ctx.value.trim().is_empty() {
            Outcome::fail("값이 비어 있음", json!({}))
        } else {
            Outcome::pass(json!({}))
        })
    }
}

// ── text.length ────────────────────────────────
struct Length;

#[derive(Deserialize)]
struct LengthParams {
    min: Option<usize>,
    max: Option<usize>,
    #[serde(default)]
    unit: LengthUnit,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum LengthUnit {
    #[default]
    Grapheme,
    Utf8Bytes,
}

impl Validator for Length {
    fn id(&self) -> &'static str {
        "text.length"
    }
    fn description(&self) -> &'static str {
        "글자 수 최소·최대"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "min": { "type": "integer", "minimum": 0, "title": "최소" },
                "max": { "type": "integer", "minimum": 0, "title": "최대" },
                "unit": { "type": "string", "enum": ["grapheme", "utf8_bytes"], "default": "grapheme", "title": "세는 방식" }
            }
        })
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, _: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: LengthParams = parse_params(params)?;
        let n = match p.unit {
            LengthUnit::Grapheme => grapheme_count(ctx.value),
            LengthUnit::Utf8Bytes => ctx.value.len(),
        };
        let metrics = json!({ "length": n, "min": p.min, "max": p.max });
        if let Some(min) = p.min {
            if n < min {
                return Ok(Outcome::fail(format!("{n}자 < 최소 {min}자"), metrics));
            }
        }
        if let Some(max) = p.max {
            if n > max {
                return Ok(Outcome::fail(format!("{n}자 > 최대 {max}자"), metrics));
            }
        }
        Ok(Outcome::pass(metrics))
    }
}

// ── text.placeholder_match ─────────────────────
struct PlaceholderMatch;

/// `{name}`, `%s`, `%d`, `%1$s` 형식. `%%`는 placeholder가 아니다.
fn placeholders(s: &str) -> Vec<String> {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = PATTERN.get_or_init(|| Regex::new(r"\{[A-Za-z0-9_.]+\}|%(?:\d+\$)?[sdifu@]").unwrap());
    let cleaned = s.replace("%%", "");
    let mut found: Vec<String> = re.find_iter(&cleaned).map(|m| m.as_str().to_string()).collect();
    found.sort();
    found
}

impl Validator for PlaceholderMatch {
    fn id(&self) -> &'static str {
        "text.placeholder_match"
    }
    fn description(&self) -> &'static str {
        "원문과 placeholder 집합 일치"
    }
    fn params_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn check(&self, ctx: &TextCtx<'_>, _: &Value, _: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let Some(source) = ctx.source_value else {
            return Ok(Outcome::pass(json!({})));
        };
        let expected = placeholders(source);
        let found = placeholders(ctx.value);
        let metrics = json!({ "expected": expected, "found": found });
        Ok(if expected == found {
            Outcome::pass(metrics)
        } else {
            Outcome::fail("원문과 placeholder가 다름", metrics)
        })
    }
}

// ── text.pattern ───────────────────────────────
struct Pattern;

#[derive(Deserialize)]
struct PatternParams {
    pattern: String,
    #[serde(default)]
    mode: PatternMode,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum PatternMode {
    #[default]
    Forbid,
    Require,
}

impl Validator for Pattern {
    fn id(&self) -> &'static str {
        "text.pattern"
    }
    fn description(&self) -> &'static str {
        "정규식 필수·금지 패턴"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": { "type": "string", "title": "정규식" },
                "mode": { "type": "string", "enum": ["forbid", "require"], "default": "forbid", "title": "방식" }
            }
        })
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, _: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: PatternParams = parse_params(params)?;
        let re = Regex::new(&p.pattern).map_err(|e| ValidateError::Params(e.to_string()))?;
        let matched = re.is_match(ctx.value);
        Ok(match (p.mode, matched) {
            (PatternMode::Forbid, true) => Outcome::fail("금지 패턴이 있음", json!({ "pattern": p.pattern })),
            (PatternMode::Require, false) => Outcome::fail("필수 패턴이 없음", json!({ "pattern": p.pattern })),
            _ => Outcome::pass(json!({ "pattern": p.pattern })),
        })
    }
}

// ── text.whitespace ────────────────────────────
struct Whitespace;

#[derive(Deserialize, Default)]
struct WhitespaceParams {
    #[serde(default)]
    allow_newlines: bool,
}

impl Validator for Whitespace {
    fn id(&self) -> &'static str {
        "text.whitespace"
    }
    fn description(&self) -> &'static str {
        "앞뒤 공백, 연속 공백, 줄바꿈 문자"
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "allow_newlines": { "type": "boolean", "default": false, "title": "줄바꿈 문자 허용" }
            }
        })
    }
    fn check(&self, ctx: &TextCtx<'_>, params: &Value, _: Option<&mut RenderCtx<'_>>) -> Result<Outcome, ValidateError> {
        let p: WhitespaceParams = parse_params(params)?;
        let v = ctx.value;
        let mut problems = Vec::new();
        if v != v.trim() {
            problems.push("앞뒤 공백");
        }
        if v.contains("  ") {
            problems.push("연속 공백");
        }
        if !p.allow_newlines && (v.contains('\n') || v.contains('\r')) {
            problems.push("줄바꿈 문자");
        }
        Ok(if problems.is_empty() {
            Outcome::pass(json!({}))
        } else {
            Outcome::fail(problems.join(", "), json!({ "problems": problems }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(value: &'a str, source: Option<&'a str>) -> TextCtx<'a> {
        TextCtx {
            token_key: "k",
            locale: "en-US",
            value,
            source_value: source,
        }
    }

    fn verdict(v: &dyn Validator, value: &str, source: Option<&str>, params: Value) -> super::super::Verdict {
        v.check(&ctx(value, source), &params, None).unwrap().verdict
    }

    use super::super::Verdict::{Fail, Pass};

    #[test]
    fn length_counts_graphemes() {
        // 이모지 1개 + 한글 2자 = 3자
        assert_eq!(verdict(&Length, "👍🏽확인", None, json!({ "max": 3 })), Pass);
        assert_eq!(verdict(&Length, "확인하기", None, json!({ "max": 3 })), Fail);
    }

    #[test]
    fn placeholders_must_match_source() {
        assert_eq!(verdict(&PlaceholderMatch, "Hi {name}, 100%%", Some("{name}님 안녕"), json!({})), Pass);
        assert_eq!(verdict(&PlaceholderMatch, "Hi", Some("{name}님 안녕"), json!({})), Fail);
        assert_eq!(verdict(&PlaceholderMatch, "%1$s / %2$s", Some("%2$s / %1$s"), json!({})), Pass);
    }

    #[test]
    fn whitespace_and_required() {
        assert_eq!(verdict(&Whitespace, " 확인", None, json!({})), Fail);
        assert_eq!(verdict(&Whitespace, "확인", None, json!({})), Pass);
        assert_eq!(verdict(&Required, "  ", None, json!({})), Fail);
    }

    #[test]
    fn pattern_modes() {
        assert_eq!(verdict(&Pattern, "TODO 번역", None, json!({ "pattern": "TODO" })), Fail);
        assert_eq!(verdict(&Pattern, "확인", None, json!({ "pattern": "^\\S", "mode": "require" })), Pass);
        assert!(Pattern.check(&ctx("x", None), &json!({}), None).is_err());
    }
}
