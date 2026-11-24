//! 검증 프레임워크: 검증기(규칙 유형) → 제약(문구에 붙인 인스턴스) → 결과.
//!
//! 새 검증기를 추가하려면 [`Validator`]를 구현하고 [`Registry::builtin`]에 등록한다.
//! `params_schema`(JSON Schema)로 화면의 제약 입력 폼이 자동으로 만들어진다.
//! 렌더링 검증기는 [`RenderCtx`]의 `Measurer`(엔진 DLL)만 쓴다. 폭을 직접 계산하지 않는다.

mod render;
mod resolve;
mod text;

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tms_measure::{FontId, MeasureError, Measurer, Style};

pub use resolve::{effective_constraints, load_constraints, ConstraintDef, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "info" => Some(Self::Info),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub verdict: Verdict,
    pub message: Option<String>,
    /// 측정값 등(폭, 줄 수 ...). 화면에 "134px / 120px"처럼 보여준다.
    pub metrics: Value,
}

impl Outcome {
    pub fn pass(metrics: Value) -> Self {
        Self {
            verdict: Verdict::Pass,
            message: None,
            metrics,
        }
    }

    pub fn fail(message: impl Into<String>, metrics: Value) -> Self {
        Self {
            verdict: Verdict::Fail,
            message: Some(message.into()),
            metrics,
        }
    }
}

/// 검사 대상 문구.
#[derive(Debug, Clone, Copy)]
pub struct TextCtx<'a> {
    pub token_key: &'a str,
    pub locale: &'a str,
    pub value: &'a str,
    /// 원문 언어 값(상대 비교·placeholder 검사용).
    pub source_value: Option<&'a str>,
}

/// 렌더링 검증에 필요한 것: 측정 엔진, 폰트 폴백 순서, 렌더 프로필의 스타일.
pub struct RenderCtx<'m> {
    pub measurer: &'m mut dyn Measurer,
    pub fonts: Vec<FontId>,
    pub style: Style,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidateError {
    #[error("파라미터 오류: {0}")]
    Params(String),
    #[error("이 검증기는 렌더 프로필이 필요함")]
    NeedsRender,
    #[error(transparent)]
    Measure(#[from] MeasureError),
}

pub trait Validator: Send + Sync {
    fn id(&self) -> &'static str;
    fn version(&self) -> u32 {
        1
    }
    fn description(&self) -> &'static str;
    /// 제약 입력 폼을 만들 JSON Schema.
    fn params_schema(&self) -> Value;
    fn needs_render(&self) -> bool {
        false
    }
    fn check(
        &self,
        ctx: &TextCtx<'_>,
        params: &Value,
        render: Option<&mut RenderCtx<'_>>,
    ) -> Result<Outcome, ValidateError>;
}

pub(crate) fn parse_params<T: DeserializeOwned>(params: &Value) -> Result<T, ValidateError> {
    serde_json::from_value(params.clone()).map_err(|e| ValidateError::Params(e.to_string()))
}

#[derive(Default)]
pub struct Registry {
    validators: BTreeMap<&'static str, Box<dyn Validator>>,
}

impl Registry {
    /// 내장 검증기 9종(텍스트 5 + 렌더 4).
    pub fn builtin() -> Self {
        let mut r = Self::default();
        for v in text::all().into_iter().chain(render::all()) {
            r.register(v);
        }
        r
    }

    pub fn register(&mut self, v: Box<dyn Validator>) {
        self.validators.insert(v.id(), v);
    }

    pub fn get(&self, id: &str) -> Option<&dyn Validator> {
        self.validators.get(id).map(|b| b.as_ref())
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Validator> {
        self.validators.values().map(|b| b.as_ref())
    }
}

/// 제약 하나를 실행한 결과.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub constraint_id: String,
    pub validator: String,
    pub severity: Severity,
    pub outcome: Option<Outcome>,
    /// 검증기 자체가 실패한 경우(파라미터 오류, 엔진 오류 ...). 문구 위반과 구분한다.
    pub error: Option<String>,
}

/// 셀 하나에 유효 제약들을 실행한다.
pub fn run_constraints(
    registry: &Registry,
    constraints: &[&ConstraintDef],
    ctx: &TextCtx<'_>,
    mut render: Option<&mut RenderCtx<'_>>,
) -> Vec<Finding> {
    constraints
        .iter()
        .map(|c| {
            let (outcome, error) = match registry.get(&c.validator) {
                None => (None, Some(format!("알 수 없는 검증기: {}", c.validator))),
                Some(v) => match v.check(ctx, &c.params, render.as_deref_mut()) {
                    Ok(o) => (Some(o), None),
                    Err(e) => (None, Some(e.to_string())),
                },
            };
            Finding {
                constraint_id: c.id.clone(),
                validator: c.validator.clone(),
                severity: c.severity,
                outcome,
                error,
            }
        })
        .collect()
}
