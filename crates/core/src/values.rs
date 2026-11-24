//! 값 표: 토큰명 × 언어 → 값.
//!
//! 엑셀 파일, 제품 형식 파일, DB 시점을 모두 이 형태로 바꾼 뒤 같은 방식으로 비교한다(CLAUDE.md "비교").

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValueTable {
    /// 표에 있는 언어 열(순서 유지).
    pub locales: Vec<String>,
    /// 표에 나온 순서대로의 토큰명(중복 제거).
    pub keys: Vec<String>,
    /// (토큰명, 언어) → 값. 빈 셀은 넣지 않는다.
    pub values: BTreeMap<(String, String), String>,
    /// 같은 토큰명이 두 번 이상 나온 경우. 뒤에 나온 행의 값을 쓴다.
    pub duplicate_keys: Vec<String>,
}

impl ValueTable {
    pub fn new(locales: Vec<String>) -> Self {
        Self {
            locales,
            ..Self::default()
        }
    }

    /// 행 하나를 넣는다. 빈 값은 건너뛴다. 이미 있는 토큰명이면 중복으로 기록하고 값을 덮어쓴다.
    pub fn push_row<'a>(&mut self, key: &str, cells: impl IntoIterator<Item = (&'a str, &'a str)>) {
        if self.keys.iter().any(|k| k == key) {
            if !self.duplicate_keys.iter().any(|k| k == key) {
                self.duplicate_keys.push(key.to_string());
            }
        } else {
            self.keys.push(key.to_string());
        }
        for (locale, value) in cells {
            let slot = (key.to_string(), locale.to_string());
            if value.is_empty() {
                self.values.remove(&slot);
            } else {
                self.values.insert(slot, value.to_string());
            }
        }
    }

    pub fn get(&self, key: &str, locale: &str) -> Option<&str> {
        self.values
            .get(&(key.to_string(), locale.to_string()))
            .map(String::as_str)
    }
}

/// 값 표 두 개의 차이 한 칸.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueDiff {
    pub key: String,
    pub locale: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// `old` → `new` 로 바뀐 칸. 두 표의 토큰명·언어를 합친 범위에서 비교한다.
pub fn diff_tables(old: &ValueTable, new: &ValueTable) -> Vec<ValueDiff> {
    let slots: BTreeSet<&(String, String)> = old.values.keys().chain(new.values.keys()).collect();
    slots
        .into_iter()
        .filter_map(|slot| {
            let a = old.values.get(slot);
            let b = new.values.get(slot);
            (a != b).then(|| ValueDiff {
                key: slot.0.clone(),
                locale: slot.1.clone(),
                old: a.cloned(),
                new: b.cloned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_keep_last_value_and_diff_covers_both_sides() {
        let mut a = ValueTable::new(vec!["ko".into(), "en".into()]);
        a.push_row("ok", [("ko", "확인"), ("en", "OK")]);
        a.push_row("cancel", [("ko", "취소"), ("en", "")]);
        a.push_row("ok", [("ko", "확인!"), ("en", "OK")]);
        assert_eq!(a.keys, vec!["ok", "cancel"]);
        assert_eq!(a.duplicate_keys, vec!["ok"]);
        assert_eq!(a.get("ok", "ko"), Some("확인!"));
        assert_eq!(a.get("cancel", "en"), None);

        let mut b = ValueTable::new(vec!["ko".into(), "en".into()]);
        b.push_row("ok", [("ko", "확인!"), ("en", "Okay")]);
        b.push_row("new", [("ko", "새것")]);
        let d = diff_tables(&a, &b);
        let summary: Vec<_> = d
            .iter()
            .map(|x| (x.key.as_str(), x.locale.as_str(), x.old.as_deref(), x.new.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("cancel", "ko", Some("취소"), None),
                ("new", "ko", None, Some("새것")),
                ("ok", "en", Some("OK"), Some("Okay")),
            ]
        );
    }
}
