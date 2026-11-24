//! 편집 화면용 조회: 행 = 토큰, 열 = 언어.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::tokens::{list_locales, list_tokens, source_locale, Locale, Token};
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridCell {
    pub value: String,
    pub status: String,
    pub version: i64,
    /// 원문 셀이 이 셀보다 나중에 바뀌었으면 true.
    pub outdated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridRow {
    pub token: Token,
    /// 언어 코드 → 셀. 없으면 미번역.
    pub cells: BTreeMap<String, GridCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Grid {
    pub locales: Vec<Locale>,
    pub source_locale: Option<String>,
    pub rows: Vec<GridRow>,
}

pub fn load_grid(conn: &Connection) -> Result<Grid> {
    let locales = list_locales(conn)?;
    let source = source_locale(conn)?;
    let tokens = list_tokens(conn)?;

    let mut cells: BTreeMap<String, BTreeMap<String, GridCell>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT token_id, locale, value, status, version FROM translation")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let token_id: String = r.get(0)?;
            let locale: String = r.get(1)?;
            cells.entry(token_id).or_default().insert(
                locale,
                GridCell {
                    value: r.get(2)?,
                    status: r.get(3)?,
                    version: r.get(4)?,
                    outdated: false,
                },
            );
        }
    }

    if let Some(src) = &source {
        for row in cells.values_mut() {
            if let Some(src_version) = row.get(src).map(|c| c.version) {
                for (locale, cell) in row.iter_mut() {
                    if locale != src && src_version > cell.version {
                        cell.outdated = true;
                    }
                }
            }
        }
    }

    let rows = tokens
        .into_iter()
        .map(|token| {
            let cells = cells.remove(&token.id).unwrap_or_default();
            GridRow { token, cells }
        })
        .collect();
    Ok(Grid {
        locales,
        source_locale: source,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::apply;
    use crate::model::{CellChange, CellKey, CellValue, ChangesetKind, ChangesetMeta};
    use crate::{db, tokens};

    #[test]
    fn marks_outdated_when_source_changes_later() {
        let mut conn = db::open_in_memory().unwrap();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let a = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let meta = ChangesetMeta::new("t", ChangesetKind::Edit);
        let set = |l: &str, v: &str| CellChange {
            key: CellKey::new(&a, l),
            new: Some(CellValue::translated(v)),
        };
        apply(&mut conn, &meta, &[set("ko-KR", "확인"), set("en-US", "OK")]).unwrap();
        apply(&mut conn, &meta, &[set("ko-KR", "확인하기")]).unwrap();

        let grid = load_grid(&conn).unwrap();
        let row = &grid.rows[0];
        assert!(row.cells["en-US"].outdated);
        assert!(!row.cells["ko-KR"].outdated);
    }
}
