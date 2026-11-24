//! 가져오기: 값 표(엑셀 등)를 DB와 비교해 계획을 만들고, 적용은 Import 변경 묶음 하나로 쓴다.
//!
//! - 토큰명은 이전 이름까지 찾는다(`tokens::find_token_by_key`). 이전 이름으로 찾은 행은 `renamed`로 알린다.
//! - 없는 토큰은 옵션에 따라 새로 만든다. 토큰 생성도 같은 변경 묶음에 들어간다.
//! - 빈 칸은 기본으로 건너뛴다. `clear_empty`면 그 칸의 값을 지운다(미번역으로).
//! - 값이 바뀐 칸만 쓴다. 바뀐 칸의 상태는 translated 가 된다.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::history::{self, insert_changeset, write_changes_in};
use crate::model::{CellChange, CellKey, CellValue, ChangesetKind, ChangesetMeta};
use crate::sheet::{self, ColumnMapping};
use crate::tokens::{self, create_token_in, find_token_by_key, list_locales};
use crate::values::ValueTable;
use crate::{export, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOptions {
    /// 프로젝트에 없는 토큰명을 새 토큰으로 만든다.
    #[serde(default)]
    pub create_missing_tokens: bool,
    /// 매핑한 언어 열의 빈 칸을 "값 지우기"로 본다. 끄면 빈 칸은 건너뛴다.
    #[serde(default)]
    pub clear_empty: bool,
}

/// 가져올 값의 출처.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportSource {
    /// 열 매핑으로 읽는 엑셀(관리용 엑셀 등). 토큰명으로 찾는다.
    Mapped { path: String, mapping: ColumnMapping },
    /// 내보내기 대상의 제품 형식 파일(역가져오기). 고정된 제품 키로 토큰을 찾는다.
    Target { path: String, target_id: String },
}

impl ImportSource {
    pub fn load(&self, conn: &Connection) -> Result<ValueTable> {
        match self {
            Self::Mapped { path, mapping } => {
                let rows = sheet::read_sheet(Path::new(path), &mapping.sheet)?;
                sheet::extract(&rows, mapping)
            }
            Self::Target { path, target_id } => export::read_target_file(conn, target_id, Path::new(path)),
        }
    }

    /// 변경 묶음 메시지에 쓸 이름(파일 이름).
    pub fn label(&self) -> String {
        let path = match self {
            Self::Mapped { path, .. } | Self::Target { path, .. } => path,
        };
        Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlannedChange {
    /// 새로 만들 토큰이면 None.
    pub token_id: Option<String>,
    /// 현재 토큰명(새 토큰이면 표의 이름).
    pub token_key: String,
    pub locale: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenamedKey {
    pub sheet_key: String,
    pub current_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImportPlan {
    pub changes: Vec<PlannedChange>,
    pub new_tokens: Vec<String>,
    /// 이전 토큰명으로 찾은 행.
    pub renamed: Vec<RenamedKey>,
    /// 없는 토큰이라 건너뜀(새로 만들기를 끈 경우).
    pub skipped_tokens: Vec<String>,
    /// 표의 다른 행과 같은 토큰을 가리켜 건너뜀(이전 이름과 현재 이름이 둘 다 있는 경우 등).
    pub conflicting_keys: Vec<String>,
    /// 프로젝트에 없는 언어 열(건너뜀).
    pub unknown_locales: Vec<String>,
    /// 표 안에서 두 번 이상 나온 토큰명(뒤 행의 값을 씀).
    pub duplicate_keys: Vec<String>,
    /// 토큰을 찾지 못한 제품 키(역가져오기). 그 행은 건너뛴다.
    pub unresolved_keys: Vec<String>,
    /// 값이 같아 쓰지 않는 칸 수.
    pub unchanged: usize,
    /// 표의 토큰 행 수(중복 제외).
    pub rows: usize,
}

/// 값 표를 지금 DB와 비교한 가져오기 계획(미리보기). DB는 바꾸지 않는다.
pub fn plan_import(conn: &Connection, table: &ValueTable, opts: &ImportOptions) -> Result<ImportPlan> {
    let known: HashSet<String> = list_locales(conn)?.into_iter().map(|l| l.code).collect();
    let now = history::current(conn)?;
    let mut plan = ImportPlan {
        duplicate_keys: table.duplicate_keys.clone(),
        unresolved_keys: table.unresolved.clone(),
        rows: table.keys.len(),
        ..ImportPlan::default()
    };
    let mut locales = Vec::new();
    for l in &table.locales {
        if known.contains(l) {
            locales.push(l.as_str());
        } else {
            plan.unknown_locales.push(l.clone());
        }
    }

    let mut seen_tokens = HashSet::new();
    for key in &table.keys {
        let (token_id, token_key) = match find_token_by_key(conn, key)? {
            Some(id) => {
                if !seen_tokens.insert(id.clone()) {
                    plan.conflicting_keys.push(key.clone());
                    continue;
                }
                let current = tokens::token_key(conn, &id)?;
                if &current != key {
                    plan.renamed.push(RenamedKey {
                        sheet_key: key.clone(),
                        current_key: current.clone(),
                    });
                }
                (Some(id), current)
            }
            None if opts.create_missing_tokens => {
                plan.new_tokens.push(key.clone());
                (None, key.clone())
            }
            None => {
                plan.skipped_tokens.push(key.clone());
                continue;
            }
        };
        for &locale in &locales {
            let new = table.get(key, locale).map(str::to_string);
            if new.is_none() && !opts.clear_empty {
                continue;
            }
            let old = token_id
                .as_ref()
                .and_then(|id| now.get(&CellKey::new(id, locale)))
                .map(|c| c.value.clone());
            if old == new {
                if new.is_some() {
                    plan.unchanged += 1;
                }
                continue;
            }
            plan.changes.push(PlannedChange {
                token_id: token_id.clone(),
                token_key: token_key.clone(),
                locale: locale.to_string(),
                old,
                new,
            });
        }
    }
    Ok(plan)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportResult {
    /// 바뀐 게 없으면 None.
    pub changeset_id: Option<i64>,
    pub created_tokens: usize,
    pub changed_cells: usize,
}

/// 가져오기 적용. 계획을 트랜잭션 안에서 다시 계산해(미리보기 뒤에 바뀐 내용 반영) 변경 묶음 하나로 쓴다.
pub fn apply_import(
    conn: &mut Connection,
    author: &str,
    table: &ValueTable,
    opts: &ImportOptions,
    label: &str,
) -> Result<ImportResult> {
    let tx = conn.transaction()?;
    let plan = plan_import(&tx, table, opts)?;
    if plan.changes.is_empty() && plan.new_tokens.is_empty() {
        return Ok(ImportResult {
            changeset_id: None,
            created_tokens: 0,
            changed_cells: 0,
        });
    }
    let meta = ChangesetMeta::new(author, ChangesetKind::Import).with_message(format!("import {label}"));
    let changeset_id = insert_changeset(&tx, &meta)?;
    let mut created: HashMap<&str, String> = HashMap::new();
    for key in &plan.new_tokens {
        created.insert(key, create_token_in(&tx, changeset_id, key, None)?);
    }
    let changes: Vec<CellChange> = plan
        .changes
        .iter()
        .map(|c| {
            let id = c
                .token_id
                .clone()
                .unwrap_or_else(|| created[c.token_key.as_str()].clone());
            CellChange {
                key: CellKey::new(id, &c.locale),
                new: c.new.clone().map(CellValue::translated),
            }
        })
        .collect();
    let written = write_changes_in(&tx, changeset_id, &changes)?;
    tx.commit()?;
    Ok(ImportResult {
        changeset_id: Some(changeset_id),
        created_tokens: created.len(),
        changed_cells: written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{apply, latest_changeset, read_cell};
    use crate::{db, tokens};

    struct Fixture {
        conn: Connection,
        title: String,
        ok: String,
    }

    /// ko/en, home.title(ko=제목, 승인됨), common.ok → common.confirm 으로 이름 변경.
    fn fixture() -> Fixture {
        let mut conn = db::open_in_memory().unwrap();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let title = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let ok = tokens::create_token(&mut conn, "t", "common.ok", None).unwrap();
        tokens::rename_token(&mut conn, "t", &ok, "common.confirm").unwrap();
        apply(
            &mut conn,
            &ChangesetMeta::new("t", ChangesetKind::Edit),
            &[CellChange {
                key: CellKey::new(&title, "ko-KR"),
                new: Some(CellValue {
                    value: "제목".into(),
                    status: "approved".into(),
                }),
            }],
        )
        .unwrap();
        Fixture { conn, title, ok }
    }

    fn table(rows: &[(&str, &str, &str, &str)]) -> ValueTable {
        let mut t = ValueTable::new(vec!["ko-KR".into(), "en-US".into(), "fr-FR".into()]);
        for (key, ko, en, fr) in rows {
            t.push_row(key, [("ko-KR", *ko), ("en-US", *en), ("fr-FR", *fr)]);
        }
        t
    }

    #[test]
    fn plan_finds_old_names_new_tokens_and_unknown_locales() {
        let f = fixture();
        let t = table(&[
            ("home.title", "제목", "Title", ""),
            ("common.ok", "확인", "", ""),
            ("new.token", "새 문구", "", "Nouveau"),
        ]);
        let opts = ImportOptions {
            create_missing_tokens: true,
            clear_empty: false,
        };
        let plan = plan_import(&f.conn, &t, &opts).unwrap();
        let changes: Vec<_> = plan
            .changes
            .iter()
            .map(|c| (c.token_key.as_str(), c.locale.as_str(), c.new.as_deref()))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("home.title", "en-US", Some("Title")),
                ("common.confirm", "ko-KR", Some("확인")),
                ("new.token", "ko-KR", Some("새 문구")),
            ]
        );
        assert_eq!(plan.unchanged, 1);
        assert_eq!(plan.new_tokens, vec!["new.token"]);
        assert_eq!(
            plan.renamed,
            vec![RenamedKey {
                sheet_key: "common.ok".into(),
                current_key: "common.confirm".into()
            }]
        );
        assert_eq!(plan.unknown_locales, vec!["fr-FR"]);

        // 새로 만들기를 끄면 건너뛴다
        let plan = plan_import(&f.conn, &t, &ImportOptions::default()).unwrap();
        assert_eq!(plan.skipped_tokens, vec!["new.token"]);
        assert_eq!(plan.changes.len(), 2);
    }

    #[test]
    fn apply_is_one_changeset_and_keeps_status_of_unchanged_cells() {
        let mut f = fixture();
        let before = latest_changeset(&f.conn).unwrap().unwrap();
        let t = table(&[("home.title", "제목", "Title", ""), ("new.token", "새 문구", "", "")]);
        let opts = ImportOptions {
            create_missing_tokens: true,
            clear_empty: false,
        };
        let out = apply_import(&mut f.conn, "t", &t, &opts, "strings.xlsx").unwrap();
        assert_eq!(out.changeset_id, Some(before + 1));
        assert_eq!((out.created_tokens, out.changed_cells), (1, 2));
        assert_eq!(latest_changeset(&f.conn).unwrap(), Some(before + 1));

        // 토큰 생성 기록도 같은 변경 묶음
        let new_id = tokens::find_token_by_key(&f.conn, "new.token").unwrap().unwrap();
        let cs: i64 = f
            .conn
            .query_row(
                "SELECT changeset_id FROM entity_rev WHERE entity_type = 'token' AND entity_id = ?1",
                [&new_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cs, before + 1);
        // 값이 같은 칸은 승인 상태 그대로
        let ko = read_cell(&f.conn, &CellKey::new(&f.title, "ko-KR")).unwrap().unwrap();
        assert_eq!(ko.status, "approved");

        // 같은 파일을 다시 가져오면 바뀐 게 없다
        let again = apply_import(&mut f.conn, "t", &t, &opts, "strings.xlsx").unwrap();
        assert_eq!(again.changeset_id, None);
    }

    #[test]
    fn clear_empty_and_conflicting_rows() {
        let f = fixture();
        // common.ok(이전 이름)와 common.confirm(현재 이름)이 같은 토큰을 가리킨다
        let t = table(&[("home.title", "", "", ""), ("common.ok", "확인", "", ""), ("common.confirm", "확인2", "", "")]);
        let opts = ImportOptions {
            create_missing_tokens: false,
            clear_empty: true,
        };
        let plan = plan_import(&f.conn, &t, &opts).unwrap();
        assert_eq!(plan.conflicting_keys, vec!["common.confirm"]);
        let deletes: Vec<_> = plan.changes.iter().filter(|c| c.new.is_none()).collect();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].token_id.as_deref(), Some(f.title.as_str()));
        assert!(plan.changes.iter().any(|c| c.token_id.as_deref() == Some(f.ok.as_str())));
    }
}
