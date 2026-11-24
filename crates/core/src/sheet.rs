//! 스프레드시트(xlsx, xlsm, xlsb, xls, ods) 읽기.
//!
//! calamine 으로 모든 칸을 문자열로 읽는다. 열·행 번호는 엑셀과 같게(A열 = 0, 1행 = 0) 맞춘다.
//! 열 매핑([`ColumnMapping`])을 적용하면 값 표([`ValueTable`])가 된다.

use std::path::Path;

use calamine::{open_workbook_auto, Data, Reader};
use serde::{Deserialize, Serialize};

use crate::values::ValueTable;
use crate::{Error, Result};

/// 시트 하나의 미리보기(열 매핑 화면용).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetPreview {
    pub name: String,
    /// 전체 행 수(빈 앞쪽 행 포함).
    pub rows: usize,
    /// 가장 긴 행의 열 수.
    pub cols: usize,
    /// 앞쪽 몇 행.
    pub head: Vec<Vec<String>>,
}

/// 시트를 문자열 표로 읽는다. 엑셀 좌표와 맞도록 앞쪽 빈 행·열을 채운다.
pub fn read_sheet(path: &Path, sheet: &str) -> Result<Vec<Vec<String>>> {
    let mut wb = open_workbook_auto(path)?;
    let range = wb.worksheet_range(sheet)?;
    let (r0, c0) = range.start().unwrap_or((0, 0));
    let mut rows: Vec<Vec<String>> = vec![Vec::new(); r0 as usize];
    for row in range.rows() {
        let mut out = vec![String::new(); c0 as usize];
        out.extend(row.iter().map(cell_text));
        // 뒤쪽 빈 칸은 잘라 둔다(열 수 계산이 실제 내용 기준이 되게).
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        rows.push(out);
    }
    Ok(rows)
}

fn cell_text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

/// 파일 안 모든 시트의 미리보기.
pub fn workbook_preview(path: &Path, head_rows: usize) -> Result<Vec<SheetPreview>> {
    let names = open_workbook_auto(path)?.sheet_names();
    names
        .into_iter()
        .map(|name| {
            let rows = read_sheet(path, &name)?;
            Ok(SheetPreview {
                cols: rows.iter().map(Vec::len).max().unwrap_or(0),
                rows: rows.len(),
                head: rows.into_iter().take(head_rows).collect(),
                name,
            })
        })
        .collect()
}

/// 어느 열이 토큰명이고 어느 열이 어느 언어인지.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub sheet: String,
    /// 머리글 행(0부터). 데이터는 그다음 행부터다.
    pub header_row: usize,
    pub key_col: usize,
    pub locale_cols: Vec<LocaleColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocaleColumn {
    pub col: usize,
    pub locale: String,
}

/// 열 매핑을 적용해 값 표를 만든다. 토큰명은 앞뒤 공백을 지우고, 비어 있는 행은 건너뛴다.
/// 값은 그대로 둔다(앞뒤 공백도 검증 대상이다).
pub fn extract(rows: &[Vec<String>], m: &ColumnMapping) -> Result<ValueTable> {
    let mut seen = std::collections::HashSet::new();
    for lc in &m.locale_cols {
        if lc.col == m.key_col {
            return Err(Error::Invalid(format!("{}열이 토큰명과 언어에 같이 지정됨", column_name(lc.col))));
        }
        if !seen.insert(lc.locale.as_str()) {
            return Err(Error::Invalid(format!("언어 {}가 두 열에 지정됨", lc.locale)));
        }
    }
    let mut table = ValueTable::new(m.locale_cols.iter().map(|c| c.locale.clone()).collect());
    fn cell(row: &[String], col: usize) -> &str {
        row.get(col).map(String::as_str).unwrap_or("")
    }
    for row in rows.iter().skip(m.header_row + 1) {
        let key = cell(row, m.key_col).trim();
        if key.is_empty() {
            continue;
        }
        table.push_row(key, m.locale_cols.iter().map(|lc| (lc.locale.as_str(), cell(row, lc.col))));
    }
    Ok(table)
}

/// 0 → A, 25 → Z, 26 → AA.
pub fn column_name(mut col: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;

    /// 테스트용 임시 xlsx 파일. 칸은 (행, 열, 값).
    pub(crate) fn temp_xlsx(sheet: &str, cells: &[(u32, u16, &str)]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("tms-test-{}.xlsx", crate::ids::new_id()));
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet().set_name(sheet).unwrap();
        for (r, c, v) in cells {
            ws.write_string(*r, *c, *v).unwrap();
        }
        wb.save(&path).unwrap();
        path
    }

    #[test]
    fn reads_with_excel_coordinates_and_extracts_mapping() {
        // B2 부터 시작하는 표(앞쪽 빈 행·열)
        let path = temp_xlsx(
            "Strings",
            &[
                (1, 1, "Key"),
                (1, 2, "Korean"),
                (1, 3, "English"),
                (2, 1, "common.ok"),
                (2, 2, "확인"),
                (2, 3, "OK"),
                (3, 1, "  common.cancel "),
                (3, 2, "취소"),
                (5, 2, "토큰명 없는 행"),
            ],
        );
        let preview = workbook_preview(&path, 3).unwrap();
        assert_eq!(preview[0].name, "Strings");
        assert_eq!(preview[0].head[1], vec!["", "Key", "Korean", "English"]);

        let rows = read_sheet(&path, "Strings").unwrap();
        let m = ColumnMapping {
            sheet: "Strings".into(),
            header_row: 1,
            key_col: 1,
            locale_cols: vec![
                LocaleColumn { col: 2, locale: "ko-KR".into() },
                LocaleColumn { col: 3, locale: "en-US".into() },
            ],
        };
        let t = extract(&rows, &m).unwrap();
        assert_eq!(t.keys, vec!["common.ok", "common.cancel"]);
        assert_eq!(t.get("common.ok", "en-US"), Some("OK"));
        assert_eq!(t.get("common.cancel", "en-US"), None);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }
}
