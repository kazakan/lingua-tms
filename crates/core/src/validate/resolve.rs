//! 제약 해석: 문구값 하나에 실제로 적용할 제약 고르기.
//!
//! 구체성: 전역 < 태그 < 토큰, 같은 레벨이면 언어 지정이 더 구체적이다.
//! - 같은 검증기가 여러 레벨에 있으면 가장 구체적인 레벨만 남는다(같은 레벨 여러 개는 모두 적용).
//! - 가장 구체적인 제약이 비활성이면 그 검증기는 적용하지 않는다(하위에서 상위 제약 끄기).
//! - 검증기가 다르면 모두 누적 적용한다.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Severity;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "ref", rename_all = "snake_case")]
pub enum Scope {
    Global,
    Tag(String),
    Token(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintDef {
    pub id: String,
    pub validator: String,
    pub params: Value,
    pub scope: Scope,
    /// None = 전 언어.
    pub locale: Option<String>,
    pub severity: Severity,
    pub enabled: bool,
    pub description: Option<String>,
}

impl ConstraintDef {
    fn specificity(&self) -> u8 {
        let base = match self.scope {
            Scope::Global => 0,
            Scope::Tag(_) => 2,
            Scope::Token(_) => 4,
        };
        base + u8::from(self.locale.is_some())
    }

    fn applies_to(&self, token_id: &str, tags: &[String], locale: &str) -> bool {
        let scope_ok = match &self.scope {
            Scope::Global => true,
            Scope::Tag(t) => tags.iter().any(|x| x == t),
            Scope::Token(id) => id == token_id,
        };
        scope_ok && self.locale.as_deref().is_none_or(|l| l == locale)
    }
}

/// 셀(토큰 × 언어)에 적용할 유효 제약.
pub fn effective_constraints<'a>(
    all: &'a [ConstraintDef],
    token_id: &str,
    tags: &[String],
    locale: &str,
) -> Vec<&'a ConstraintDef> {
    let mut by_validator: BTreeMap<&str, Vec<&ConstraintDef>> = BTreeMap::new();
    for c in all.iter().filter(|c| c.applies_to(token_id, tags, locale)) {
        by_validator.entry(c.validator.as_str()).or_default().push(c);
    }
    let mut out = Vec::new();
    for group in by_validator.into_values() {
        let top = group.iter().map(|c| c.specificity()).max().unwrap_or(0);
        out.extend(group.into_iter().filter(|c| c.specificity() == top && c.enabled));
    }
    out
}

pub fn load_constraints(conn: &Connection) -> Result<Vec<ConstraintDef>> {
    let mut stmt = conn.prepare(
        "SELECT id, validator, params, scope_kind, scope_ref, locale, severity, enabled, description
         FROM constraint_def",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let kind: String = r.get(3)?;
        let scope_ref: Option<String> = r.get(4)?;
        let scope = match (kind.as_str(), scope_ref) {
            ("global", _) => Scope::Global,
            ("tag", Some(t)) => Scope::Tag(t),
            ("token", Some(t)) => Scope::Token(t),
            (k, _) => return Err(Error::Invalid(format!("알 수 없는 제약 범위: {k}"))),
        };
        let severity: String = r.get(6)?;
        out.push(ConstraintDef {
            id: r.get(0)?,
            validator: r.get(1)?,
            params: serde_json::from_str(&r.get::<_, String>(2)?)?,
            scope,
            locale: r.get(5)?,
            severity: Severity::parse(&severity)
                .ok_or_else(|| Error::Invalid(format!("알 수 없는 심각도: {severity}")))?,
            enabled: r.get::<_, i64>(7)? != 0,
            description: r.get(8)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(id: &str, validator: &str, scope: Scope, locale: Option<&str>, enabled: bool) -> ConstraintDef {
        ConstraintDef {
            id: id.into(),
            validator: validator.into(),
            params: json!({}),
            scope,
            locale: locale.map(Into::into),
            severity: Severity::Error,
            enabled,
            description: None,
        }
    }

    #[test]
    fn most_specific_wins_per_validator() {
        let all = vec![
            c("g-ws", "text.whitespace", Scope::Global, None, true),
            c("tag-w", "render.max_width", Scope::Tag("button".into()), None, true),
            c("tok-w", "render.max_width", Scope::Token("t1".into()), None, true),
            c("tok-de", "render.max_width", Scope::Token("t1".into()), Some("de-DE"), true),
        ];
        let tags = vec!["button".to_string()];

        let ko: Vec<_> = effective_constraints(&all, "t1", &tags, "ko-KR").iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ko, vec!["tok-w", "g-ws"]);

        let de: Vec<_> = effective_constraints(&all, "t1", &tags, "de-DE").iter().map(|c| c.id.as_str()).collect();
        assert_eq!(de, vec!["tok-de", "g-ws"]);

        let other: Vec<_> = effective_constraints(&all, "t2", &tags, "ko-KR").iter().map(|c| c.id.as_str()).collect();
        assert_eq!(other, vec!["tag-w", "g-ws"]);
    }

    #[test]
    fn disabled_specific_constraint_turns_off_inherited_one() {
        let all = vec![
            c("g-ws", "text.whitespace", Scope::Global, None, true),
            c("tok-off", "text.whitespace", Scope::Token("t1".into()), None, false),
        ];
        assert!(effective_constraints(&all, "t1", &[], "ko-KR").is_empty());
        assert_eq!(effective_constraints(&all, "t2", &[], "ko-KR").len(), 1);
    }
}
