//! 문구값 변경 이력.
//!
//! 규칙
//! - 문구값(`translation`)을 바꾸는 유일한 경로는 [`apply`]다. 직접 UPDATE/DELETE 하면 트리거가 거부한다.
//! - 이력(`translation_rev`)은 추가만 한다. 롤백도 과거 값을 새 변경 묶음으로 쓰는 방식이다.
//! - 변경 묶음 번호(`changeset.id`)가 곧 버전이다. 릴리스는 번호에 붙인 이름표다.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;

use crate::model::{CellChange, CellDiff, CellEdit, CellKey, CellValue, ChangesetKind, ChangesetMeta, RevRow};
use crate::{Error, Result};

pub(crate) fn insert_changeset(tx: &Transaction, meta: &ChangesetMeta) -> Result<i64> {
    tx.execute(
        "INSERT INTO changeset (author, kind, message, reverts_id) VALUES (?1, ?2, ?3, ?4)",
        params![meta.author, meta.kind.as_str(), meta.message, meta.reverts_id],
    )?;
    Ok(tx.last_insert_rowid())
}

/// 문구값 밖의 엔티티(토큰, 제약, 내보내기 대상 ...) 변경을 entity_rev 에 남긴다. 둘 다 None 이면 아무것도 안 한다.
pub(crate) fn record_entity<T: Serialize>(
    tx: &Transaction,
    changeset_id: i64,
    entity_type: &str,
    entity_id: &str,
    old: Option<&T>,
    new: Option<&T>,
) -> Result<()> {
    let op = match (old, new) {
        (None, Some(_)) => "insert",
        (Some(_), Some(_)) => "update",
        (Some(_), None) => "delete",
        (None, None) => return Ok(()),
    };
    let to_json = |v: Option<&T>| v.map(serde_json::to_string).transpose();
    tx.execute(
        "INSERT INTO entity_rev (changeset_id, entity_type, entity_id, op, old_json, new_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![changeset_id, entity_type, entity_id, op, to_json(old)?, to_json(new)?],
    )?;
    Ok(())
}

fn cell(value: Option<String>, status: Option<String>) -> Option<CellValue> {
    match (value, status) {
        (Some(value), Some(status)) => Some(CellValue { value, status }),
        _ => None,
    }
}

/// 셀의 현재 값.
pub fn read_cell(conn: &Connection, key: &CellKey) -> Result<Option<CellValue>> {
    Ok(conn
        .query_row(
            "SELECT value, status FROM translation WHERE token_id = ?1 AND locale = ?2",
            params![key.token_id, key.locale],
            |r| {
                Ok(CellValue {
                    value: r.get(0)?,
                    status: r.get(1)?,
                })
            },
        )
        .optional()?)
}

/// 문구값을 바꾸는 유일한 경로. 한 트랜잭션에서 changeset → rev → translation 순서로 쓴다.
///
/// 실제로 바뀐 셀이 없으면 변경 묶음을 만들지 않고 `None`을 돌려준다.
pub fn apply(conn: &mut Connection, meta: &ChangesetMeta, changes: &[CellChange]) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let changeset_id = insert_changeset(&tx, meta)?;
    let written = write_changes_in(&tx, changeset_id, changes)?; // 오류면 tx 가 drop 되면서 롤백
    if written == 0 {
        return Ok(None); // tx 가 drop 되면서 롤백된다
    }
    tx.commit()?;
    Ok(Some(changeset_id))
}

/// 화면 편집(셀 하나 또는 여러 셀 붙여넣기)을 변경 묶음 하나로 쓴다.
///
/// 값이 지금과 같은 셀은 건너뛴다. 그래서 같은 값을 다시 붙여넣어도 상태(승인 등)가 풀리지 않는다.
/// 값이 바뀐 셀은 상태가 `translated`가 된다. 빈 문자열은 미번역으로 되돌린다.
pub fn apply_edits(conn: &mut Connection, meta: &ChangesetMeta, edits: &[CellEdit]) -> Result<Option<i64>> {
    let mut changes = Vec::with_capacity(edits.len());
    for e in edits {
        let key = CellKey::new(&e.token_id, &e.locale);
        let old = read_cell(conn, &key)?;
        let new = (!e.value.is_empty()).then(|| CellValue::translated(&e.value));
        if old.as_ref().map(|c| c.value.as_str()) == new.as_ref().map(|c| c.value.as_str()) {
            continue;
        }
        changes.push(CellChange { key, new });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    apply(conn, meta, &changes)
}

/// 이미 연 트랜잭션 안에서 변경 묶음 `changeset_id`에 셀 변경을 쓴다(가져오기처럼 토큰 생성과 한 묶음일 때).
pub(crate) fn write_changes_in(tx: &Transaction, changeset_id: i64, changes: &[CellChange]) -> Result<usize> {
    let mut seen = HashSet::new();
    for change in changes {
        if !seen.insert(&change.key) {
            return Err(Error::Invalid(format!(
                "같은 셀이 한 변경 묶음에 두 번 있음: {} / {}",
                change.key.token_id, change.key.locale
            )));
        }
    }
    write_changes(tx, changeset_id, changes)
}

fn write_changes(tx: &Transaction, changeset_id: i64, changes: &[CellChange]) -> Result<usize> {
    let mut written = 0;
    for change in changes {
        let key = &change.key;
        let old = read_cell(tx, key)?;
        if old == change.new {
            continue;
        }
        let op = match (&old, &change.new) {
            (None, Some(_)) => "insert",
            (Some(_), Some(_)) => "update",
            (Some(_), None) => "delete",
            (None, None) => continue,
        };
        // 1) 이력 먼저
        tx.execute(
            "INSERT INTO translation_rev
               (changeset_id, token_id, locale, op, old_value, new_value, old_status, new_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                changeset_id,
                key.token_id,
                key.locale,
                op,
                old.as_ref().map(|c| &c.value),
                change.new.as_ref().map(|c| &c.value),
                old.as_ref().map(|c| &c.status),
                change.new.as_ref().map(|c| &c.status),
            ],
        )?;
        // 2) 현재 값 갱신
        match (&old, &change.new) {
            (None, Some(new)) => {
                tx.execute(
                    "INSERT INTO translation (token_id, locale, value, status, version)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![key.token_id, key.locale, new.value, new.status, changeset_id],
                )?;
            }
            (Some(_), Some(new)) => {
                tx.execute(
                    "UPDATE translation SET value = ?3, status = ?4, version = ?5
                     WHERE token_id = ?1 AND locale = ?2",
                    params![key.token_id, key.locale, new.value, new.status, changeset_id],
                )?;
            }
            (Some(_), None) => {
                tx.execute(
                    "DELETE FROM translation WHERE token_id = ?1 AND locale = ?2",
                    params![key.token_id, key.locale],
                )?;
            }
            (None, None) => unreachable!(),
        }
        written += 1;
    }
    Ok(written)
}

/// 가장 최근 변경 묶음 번호.
pub fn latest_changeset(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT max(id) FROM changeset", [], |r| r.get(0))?)
}

/// 현재 전체 상태.
pub fn current(conn: &Connection) -> Result<BTreeMap<CellKey, CellValue>> {
    let mut stmt = conn.prepare("SELECT token_id, locale, value, status FROM translation")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            CellKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            CellValue {
                value: r.get(2)?,
                status: r.get(3)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 변경 묶음 `changeset_id` 시점의 전체 상태. 이력에서 계산한다.
pub fn state_at(conn: &Connection, changeset_id: i64) -> Result<BTreeMap<CellKey, CellValue>> {
    let mut stmt = conn.prepare(
        "WITH ranked AS (
           SELECT token_id, locale, op, new_value, new_status,
                  ROW_NUMBER() OVER (PARTITION BY token_id, locale ORDER BY changeset_id DESC) AS rn
           FROM translation_rev
           WHERE changeset_id <= ?1)
         SELECT token_id, locale, new_value, new_status
         FROM ranked WHERE rn = 1 AND op <> 'delete'",
    )?;
    let rows = stmt.query_map([changeset_id], |r| {
        Ok((
            CellKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            CellValue {
                value: r.get(2)?,
                status: r.get(3)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `from` 시점과 `to` 시점 사이에 바뀐 셀. 상태를 재계산하지 않고 사이 구간의 이력만 본다.
pub fn diff(conn: &Connection, from: i64, to: i64) -> Result<Vec<CellDiff>> {
    if from > to {
        return Ok(diff(conn, to, from)?
            .into_iter()
            .map(|d| CellDiff {
                key: d.key,
                old: d.new,
                new: d.old,
            })
            .collect());
    }
    let mut stmt = conn.prepare(
        "SELECT token_id, locale, old_value, old_status, new_value, new_status
         FROM translation_rev
         WHERE changeset_id > ?1 AND changeset_id <= ?2
         ORDER BY token_id, locale, changeset_id",
    )?;
    let mut acc: BTreeMap<CellKey, (Option<CellValue>, Option<CellValue>)> = BTreeMap::new();
    let mut rows = stmt.query(params![from, to])?;
    while let Some(r) = rows.next()? {
        let key = CellKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?);
        let old = cell(r.get(2)?, r.get(3)?);
        let new = cell(r.get(4)?, r.get(5)?);
        acc.entry(key).or_insert((old, None)).1 = new;
    }
    Ok(acc
        .into_iter()
        .filter(|(_, (old, new))| old != new)
        .map(|(key, (old, new))| CellDiff { key, old, new })
        .collect())
}

/// 셀 하나의 이력(최신 순).
pub fn cell_history(conn: &Connection, key: &CellKey) -> Result<Vec<RevRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.changeset_id, r.op, r.old_value, r.old_status, r.new_value, r.new_status,
                c.author, c.created_at
         FROM translation_rev r JOIN changeset c ON c.id = r.changeset_id
         WHERE r.token_id = ?1 AND r.locale = ?2
         ORDER BY r.changeset_id DESC",
    )?;
    let rows = stmt.query_map(params![key.token_id, key.locale], |r| {
        Ok(RevRow {
            changeset_id: r.get(0)?,
            op: r.get(1)?,
            old: cell(r.get(2)?, r.get(3)?),
            new: cell(r.get(4)?, r.get(5)?),
            author: r.get(6)?,
            created_at: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RevertOutcome {
    /// 되돌리기로 만든 새 변경 묶음. 되돌릴 셀이 없으면 None.
    pub changeset_id: Option<i64>,
    /// 대상 변경 묶음 뒤에 다시 바뀌어 건너뛴 셀.
    pub conflicts: Vec<CellKey>,
}

/// 변경 묶음 하나를 되돌린다(예: 잘못된 엑셀 가져오기). 그 뒤에 다시 바뀐 셀은 충돌로 보고하고 건너뛴다.
pub fn revert_changeset(conn: &mut Connection, target: i64, author: &str) -> Result<RevertOutcome> {
    let rows: Vec<(CellKey, Option<CellValue>, Option<CellValue>)> = {
        let mut stmt = conn.prepare(
            "SELECT token_id, locale, old_value, old_status, new_value, new_status
             FROM translation_rev WHERE changeset_id = ?1",
        )?;
        let mapped = stmt.query_map([target], |r| {
            Ok((
                CellKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                cell(r.get(2)?, r.get(3)?),
                cell(r.get(4)?, r.get(5)?),
            ))
        })?;
        mapped.collect::<rusqlite::Result<_>>()?
    };
    if rows.is_empty() {
        return Err(Error::NotFound(format!("변경 묶음 {target}의 문구 변경")));
    }

    let mut changes = Vec::new();
    let mut conflicts = Vec::new();
    for (key, old, new) in rows {
        if read_cell(conn, &key)? != new {
            conflicts.push(key);
        } else {
            changes.push(CellChange { key, new: old });
        }
    }
    let meta = ChangesetMeta {
        author: author.to_string(),
        kind: ChangesetKind::Revert,
        message: Some(format!("revert changeset {target}")),
        reverts_id: Some(target),
    };
    let changeset_id = if changes.is_empty() {
        None
    } else {
        apply(conn, &meta, &changes)?
    };
    Ok(RevertOutcome {
        changeset_id,
        conflicts,
    })
}

/// 전체를 `changeset_id` 시점으로 되돌린다. 현재와 다른 셀만 새 변경 묶음으로 쓴다.
pub fn rollback_to(conn: &mut Connection, changeset_id: i64, author: &str) -> Result<Option<i64>> {
    let target = state_at(conn, changeset_id)?;
    let now = current(conn)?;
    let keys: BTreeSet<&CellKey> = target.keys().chain(now.keys()).collect();
    let changes: Vec<CellChange> = keys
        .into_iter()
        .filter(|k| target.get(*k) != now.get(*k))
        .map(|k| CellChange {
            key: k.clone(),
            new: target.get(k).cloned(),
        })
        .collect();
    let meta = ChangesetMeta::new(author, ChangesetKind::Revert)
        .with_message(format!("rollback to changeset {changeset_id}"));
    apply(conn, &meta, &changes)
}

/// 릴리스 만들기. `changeset_id`가 None이면 가장 최근 변경 묶음에 붙인다.
pub fn create_release(conn: &Connection, name: &str, changeset_id: Option<i64>, note: Option<&str>) -> Result<i64> {
    let id = match changeset_id {
        Some(id) => id,
        None => latest_changeset(conn)?.ok_or_else(|| Error::Invalid("변경 이력이 없음".into()))?,
    };
    conn.execute(
        "INSERT INTO release (name, changeset_id, note) VALUES (?1, ?2, ?3)",
        params![name, id, note],
    )?;
    Ok(id)
}

pub fn release_changeset(conn: &Connection, name: &str) -> Result<i64> {
    conn.query_row("SELECT changeset_id FROM release WHERE name = ?1", [name], |r| r.get(0))
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("릴리스 {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, tokens};

    fn setup() -> (Connection, String, String) {
        let mut conn = db::open_in_memory().unwrap();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let a = tokens::create_token(&mut conn, "tester", "home.title", None).unwrap();
        let b = tokens::create_token(&mut conn, "tester", "home.sub", None).unwrap();
        (conn, a, b)
    }

    fn set(key: &CellKey, v: &str) -> CellChange {
        CellChange {
            key: key.clone(),
            new: Some(CellValue::translated(v)),
        }
    }

    fn meta(kind: ChangesetKind) -> ChangesetMeta {
        ChangesetMeta::new("tester", kind)
    }

    #[test]
    fn apply_writes_history_and_skips_noop() {
        let (mut conn, a, _) = setup();
        let ko = CellKey::new(&a, "ko-KR");
        let c1 = apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&ko, "확인")]).unwrap();
        assert!(c1.is_some());
        // 같은 값이면 변경 묶음을 만들지 않는다
        let c2 = apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&ko, "확인")]).unwrap();
        assert_eq!(c2, None);
        assert_eq!(cell_history(&conn, &ko).unwrap().len(), 1);
    }

    #[test]
    fn direct_writes_are_rejected() {
        let (mut conn, a, _) = setup();
        let ko = CellKey::new(&a, "ko-KR");
        apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&ko, "확인")]).unwrap();
        assert!(conn
            .execute("UPDATE translation SET value = 'x' WHERE token_id = ?1", [&a])
            .is_err());
        assert!(conn
            .execute("DELETE FROM translation WHERE token_id = ?1", [&a])
            .is_err());
    }

    #[test]
    fn revert_reports_conflicts_and_rollback_restores() {
        let (mut conn, a, b) = setup();
        let a_ko = CellKey::new(&a, "ko-KR");
        let a_en = CellKey::new(&a, "en-US");
        let b_ko = CellKey::new(&b, "ko-KR");

        let c1 = apply(
            &mut conn,
            &meta(ChangesetKind::Import),
            &[set(&a_ko, "확인"), set(&a_en, "OK"), set(&b_ko, "취소")],
        )
        .unwrap()
        .unwrap();
        create_release(&conn, "v1.0", Some(c1), None).unwrap();

        // 잘못된 가져오기
        let bad = apply(
            &mut conn,
            &meta(ChangesetKind::Import),
            &[
                set(&a_ko, "확인하기"),
                CellChange {
                    key: b_ko.clone(),
                    new: None,
                },
            ],
        )
        .unwrap()
        .unwrap();
        // 그 뒤에 a_ko 를 다시 고침
        apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&a_ko, "확인!!")]).unwrap();

        let out = revert_changeset(&mut conn, bad, "tester").unwrap();
        assert_eq!(out.conflicts, vec![a_ko.clone()]);
        assert_eq!(read_cell(&conn, &b_ko).unwrap(), Some(CellValue::translated("취소")));
        assert_eq!(read_cell(&conn, &a_ko).unwrap(), Some(CellValue::translated("확인!!")));

        // v1.0 으로 전체 롤백
        let v1 = release_changeset(&conn, "v1.0").unwrap();
        rollback_to(&mut conn, v1, "tester").unwrap();
        assert_eq!(current(&conn).unwrap(), state_at(&conn, v1).unwrap());
    }

    #[test]
    fn diff_uses_only_the_range() {
        let (mut conn, a, b) = setup();
        let a_ko = CellKey::new(&a, "ko-KR");
        let b_ko = CellKey::new(&b, "ko-KR");
        let c1 = apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&a_ko, "가"), set(&b_ko, "나")])
            .unwrap()
            .unwrap();
        apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&a_ko, "다")]).unwrap();
        let c3 = apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&a_ko, "가")])
            .unwrap()
            .unwrap();
        // c1 → c3: a_ko 는 다시 "가"가 되어 차이가 없다
        assert!(diff(&conn, c1, c3).unwrap().is_empty());
        // 0 → c1: 두 셀 추가
        let d = diff(&conn, 0, c1).unwrap();
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.old.is_none()));
        // 거꾸로 비교하면 old/new 가 뒤집힌다
        assert!(diff(&conn, c1, 0).unwrap().iter().all(|x| x.new.is_none()));
    }

    #[test]
    fn pasted_edits_are_one_changeset_and_keep_status_of_unchanged_cells() {
        let (mut conn, a, b) = setup();
        let a_ko = CellKey::new(&a, "ko-KR");
        let approved = CellValue {
            value: "확인".into(),
            status: "approved".into(),
        };
        apply(
            &mut conn,
            &meta(ChangesetKind::Edit),
            &[CellChange {
                key: a_ko.clone(),
                new: Some(approved.clone()),
            }],
        )
        .unwrap();

        let edit = |t: &str, l: &str, v: &str| CellEdit {
            token_id: t.into(),
            locale: l.into(),
            value: v.into(),
        };
        // 같은 값(a_ko)은 건너뛰고, 나머지 두 셀만 변경 묶음 하나로
        let cs = apply_edits(
            &mut conn,
            &meta(ChangesetKind::Edit),
            &[edit(&a, "ko-KR", "확인"), edit(&a, "en-US", "OK"), edit(&b, "ko-KR", "취소")],
        )
        .unwrap()
        .unwrap();
        assert_eq!(read_cell(&conn, &a_ko).unwrap(), Some(approved));
        assert_eq!(diff(&conn, cs - 1, cs).unwrap().len(), 2);

        // 빈 문자열은 미번역으로 되돌린다
        apply_edits(&mut conn, &meta(ChangesetKind::Edit), &[edit(&b, "ko-KR", "")]).unwrap();
        assert_eq!(read_cell(&conn, &CellKey::new(&b, "ko-KR")).unwrap(), None);
        // 바뀐 게 없으면 변경 묶음을 만들지 않는다
        assert_eq!(apply_edits(&mut conn, &meta(ChangesetKind::Edit), &[edit(&b, "ko-KR", "")]).unwrap(), None);
    }

    #[test]
    fn duplicate_keys_in_one_changeset_are_rejected() {
        let (mut conn, a, _) = setup();
        let ko = CellKey::new(&a, "ko-KR");
        let err = apply(&mut conn, &meta(ChangesetKind::Edit), &[set(&ko, "가"), set(&ko, "나")]);
        assert!(matches!(err, Err(Error::Invalid(_))));
    }
}
