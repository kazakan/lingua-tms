//! 제약 추가·수정·삭제. 변경은 entity_rev(entity_type = "constraint")에 남는다.

use std::collections::HashSet;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{load_constraints, ConstraintDef, Registry, Scope, Severity, TextCtx, ValidateError};
use crate::history::{insert_changeset, record_entity};
use crate::ids::new_id;
use crate::model::{ChangesetKind, ChangesetMeta};
use crate::tokens::{list_locales, token_key};
use crate::{Error, Result};

/// 제약 저장 입력. id 가 없으면 새로 만든다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintInput {
    pub id: Option<String>,
    pub validator: String,
    #[serde(default = "empty_object")]
    pub params: Value,
    pub scope: Scope,
    /// None = 전 언어.
    pub locale: Option<String>,
    pub severity: Severity,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub description: Option<String>,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn yes() -> bool {
    true
}

/// 제약 저장. 검증기·범위·언어를 확인하고, 텍스트 검증기는 파라미터도 미리 검사한다
/// (렌더 검증기는 측정 엔진이 있어야 파라미터를 읽으므로 실행할 때 확인된다).
pub fn save_constraint(conn: &mut Connection, author: &str, registry: &Registry, input: &ConstraintInput) -> Result<String> {
    let v = registry
        .get(&input.validator)
        .ok_or_else(|| Error::Invalid(format!("알 수 없는 검증기: {}", input.validator)))?;
    if !input.params.is_object() {
        return Err(Error::Invalid("파라미터는 객체(JSON object)여야 함".into()));
    }
    if !v.needs_render() {
        let probe = TextCtx {
            token_key: "",
            locale: "",
            value: "",
            source_value: None,
        };
        if let Err(ValidateError::Params(e)) = v.check(&probe, &input.params, None) {
            return Err(Error::Invalid(format!("파라미터 오류: {e}")));
        }
    }
    let scope = match &input.scope {
        Scope::Global => Scope::Global,
        Scope::Tag(t) if t.trim().is_empty() => return Err(Error::Invalid("태그가 비어 있음".into())),
        Scope::Tag(t) => Scope::Tag(t.trim().to_string()),
        Scope::Token(id) => {
            token_key(conn, id)?;
            Scope::Token(id.clone())
        }
    };
    if let Some(l) = &input.locale {
        let known: HashSet<String> = list_locales(conn)?.into_iter().map(|x| x.code).collect();
        if !known.contains(l) {
            return Err(Error::Invalid(format!("프로젝트에 없는 언어: {l}")));
        }
    }
    let description = input
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let tx = conn.transaction()?;
    let old = match &input.id {
        Some(id) => Some(
            load_constraints(&tx)?
                .into_iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| Error::NotFound(format!("제약 {id}")))?,
        ),
        None => None,
    };
    let id = input.id.clone().unwrap_or_else(new_id);
    let new = ConstraintDef {
        id: id.clone(),
        validator: input.validator.clone(),
        params: input.params.clone(),
        scope,
        locale: input.locale.clone(),
        severity: input.severity,
        enabled: input.enabled,
        description,
    };
    let (kind, scope_ref) = match &new.scope {
        Scope::Global => ("global", None),
        Scope::Tag(t) => ("tag", Some(t.as_str())),
        Scope::Token(t) => ("token", Some(t.as_str())),
    };
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("save constraint {}", new.validator));
    let changeset_id = insert_changeset(&tx, &meta)?;
    let row = params![
        id,
        new.validator,
        serde_json::to_string(&new.params)?,
        kind,
        scope_ref,
        new.locale,
        new.severity.as_str(),
        new.enabled as i64,
        new.description,
    ];
    if old.is_some() {
        tx.execute(
            "UPDATE constraint_def SET validator = ?2, params = ?3, scope_kind = ?4, scope_ref = ?5, locale = ?6,
                    severity = ?7, enabled = ?8, description = ?9
             WHERE id = ?1",
            row,
        )?;
    } else {
        tx.execute(
            "INSERT INTO constraint_def (id, validator, params, scope_kind, scope_ref, locale, severity, enabled, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            row,
        )?;
    }
    record_entity(&tx, changeset_id, "constraint", &id, old.as_ref(), Some(&new))?;
    tx.commit()?;
    Ok(id)
}

pub fn delete_constraint(conn: &mut Connection, author: &str, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let old = load_constraints(&tx)?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::NotFound(format!("제약 {id}")))?;
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("delete constraint {}", old.validator));
    let changeset_id = insert_changeset(&tx, &meta)?;
    tx.execute("DELETE FROM constraint_def WHERE id = ?1", [id])?;
    record_entity(&tx, changeset_id, "constraint", id, Some(&old), None)?;
    tx.commit()?;
    Ok(())
}
