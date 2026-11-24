//! 토큰과 언어 관리.
//!
//! 토큰 id 는 바뀌지 않고 key(토큰명)는 바뀔 수 있다. 이전 이름은 `token_key_history`에 남고
//! 다른 토큰이 재사용할 수 없다.

use std::collections::{BTreeSet, HashMap};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::json;

use crate::history::{insert_changeset, record_entity};
use crate::ids::new_id;
use crate::model::{ChangesetKind, ChangesetMeta};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Token {
    pub id: String,
    pub key: String,
    pub description: Option<String>,
    pub status: String,
    /// 태그(제약 범위 "태그"에 쓴다). 이름순.
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Locale {
    pub code: String,
    pub name: String,
    pub is_source: bool,
    pub sort_order: i64,
}

/// 언어 추가. `is_source`면 기존 원문 언어 표시는 해제한다.
pub fn add_locale(conn: &Connection, code: &str, name: &str, is_source: bool) -> Result<()> {
    let code = code.trim();
    if code.is_empty() {
        return Err(Error::Invalid("언어 코드가 비어 있음".into()));
    }
    let tx = conn.unchecked_transaction()?;
    if is_source {
        tx.execute("UPDATE locale SET is_source = 0", [])?;
    }
    tx.execute(
        "INSERT INTO locale (code, name, is_source, sort_order)
         VALUES (?1, ?2, ?3, (SELECT coalesce(max(sort_order), 0) + 1 FROM locale))",
        params![code, name, is_source as i64],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn list_locales(conn: &Connection) -> Result<Vec<Locale>> {
    let mut stmt =
        conn.prepare("SELECT code, name, is_source, sort_order FROM locale ORDER BY sort_order, code")?;
    let rows = stmt.query_map([], |r| {
        Ok(Locale {
            code: r.get(0)?,
            name: r.get(1)?,
            is_source: r.get::<_, i64>(2)? != 0,
            sort_order: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn source_locale(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT code FROM locale WHERE is_source = 1", [], |r| r.get(0))
        .optional()?)
}

/// 토큰 만들기. 생성 기록은 entity_rev 에 남긴다.
pub fn create_token(conn: &mut Connection, author: &str, key: &str, description: Option<&str>) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::Invalid("토큰명이 비어 있음".into()));
    }
    let tx = conn.transaction()?;
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("create token {key}"));
    let changeset_id = insert_changeset(&tx, &meta)?;
    let id = create_token_in(&tx, changeset_id, key, description)?;
    tx.commit()?;
    Ok(id)
}

/// 이미 만든 변경 묶음 안에서 토큰을 만든다(엑셀 가져오기처럼 한 동작이 한 묶음일 때).
pub(crate) fn create_token_in(
    tx: &Transaction,
    changeset_id: i64,
    key: &str,
    description: Option<&str>,
) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::Invalid("토큰명이 비어 있음".into()));
    }
    let id = new_id();
    tx.execute(
        "INSERT INTO token (id, key, description) VALUES (?1, ?2, ?3)",
        params![id, key, description],
    )?;
    tx.execute(
        "INSERT INTO entity_rev (changeset_id, entity_type, entity_id, op, new_json)
         VALUES (?1, 'token', ?2, 'insert', ?3)",
        params![changeset_id, id, json!({ "key": key, "description": description }).to_string()],
    )?;
    Ok(id)
}

/// 토큰 id → 현재 토큰명.
pub fn token_key(conn: &Connection, token_id: &str) -> Result<String> {
    conn.query_row("SELECT key FROM token WHERE id = ?1", [token_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("토큰 {token_id}")))
}

/// 토큰명 바꾸기. 이전 이름은 이력에 남고 다른 토큰이 재사용할 수 없다.
/// 이름이 같으면 아무것도 하지 않고 None.
pub fn rename_token(conn: &mut Connection, author: &str, token_id: &str, new_key: &str) -> Result<Option<i64>> {
    let new_key = new_key.trim();
    if new_key.is_empty() {
        return Err(Error::Invalid("토큰명이 비어 있음".into()));
    }
    let tx = conn.transaction()?;
    let old_key: String = tx
        .query_row("SELECT key FROM token WHERE id = ?1", [token_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("토큰 {token_id}")))?;
    if old_key == new_key {
        return Ok(None);
    }
    let meta = ChangesetMeta::new(author, ChangesetKind::Rename)
        .with_message(format!("rename {old_key} -> {new_key}"));
    let changeset_id = insert_changeset(&tx, &meta)?;
    // 자기 자신의 옛 이름으로 되돌아가는 경우 그 이력 행을 지운다
    tx.execute(
        "DELETE FROM token_key_history WHERE old_key = ?1 AND token_id = ?2",
        params![new_key, token_id],
    )?;
    tx.execute("UPDATE token SET key = ?1 WHERE id = ?2", params![new_key, token_id])?;
    tx.execute(
        "INSERT INTO token_key_history (old_key, token_id, changeset_id) VALUES (?1, ?2, ?3)",
        params![old_key, token_id, changeset_id],
    )?;
    tx.execute(
        "INSERT INTO entity_rev (changeset_id, entity_type, entity_id, op, old_json, new_json)
         VALUES (?1, 'token', ?2, 'update', ?3, ?4)",
        params![
            changeset_id,
            token_id,
            json!({ "key": old_key }).to_string(),
            json!({ "key": new_key }).to_string()
        ],
    )?;
    tx.commit()?;
    Ok(Some(changeset_id))
}

pub fn list_tokens(conn: &Connection) -> Result<Vec<Token>> {
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT token_id, tag FROM token_tag ORDER BY tag")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            tags.entry(r.get(0)?).or_default().push(r.get(1)?);
        }
    }
    let mut stmt = conn.prepare("SELECT id, key, description, status FROM token ORDER BY key")?;
    let rows = stmt.query_map([], |r| {
        Ok(Token {
            id: r.get(0)?,
            key: r.get(1)?,
            description: r.get(2)?,
            status: r.get(3)?,
            tags: Vec::new(),
        })
    })?;
    let mut out: Vec<Token> = rows.collect::<rusqlite::Result<_>>()?;
    for t in &mut out {
        t.tags = tags.remove(&t.id).unwrap_or_default();
    }
    Ok(out)
}

/// 프로젝트에서 쓰는 태그 전체(이름순).
pub fn list_tags(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT tag FROM token_tag ORDER BY tag")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 토큰의 태그를 통째로 바꾼다. 앞뒤 공백을 지우고 빈 태그·중복은 뺀다. 바뀐 게 없으면 None.
pub fn set_tags(conn: &mut Connection, author: &str, token_id: &str, tags: &[String]) -> Result<Option<i64>> {
    let new: BTreeSet<String> = tags
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let tx = conn.transaction()?;
    let key = token_key(&tx, token_id)?;
    let old: BTreeSet<String> = {
        let mut stmt = tx.prepare("SELECT tag FROM token_tag WHERE token_id = ?1")?;
        let rows = stmt.query_map([token_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if old == new {
        return Ok(None);
    }
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("set tags of {key}"));
    let changeset_id = insert_changeset(&tx, &meta)?;
    tx.execute("DELETE FROM token_tag WHERE token_id = ?1", [token_id])?;
    for t in &new {
        tx.execute("INSERT INTO token_tag (token_id, tag) VALUES (?1, ?2)", params![token_id, t])?;
    }
    record_entity(&tx, changeset_id, "token_tags", token_id, Some(&old), Some(&new))?;
    tx.commit()?;
    Ok(Some(changeset_id))
}

/// 토큰명으로 찾기. 현재 이름이 없으면 이전 이름에서 찾는다(엑셀 재가져오기용).
pub fn find_token_by_key(conn: &Connection, key: &str) -> Result<Option<String>> {
    let current: Option<String> = conn
        .query_row("SELECT id FROM token WHERE key = ?1", [key], |r| r.get(0))
        .optional()?;
    if current.is_some() {
        return Ok(current);
    }
    Ok(conn
        .query_row("SELECT token_id FROM token_key_history WHERE old_key = ?1", [key], |r| r.get(0))
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn rename_keeps_history_and_blocks_reuse() {
        let mut conn = db::open_in_memory().unwrap();
        let a = create_token(&mut conn, "t", "home.title", None).unwrap();
        rename_token(&mut conn, "t", &a, "home.header").unwrap();

        // 이전 이름으로도 찾을 수 있다
        assert_eq!(find_token_by_key(&conn, "home.title").unwrap(), Some(a.clone()));
        // 다른 토큰은 이전 이름을 쓸 수 없다
        assert!(create_token(&mut conn, "t", "home.title", None).is_err());
        // 자기 자신은 옛 이름으로 돌아갈 수 있다
        rename_token(&mut conn, "t", &a, "home.title").unwrap();
        assert_eq!(find_token_by_key(&conn, "home.header").unwrap(), Some(a));
    }

    #[test]
    fn tags_are_normalized_and_recorded() {
        let mut conn = db::open_in_memory().unwrap();
        let a = create_token(&mut conn, "t", "home.title", None).unwrap();
        let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(set_tags(&mut conn, "t", &a, &tags(&[" button", "title", "button", ""])).unwrap().is_some());
        assert_eq!(list_tokens(&conn).unwrap()[0].tags, vec!["button", "title"]);
        // 같으면 변경 묶음을 만들지 않는다
        assert_eq!(set_tags(&mut conn, "t", &a, &tags(&["title", "button"])).unwrap(), None);
        assert_eq!(list_tags(&conn).unwrap(), vec!["button", "title"]);
    }

    #[test]
    fn single_source_locale() {
        let conn = db::open_in_memory().unwrap();
        add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        add_locale(&conn, "en-US", "English", true).unwrap();
        assert_eq!(source_locale(&conn).unwrap().as_deref(), Some("en-US"));
    }
}
