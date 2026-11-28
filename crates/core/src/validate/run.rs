//! 전체 검증: 모든 셀(사용 중인 토큰 × 언어)에 유효 제약을 실행한다. 캐시 없이 매번 전체를 본다.
//!
//! - 미번역 셀은 `checks_missing()`인 검증기(`text.required`)만 검사한다.
//! - 렌더 검증기는 제약의 렌더 프로필로 엔진에서 잰다. 이 PC 에서 엔진 DLL 을 쓸 수 없으면
//!   건너뛰고 수와 이유를 보고한다. 프로필 설정이 잘못됐으면 검증기 오류로 보고한다.
//! - 결과에는 위반(fail)이거나 검증기 자체가 실패한 항목이 있는 셀만 담는다.

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde::Serialize;

use super::{effective_constraints, load_constraints, Finding, Registry, Severity, TextCtx, Verdict};
use crate::model::CellKey;
use crate::render::{EngineSet, RenderSession, SetupError};
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
    /// 측정 엔진을 쓸 수 없어 건너뛴 렌더 검사.
    pub skipped_render: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ValidationReport {
    pub cells: Vec<CellFindings>,
    pub summary: ValidationSummary,
    /// 렌더 검사를 건너뛴 이유(중복 없이).
    pub render_skipped: Vec<String>,
    /// 렌더 검사를 건너뛴 셀(토큰 id, 언어). 내보내기 게이트가 쓴다. 화면에는 보내지 않는다.
    #[serde(skip)]
    pub skipped_cells: Vec<(String, String)>,
}

/// `engines`가 None 이면 렌더 검사는 모두 건너뛴다.
pub fn validate_all(conn: &Connection, registry: &Registry, engines: Option<&mut EngineSet>) -> Result<ValidationReport> {
    let constraints = load_constraints(conn)?;
    let locales = list_locales(conn)?;
    let source = source_locale(conn)?;
    let now = history::current(conn)?;
    let mut render = RenderSession::new(conn, engines)?;
    let mut skipped_reasons = BTreeSet::new();
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
                let mut finding = Finding::new(c);
                let Some(v) = registry.get(&c.validator) else {
                    s.broken += 1;
                    finding.error = Some(format!("알 수 없는 검증기: {}", c.validator));
                    findings.push(finding);
                    continue;
                };
                if value.is_none() && !v.checks_missing() {
                    continue;
                }
                let ctx = TextCtx {
                    token_key: &token.key,
                    locale: &locale.code,
                    value: value.unwrap_or(""),
                    source_value: if locale.is_source { None } else { source_value },
                };
                let result = if v.needs_render() {
                    match render.for_params(&c.params, &locale.code) {
                        Err(SetupError::Unavailable(why)) => {
                            s.skipped_render += 1;
                            skipped_reasons.insert(why);
                            report.skipped_cells.push((token.id.clone(), locale.code.clone()));
                            continue;
                        }
                        Err(SetupError::Config(msg)) => Err(msg),
                        Ok(mut p) => {
                            finding.render = Some(p.info.clone());
                            v.check(&ctx, &c.params, Some(&mut p.ctx)).map_err(|e| e.to_string())
                        }
                    }
                } else {
                    v.check(&ctx, &c.params, None).map_err(|e| e.to_string())
                };
                s.checks += 1;
                match result {
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
                        finding.error = Some(e);
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
    report.render_skipped = skipped_reasons.into_iter().collect();
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

        let r = validate_all(&conn, &reg, None).unwrap();
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
        assert_eq!(r.summary.skipped_render, 0);

        delete_constraint(&mut conn, "t", &tag_len).unwrap();
        assert_eq!(validate_all(&conn, &reg, None).unwrap().summary.info, 0);
    }

    #[test]
    fn runs_render_checks_and_skips_when_engine_is_missing() {
        use crate::render::tests::{fake_engines, profile_input, temp_project};
        use crate::render::{add_font_bytes, register_engine, save_profile};

        let (dir, mut conn) = temp_project("validate");
        let (mut engines, _) = fake_engines();
        let reg = Registry::builtin();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let dll = dir.join("engines").join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();
        register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();
        let font = add_font_bytes(&mut conn, "t", "a.ttf", b"a").unwrap();
        let profile = save_profile(&mut conn, "t", &profile_input("v1", &font.id)).unwrap();
        let a = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let set = |l: &str, v: &str| CellChange {
            key: CellKey::new(&a, l),
            new: Some(CellValue::translated(v)),
        };
        apply(&mut conn, &ChangesetMeta::new("t", ChangesetKind::Edit), &[set("ko-KR", "확인"), set("en-US", "Confirm")])
            .unwrap();

        // 렌더 규칙은 프로필이 있어야 저장된다
        let no_profile = input("render.max_width", json!({ "max_width_px": 50 }), Scope::Global, Severity::Error);
        assert!(save_constraint(&mut conn, "t", &reg, &no_profile).is_err());
        let missing = input("render.max_width", json!({ "profile": "none", "max_width_px": 50 }), Scope::Global, Severity::Error);
        assert!(save_constraint(&mut conn, "t", &reg, &missing).is_err());
        // 글자 10px: 확인 20px 통과, Confirm 70px 실패
        let rule = input("render.max_width", json!({ "profile": profile, "max_width_px": 50 }), Scope::Global, Severity::Error);
        save_constraint(&mut conn, "t", &reg, &rule).unwrap();

        let r = validate_all(&conn, &reg, Some(&mut engines)).unwrap();
        assert_eq!(r.summary.checks, 2);
        assert_eq!(r.cells.len(), 1);
        assert_eq!(r.cells[0].locale, "en-US");
        let f = &r.cells[0].findings[0];
        assert_eq!(f.validator, "render.max_width");
        assert_eq!(f.render.as_ref().map(|i| (i.profile.as_str(), i.engine.as_str())), Some(("버튼", "v1")));

        // 엔진 없이 검증하면 건너뛴다
        let r = validate_all(&conn, &reg, None).unwrap();
        assert_eq!((r.summary.skipped_render, r.cells.len()), (2, 0));

        // 이 PC 에 DLL 이 없으면 이유와 함께 건너뛴다
        std::fs::remove_file(&dll).unwrap();
        let (mut fresh, _) = fake_engines();
        let r = validate_all(&conn, &reg, Some(&mut fresh)).unwrap();
        assert_eq!(r.summary.skipped_render, 2);
        assert_eq!(r.skipped_cells.len(), 2);
        assert!(r.render_skipped[0].contains("DLL이 없음"), "{:?}", r.render_skipped);
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
