//! 전체 검증: 모든 셀(사용 중인 토큰 × 언어)에 유효 제약을 실행한다. 캐시 없이 매번 전체를 본다.
//!
//! - 미번역 셀은 `checks_missing()`인 검증기(`text.required`)만 검사한다.
//! - 렌더 검증기는 측정 엔진 연결(후순위) 전까지 실행하지 않고 건너뛴 수만 센다.
//! - 결과에는 위반(fail)이거나 검증기 자체가 실패한 항목이 있는 셀만 담는다.

use rusqlite::Connection;
use serde::Serialize;

use super::{effective_constraints, load_constraints, Finding, Registry, Severity, TextCtx, Verdict};
use crate::model::CellKey;
use crate::tokens::{list_locales, list_tokens, source_locale};
use crate::{history, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellFindings {
    pub token_id: String,
    pub token_key: String,
    pub locale: String,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ValidationSummary {
    /// 검사한 셀 수(토큰 × 언어).
    pub cells: usize,
    /// 실행한 검사 수(셀 × 유효 제약).
    pub checks: usize,
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    /// 검증기 자체가 실패한 검사(파라미터 오류 등).
    pub broken: usize,
    /// 측정 엔진이 없어 건너뛴 렌더 검사.
    pub skipped_render: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ValidationReport {
    pub cells: Vec<CellFindings>,
    pub summary: ValidationSummary,
}

pub fn validate_all(conn: &Connection, registry: &Registry) -> Result<ValidationReport> {
    let constraints = load_constraints(conn)?;
    let locales = list_locales(conn)?;
    let source = source_locale(conn)?;
    let now = history::current(conn)?;
    let mut report = ValidationReport::default();
    let s = &mut report.summary;

    for token in list_tokens(conn)? {
        if token.status == "deprecated" {
            continue;
        }
        let value_of = |locale: &str| now.get(&CellKey::new(&token.id, locale)).map(|c| c.value.as_str());
        let source_value = source.as_deref().and_then(value_of);
        for locale in &locales {
            s.cells += 1;
            let value = value_of(&locale.code);
            let mut findings = Vec::new();
            for c in effective_constraints(&constraints, &token.id, &token.tags, &locale.code) {
                let mut finding = Finding {
                    constraint_id: c.id.clone(),
                    validator: c.validator.clone(),
                    severity: c.severity,
                    outcome: None,
                    error: None,
                };
                let Some(v) = registry.get(&c.validator) else {
                    s.broken += 1;
                    finding.error = Some(format!("알 수 없는 검증기: {}", c.validator));
                    findings.push(finding);
                    continue;
                };
                if v.needs_render() {
                    s.skipped_render += 1;
                    continue;
                }
                if value.is_none() && !v.checks_missing() {
                    continue;
                }
                let ctx = TextCtx {
                    token_key: &token.key,
                    locale: &locale.code,
                    value: value.unwrap_or(""),
                    source_value: if locale.is_source { None } else { source_value },
                };
                s.checks += 1;
                match v.check(&ctx, &c.params, None) {
                    Ok(o) if o.verdict == Verdict::Fail => {
                        match c.severity {
                            Severity::Error => s.error += 1,
                            Severity::Warning => s.warning += 1,
                            Severity::Info => s.info += 1,
                        }
                        finding.outcome = Some(o);
                        findings.push(finding);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        s.broken += 1;
                        finding.error = Some(e.to_string());
                        findings.push(finding);
                    }
                }
            }
            if !findings.is_empty() {
                report.cells.push(CellFindings {
                    token_id: token.id.clone(),
                    token_key: token.key.clone(),
                    locale: locale.code.clone(),
                    findings,
                });
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::history::apply;
    use crate::model::{CellChange, CellValue, ChangesetKind, ChangesetMeta};
    use crate::validate::{delete_constraint, save_constraint, ConstraintInput, Scope};
    use crate::{db, tokens};

    fn input(validator: &str, params: serde_json::Value, scope: Scope, severity: Severity) -> ConstraintInput {
        ConstraintInput {
            id: None,
            validator: validator.into(),
            params,
            scope,
            locale: None,
            severity,
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn runs_effective_constraints_and_skips_render_and_missing() {
        let mut conn = db::open_in_memory().unwrap();
        let reg = Registry::builtin();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let a = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let b = tokens::create_token(&mut conn, "t", "common.ok", None).unwrap();
        tokens::set_tags(&mut conn, "t", &b, &["button".to_string()]).unwrap();
        let set = |t: &str, l: &str, v: &str| CellChange {
            key: CellKey::new(t, l),
            new: Some(CellValue::translated(v)),
        };
        apply(
            &mut conn,
            &ChangesetMeta::new("t", ChangesetKind::Edit),
            &[
                set(&a, "ko-KR", "{n}개의 제목"),
                set(&a, "en-US", "Title "),
                set(&b, "ko-KR", "확인"),
            ],
        )
        .unwrap();

        save_constraint(&mut conn, "t", &reg, &input("text.required", json!({}), Scope::Global, Severity::Error)).unwrap();
        save_constraint(&mut conn, "t", &reg, &input("text.placeholder_match", json!({}), Scope::Global, Severity::Error)).unwrap();
        save_constraint(&mut conn, "t", &reg, &input("text.whitespace", json!({}), Scope::Global, Severity::Warning)).unwrap();
        // 버튼 태그: 최대 1자(info)
        let tag_len = save_constraint(
            &mut conn,
            "t",
            &reg,
            &input("text.length", json!({ "max": 1 }), Scope::Tag("button".into()), Severity::Info),
        )
        .unwrap();
        save_constraint(
            &mut conn,
            "t",
            &reg,
            &input("render.max_width", json!({ "max_width_px": 100 }), Scope::Global, Severity::Error),
        )
        .unwrap();

        let r = validate_all(&conn, &reg).unwrap();
        let mut got: Vec<(String, String, String)> = r
            .cells
            .iter()
            .flat_map(|c| c.findings.iter().map(move |f| (c.token_key.clone(), c.locale.clone(), f.validator.clone())))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                ("common.ok".into(), "en-US".into(), "text.required".into()), // 미번역
                ("common.ok".into(), "ko-KR".into(), "text.length".into()),   // 2자 > 1자
                ("home.title".into(), "en-US".into(), "text.placeholder_match".into()),
                ("home.title".into(), "en-US".into(), "text.whitespace".into()), // 끝 공백
            ]
        );
        assert_eq!((r.summary.error, r.summary.warning, r.summary.info), (2, 1, 1));
        assert_eq!(r.summary.cells, 4);
        assert_eq!(r.summary.skipped_render, 4);

        delete_constraint(&mut conn, "t", &tag_len).unwrap();
        assert_eq!(validate_all(&conn, &reg).unwrap().summary.info, 0);
    }

    #[test]
    fn save_checks_validator_params_and_scope() {
        let mut conn = db::open_in_memory().unwrap();
        let reg = Registry::builtin();
        let bad_regex = input("text.pattern", json!({ "pattern": "(" }), Scope::Global, Severity::Error);
        assert!(save_constraint(&mut conn, "t", &reg, &bad_regex).is_err());
        let unknown = input("text.nope", json!({}), Scope::Global, Severity::Error);
        assert!(save_constraint(&mut conn, "t", &reg, &unknown).is_err());
        let no_token = input("text.required", json!({}), Scope::Token("missing".into()), Severity::Error);
        assert!(save_constraint(&mut conn, "t", &reg, &no_token).is_err());

        // 수정은 entity_rev 에 update 로 남는다
        let id = save_constraint(&mut conn, "t", &reg, &input("text.required", json!({}), Scope::Global, Severity::Error)).unwrap();
        let mut edit = input("text.required", json!({}), Scope::Global, Severity::Warning);
        edit.id = Some(id.clone());
        save_constraint(&mut conn, "t", &reg, &edit).unwrap();
        let ops: Vec<String> = conn
            .prepare("SELECT op FROM entity_rev WHERE entity_type = 'constraint' AND entity_id = ?1 ORDER BY changeset_id")
            .unwrap()
            .query_map([&id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(ops, vec!["insert", "update"]);
        assert_eq!(load_constraints(&conn).unwrap()[0].severity, Severity::Warning);
    }
}
