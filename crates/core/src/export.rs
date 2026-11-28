//! 제품 내보내기와 포맷 어댑터.
//!
//! - 내보내기 대상(`export_target`)마다 포맷 어댑터와 설정(JSON)이 있다. 기본 어댑터는 설정형 표(`table`).
//!   형식이 안 맞는 제품은 외부 명령 어댑터(`command`, [`command`] 모듈)로 사용자가 만든 프로그램에 맡기거나,
//!   [`FormatAdapter`]를 구현한 Rust 어댑터를 [`adapters`]에 추가한다.
//! - 제품 키(`token_external_key`)는 처음 내보낼 때 정해져 고정된다. 규칙(접두어 등)은 아직 키가 없는
//!   토큰의 제안에만 쓴다. 고정 전에 키를 직접 지정할 수도 있다([`fix_external_key`]).
//! - 같은 어댑터가 제품 형식 파일을 다시 읽는다(역가져오기). 토큰은 고정된 제품 키로만 찾는다.

pub mod command;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::history::{insert_changeset, record_entity};
use crate::ids::new_id;
use crate::model::{ChangesetKind, ChangesetMeta};
use crate::tokens::{list_locales, list_tokens, source_locale, token_key};
use crate::render::EngineSet;
use crate::validate::{validate_all, Registry, Severity};
use crate::values::ValueTable;
use crate::{history, sheet, Error, Result};

// ── 대상 ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTarget {
    pub id: String,
    pub name: String,
    pub adapter: String,
    pub config: Value,
}

/// 대상 만들기·고치기 입력. id 가 없으면 새로 만든다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportTargetInput {
    pub id: Option<String>,
    pub name: String,
    pub adapter: String,
    pub config: Value,
}

pub fn list_targets(conn: &Connection) -> Result<Vec<ExportTarget>> {
    let mut stmt = conn.prepare("SELECT id, name, adapter, config FROM export_target ORDER BY name")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(ExportTarget {
            id: r.get(0)?,
            name: r.get(1)?,
            adapter: r.get(2)?,
            config: serde_json::from_str(&r.get::<_, String>(3)?)?,
        });
    }
    Ok(out)
}

pub fn get_target(conn: &Connection, id: &str) -> Result<ExportTarget> {
    list_targets(conn)?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| Error::NotFound(format!("내보내기 대상 {id}")))
}

/// 대상 저장. 설정은 어댑터가 확인한다. 기록은 entity_rev 에 남는다.
pub fn save_target(conn: &mut Connection, author: &str, input: &ExportTargetInput) -> Result<String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("대상 이름이 비어 있음".into()));
    }
    let adapter = adapter(&input.adapter)?;
    let known: HashSet<String> = list_locales(conn)?.into_iter().map(|l| l.code).collect();
    adapter.check_config(&input.config, &known)?;

    let tx = conn.transaction()?;
    let old = match &input.id {
        Some(id) => Some(get_target(&tx, id)?),
        None => None,
    };
    let id = input.id.clone().unwrap_or_else(new_id);
    let new = ExportTarget {
        id: id.clone(),
        name: name.to_string(),
        adapter: input.adapter.clone(),
        config: input.config.clone(),
    };
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("save export target {name}"));
    let changeset_id = insert_changeset(&tx, &meta)?;
    let config = serde_json::to_string(&new.config)?;
    if old.is_some() {
        tx.execute(
            "UPDATE export_target SET name = ?2, adapter = ?3, config = ?4 WHERE id = ?1",
            params![id, new.name, new.adapter, config],
        )?;
    } else {
        tx.execute(
            "INSERT INTO export_target (id, name, adapter, config) VALUES (?1, ?2, ?3, ?4)",
            params![id, new.name, new.adapter, config],
        )?;
    }
    record_entity(&tx, changeset_id, "export_target", &id, old.as_ref(), Some(&new))?;
    tx.commit()?;
    Ok(id)
}

/// 대상 삭제. 이미 고정한 제품 키가 있으면 거부한다(키는 고정이므로).
pub fn delete_target(conn: &mut Connection, author: &str, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let old = get_target(&tx, id)?;
    let keys: i64 = tx.query_row("SELECT count(*) FROM token_external_key WHERE target_id = ?1", [id], |r| r.get(0))?;
    if keys > 0 {
        return Err(Error::Invalid(format!(
            "이미 내보낸 제품 키 {keys}개가 고정되어 있어 삭제할 수 없음"
        )));
    }
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin).with_message(format!("delete export target {}", old.name));
    let changeset_id = insert_changeset(&tx, &meta)?;
    tx.execute("DELETE FROM export_target WHERE id = ?1", [id])?;
    record_entity(&tx, changeset_id, "export_target", id, Some(&old), None::<&ExportTarget>)?;
    tx.commit()?;
    Ok(())
}

// ── 제품 키 ───────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyCase {
    #[default]
    Keep,
    Upper,
    Lower,
}

/// 아직 제품 키가 없는 토큰의 키를 제안하는 규칙.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyRule {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub case: KeyCase,
    /// 토큰명의 '.' 을 이 글자로 바꾼다. None 이면 그대로.
    #[serde(default)]
    pub dot_replacement: Option<String>,
}

pub fn suggest_key(rule: &KeyRule, token_key: &str) -> String {
    let mut s = match &rule.dot_replacement {
        Some(r) => token_key.replace('.', r),
        None => token_key.to_string(),
    };
    s = match rule.case {
        KeyCase::Keep => s,
        KeyCase::Upper => s.to_uppercase(),
        KeyCase::Lower => s.to_lowercase(),
    };
    format!("{}{}", rule.prefix, s)
}

/// 대상의 고정된 제품 키: 토큰 id → 키.
pub fn fixed_keys(conn: &Connection, target_id: &str) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT token_id, external_key FROM token_external_key WHERE target_id = ?1")?;
    let rows = stmt.query_map([target_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 아직 키가 없는 토큰에 제품 키를 직접 지정해 고정한다(기존 제품 키를 이어 쓸 때).
pub fn fix_external_key(conn: &mut Connection, author: &str, target_id: &str, token_id: &str, key: &str) -> Result<i64> {
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::Invalid("제품 키가 비어 있음".into()));
    }
    let tx = conn.transaction()?;
    let target = get_target(&tx, target_id)?;
    if let Some(existing) = fixed_keys(&tx, target_id)?.get(token_id) {
        return Err(Error::Invalid(format!("이미 고정된 키가 있음: {existing}")));
    }
    let used: Option<String> = tx
        .query_row(
            "SELECT token_id FROM token_external_key WHERE target_id = ?1 AND external_key = ?2",
            params![target_id, key],
            |r| r.get(0),
        )
        .optional()?;
    if used.is_some() {
        return Err(Error::Invalid(format!("다른 토큰이 이미 쓰는 키: {key}")));
    }
    let meta = ChangesetMeta::new(author, ChangesetKind::Admin)
        .with_message(format!("fix product key {key} for {}", target.name));
    let changeset_id = insert_changeset(&tx, &meta)?;
    insert_key(&tx, changeset_id, target_id, token_id, key)?;
    tx.commit()?;
    Ok(changeset_id)
}

fn insert_key(tx: &Transaction, changeset_id: i64, target_id: &str, token_id: &str, key: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO token_external_key (target_id, external_key, token_id) VALUES (?1, ?2, ?3)",
        params![target_id, key, token_id],
    )?;
    record_entity(
        tx,
        changeset_id,
        "token_external_key",
        &format!("{target_id}:{token_id}"),
        None,
        Some(&json!({ "target_id": target_id, "token_id": token_id, "external_key": key })),
    )
}

// ── 내보내기 ──────────────────────────────────

/// 내보낼 한 행.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportRow {
    pub token_id: String,
    pub token_key: String,
    pub external_key: String,
    /// 이미 고정된 키면 true, 이번에 처음 정해지는 제안 키면 false.
    pub fixed: bool,
    /// 언어 → 값. 값이 없으면 빠진다.
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportPreview {
    pub target: ExportTarget,
    pub locales: Vec<String>,
    pub rows: Vec<ExportRow>,
    /// 이번 내보내기로 새로 고정될 키 수.
    pub new_keys: usize,
}

/// 내보낼 내용 계산(파일을 쓰지 않음). 키가 없는 토큰은 규칙으로 제안하고, 겹치면 `_2`, `_3`을 붙인다.
pub fn plan_export(conn: &Connection, target_id: &str) -> Result<ExportPreview> {
    let target = get_target(conn, target_id)?;
    let adapter = adapter(&target.adapter)?;
    let opts = adapter.options(&target.config)?;
    let fixed = fixed_keys(conn, target_id)?;
    let mut used: HashSet<String> = fixed.values().cloned().collect();
    let now = history::current(conn)?;
    let source = source_locale(conn)?;

    let mut rows = Vec::new();
    let mut new_keys = 0;
    for token in list_tokens(conn)? {
        if token.status == "deprecated" && !opts.include_deprecated {
            continue;
        }
        let (external_key, is_fixed) = match fixed.get(&token.id) {
            Some(k) => (k.clone(), true),
            None => {
                let base = suggest_key(&opts.key_rule, &token.key);
                let mut candidate = base.clone();
                let mut n = 2;
                while used.contains(&candidate) {
                    candidate = format!("{base}_{n}");
                    n += 1;
                }
                used.insert(candidate.clone());
                new_keys += 1;
                (candidate, false)
            }
        };
        let value_of = |locale: &str| now.get(&crate::model::CellKey::new(&token.id, locale)).map(|c| c.value.clone());
        let mut values = BTreeMap::new();
        for locale in &opts.locales {
            let v = value_of(locale).or_else(|| {
                opts.fallback_to_source
                    .then(|| source.as_deref().and_then(value_of))
                    .flatten()
            });
            if let Some(v) = v {
                values.insert(locale.clone(), v);
            }
        }
        rows.push(ExportRow {
            token_id: token.id,
            token_key: token.key,
            external_key,
            fixed: is_fixed,
            values,
        });
    }
    Ok(ExportPreview {
        target,
        locales: opts.locales,
        rows,
        new_keys,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportReport {
    pub rows: usize,
    pub new_keys: usize,
    /// 기록을 남긴 변경 묶음(새 키 고정, 경고 사유). 남길 게 없으면 None.
    pub changeset_id: Option<i64>,
}

/// 내보내기를 막거나 사유가 필요한 검증 결과 한 건.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportIssue {
    pub token_key: String,
    pub locale: String,
    pub validator: String,
    pub message: String,
}

/// 내보낼 셀(토큰 × 대상 언어)의 검증 결과. error 는 내보내기를 막고, warning 은 사유를 적어야 진행한다.
/// 측정 엔진을 쓸 수 없어 건너뛴 렌더 검사도 확인하지 못한 것이므로 warning 처럼 사유를 받는다.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExportCheck {
    pub errors: Vec<ExportIssue>,
    pub warnings: Vec<ExportIssue>,
    /// 내보낼 셀에서 건너뛴 렌더 검사 수.
    pub skipped_render: usize,
    /// 건너뛴 이유.
    pub render_skipped: Vec<String>,
}

impl ExportCheck {
    /// 사유를 적어야 내보낼 수 있는가.
    pub fn needs_note(&self) -> bool {
        !self.warnings.is_empty() || self.skipped_render > 0
    }
}

/// `engines`가 None 이면 렌더 검사는 건너뛴 것으로 센다.
pub fn check_export(
    conn: &Connection,
    registry: &Registry,
    engines: Option<&mut EngineSet>,
    target_id: &str,
) -> Result<ExportCheck> {
    let preview = plan_export(conn, target_id)?;
    let tokens: HashSet<&str> = preview.rows.iter().map(|r| r.token_id.as_str()).collect();
    let locales: HashSet<&str> = preview.locales.iter().map(String::as_str).collect();
    let report = validate_all(conn, registry, engines)?;
    let mut out = ExportCheck {
        skipped_render: report
            .skipped_cells
            .iter()
            .filter(|(t, l)| tokens.contains(t.as_str()) && locales.contains(l.as_str()))
            .count(),
        ..Default::default()
    };
    if out.skipped_render > 0 {
        out.render_skipped = report.render_skipped.clone();
    }
    for cell in report
        .cells
        .iter()
        .filter(|c| tokens.contains(c.token_id.as_str()) && locales.contains(c.locale.as_str()))
    {
        for f in &cell.findings {
            let issue = ExportIssue {
                token_key: cell.token_key.clone(),
                locale: cell.locale.clone(),
                validator: f.validator.clone(),
                // 검증기 자체 오류도 확인하지 못한 것이므로 제약의 심각도대로 다룬다.
                message: f
                    .outcome
                    .as_ref()
                    .and_then(|o| o.message.clone())
                    .or_else(|| f.error.as_ref().map(|e| format!("검증기 오류: {e}")))
                    .unwrap_or_default(),
            };
            match f.severity {
                Severity::Error => out.errors.push(issue),
                Severity::Warning => out.warnings.push(issue),
                Severity::Info => {}
            }
        }
    }
    Ok(out)
}

/// 파일로 내보낸다. 새 제품 키 고정과 파일 쓰기를 한 트랜잭션으로 묶는다(파일 쓰기가 실패하면 키도 고정하지 않음).
///
/// 검증 error 가 있으면 거부한다. warning 이 있으면 `note`(사유)가 있어야 하고, 사유는 변경 묶음 메시지에 남긴다.
pub fn export_to_file(
    conn: &mut Connection,
    author: &str,
    registry: &Registry,
    engines: Option<&mut EngineSet>,
    target_id: &str,
    path: &Path,
    note: Option<&str>,
) -> Result<ExportReport> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    let tx = conn.transaction()?;
    let check = check_export(&tx, registry, engines, target_id)?;
    if !check.errors.is_empty() {
        return Err(Error::Invalid(format!(
            "검증 오류 {}건이 있어 내보낼 수 없음(검증 탭에서 확인)",
            check.errors.len()
        )));
    }
    if check.needs_note() && note.is_none() {
        let mut why = Vec::new();
        if !check.warnings.is_empty() {
            why.push(format!("검증 경고 {}건", check.warnings.len()));
        }
        if check.skipped_render > 0 {
            why.push(format!("하지 못한 렌더 검사 {}건", check.skipped_render));
        }
        return Err(Error::Invalid(format!("{}: 내보내려면 사유를 적어야 함", why.join(", "))));
    }
    let preview = plan_export(&tx, target_id)?;
    let adapter = adapter(&preview.target.adapter)?;
    let changeset_id = if preview.new_keys > 0 || note.is_some() {
        let mut message = format!("export {} ({} rows)", preview.target.name, preview.rows.len());
        if !check.warnings.is_empty() {
            message.push_str(&format!(", {} warnings", check.warnings.len()));
        }
        if check.skipped_render > 0 {
            message.push_str(&format!(", {} render checks skipped", check.skipped_render));
        }
        if let Some(n) = note {
            message.push_str(": ");
            message.push_str(n);
        }
        let changeset_id = insert_changeset(&tx, &ChangesetMeta::new(author, ChangesetKind::Admin).with_message(message))?;
        for row in preview.rows.iter().filter(|r| !r.fixed) {
            insert_key(&tx, changeset_id, target_id, &row.token_id, &row.external_key)?;
        }
        Some(changeset_id)
    } else {
        None
    };
    let ctx = AdapterContext::new(&tx, &preview.target);
    adapter.write(&ctx, &preview.rows, path)?;
    tx.commit()?;
    Ok(ExportReport {
        rows: preview.rows.len(),
        new_keys: preview.new_keys,
        changeset_id,
    })
}

/// 제품 형식 파일을 읽어 값 표로 만든다(역가져오기). 제품 키는 고정된 키로만 토큰을 찾고,
/// 못 찾은 키는 `unresolved`에 남긴다.
///
/// "원문으로 채우기"가 켜진 대상이면, DB에 번역이 없는 칸의 값이 원문과 같을 때 채워 넣은 값으로 보고 건너뛴다
/// (원문이 번역으로 들어오지 않게).
pub fn read_target_file(conn: &Connection, target_id: &str, path: &Path) -> Result<ValueTable> {
    let target = get_target(conn, target_id)?;
    let adapter = adapter(&target.adapter)?;
    let opts = adapter.options(&target.config)?;
    let rows = adapter.read(&AdapterContext::new(conn, &target), path)?;
    let by_key: HashMap<String, String> = fixed_keys(conn, target_id)?
        .into_iter()
        .map(|(token, key)| (key, token))
        .collect();
    let now = history::current(conn)?;
    let source = source_locale(conn)?;
    let mut table = ValueTable::new(opts.locales);
    for row in rows {
        let Some(token_id) = by_key.get(&row.external_key) else {
            table.unresolved.push(row.external_key);
            continue;
        };
        let cell = |locale: &str| now.get(&crate::model::CellKey::new(token_id, locale)).map(|c| c.value.as_str());
        let source_value = source.as_deref().and_then(cell);
        let values: Vec<(&str, &str)> = row
            .values
            .iter()
            .map(|(l, v)| {
                let filled = opts.fallback_to_source
                    && Some(l.as_str()) != source.as_deref()
                    && cell(l).is_none()
                    && Some(v.as_str()) == source_value;
                (l.as_str(), if filled { "" } else { v.as_str() })
            })
            .collect();
        table.push_row(&token_key(conn, token_id)?, values);
    }
    Ok(table)
}

// ── 어댑터 ────────────────────────────────────

/// 어댑터 공통 옵션(설정에서 뽑음).
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterOptions {
    pub locales: Vec<String>,
    pub key_rule: KeyRule,
    pub fallback_to_source: bool,
    pub include_deprecated: bool,
}

/// 제품 형식 파일의 한 행(역가져오기).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalRow {
    pub external_key: String,
    pub values: Vec<(String, String)>,
}

/// 파일을 쓰고 읽을 때 어댑터에 넘기는 정보.
pub struct AdapterContext<'a> {
    /// 설정은 `target.config`.
    pub target: &'a ExportTarget,
    /// 프로젝트(.tms) 파일이 있는 폴더. 외부 명령의 작업 폴더가 된다. 메모리 DB 면 None.
    pub project_dir: Option<PathBuf>,
}

impl<'a> AdapterContext<'a> {
    pub fn new(conn: &Connection, target: &'a ExportTarget) -> Self {
        Self {
            target,
            project_dir: crate::db::project_dir(conn),
        }
    }
}

pub trait FormatAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// 설정이 맞는지 확인한다. `locales`는 프로젝트에 있는 언어 코드.
    fn check_config(&self, config: &Value, locales: &HashSet<String>) -> Result<()>;
    fn options(&self, config: &Value) -> Result<AdapterOptions>;
    fn write(&self, ctx: &AdapterContext, rows: &[ExportRow], path: &Path) -> Result<()>;
    fn read(&self, ctx: &AdapterContext, path: &Path) -> Result<Vec<ExternalRow>>;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdapterInfo {
    pub id: &'static str,
    pub description: &'static str,
}

static TABLE: TableAdapter = TableAdapter;
static COMMAND: command::CommandAdapter = command::CommandAdapter;

/// 어댑터 목록. 새 Rust 어댑터는 여기에 추가한다.
pub fn adapters() -> Vec<&'static dyn FormatAdapter> {
    vec![&TABLE, &COMMAND]
}

pub fn adapter(id: &str) -> Result<&'static dyn FormatAdapter> {
    adapters()
        .into_iter()
        .find(|a| a.id() == id)
        .ok_or_else(|| Error::Invalid(format!("알 수 없는 포맷 어댑터: {id}")))
}

pub fn list_adapters() -> Vec<AdapterInfo> {
    adapters()
        .into_iter()
        .map(|a| AdapterInfo {
            id: a.id(),
            description: a.description(),
        })
        .collect()
}

/// 설정형 표: 시트 하나, 첫 행이 머리글(키 열 + 언어 열).
pub struct TableAdapter;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableConfig {
    #[serde(default = "default_sheet")]
    pub sheet_name: String,
    #[serde(default = "default_key_header")]
    pub key_header: String,
    #[serde(default)]
    pub columns: Vec<TableColumn>,
    #[serde(default)]
    pub key_rule: KeyRule,
    /// 번역이 없으면 원문 값으로 채운다.
    #[serde(default)]
    pub fallback_to_source: bool,
    /// deprecated 토큰도 내보낸다.
    #[serde(default)]
    pub include_deprecated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    pub header: String,
    pub locale: String,
}

fn default_sheet() -> String {
    "Strings".into()
}

fn default_key_header() -> String {
    "Key".into()
}

impl TableAdapter {
    fn config(config: &Value) -> Result<TableConfig> {
        serde_json::from_value(config.clone()).map_err(|e| Error::Invalid(format!("표 설정 오류: {e}")))
    }
}

impl FormatAdapter for TableAdapter {
    fn id(&self) -> &'static str {
        "table"
    }

    fn description(&self) -> &'static str {
        "설정형 표: 시트 하나, 첫 행 머리글(키 열 + 언어 열)"
    }

    fn check_config(&self, config: &Value, locales: &HashSet<String>) -> Result<()> {
        let c = Self::config(config)?;
        if c.columns.is_empty() {
            return Err(Error::Invalid("언어 열이 하나도 없음".into()));
        }
        rust_xlsxwriter::Worksheet::new()
            .set_name(&c.sheet_name)
            .map_err(|e| Error::Invalid(format!("시트 이름 오류: {e}")))?;
        let mut headers = HashSet::new();
        headers.insert(c.key_header.trim().to_string());
        if c.key_header.trim().is_empty() {
            return Err(Error::Invalid("키 열 머리글이 비어 있음".into()));
        }
        for col in &c.columns {
            if col.header.trim().is_empty() {
                return Err(Error::Invalid("머리글이 빈 열이 있음".into()));
            }
            if !headers.insert(col.header.trim().to_string()) {
                return Err(Error::Invalid(format!("머리글이 겹침: {}", col.header)));
            }
            if !locales.contains(&col.locale) {
                return Err(Error::Invalid(format!("프로젝트에 없는 언어: {}", col.locale)));
            }
        }
        Ok(())
    }

    fn options(&self, config: &Value) -> Result<AdapterOptions> {
        let c = Self::config(config)?;
        Ok(AdapterOptions {
            locales: c.columns.iter().map(|x| x.locale.clone()).collect(),
            key_rule: c.key_rule,
            fallback_to_source: c.fallback_to_source,
            include_deprecated: c.include_deprecated,
        })
    }

    fn write(&self, ctx: &AdapterContext, rows: &[ExportRow], path: &Path) -> Result<()> {
        use rust_xlsxwriter::{Format, Workbook};
        let c = Self::config(&ctx.target.config)?;
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet().set_name(&c.sheet_name)?;
        let bold = Format::new().set_bold();
        ws.write_string_with_format(0, 0, &c.key_header, &bold)?;
        ws.set_column_width(0, 32)?;
        for (i, col) in c.columns.iter().enumerate() {
            let x = (i + 1) as u16;
            ws.write_string_with_format(0, x, &col.header, &bold)?;
            ws.set_column_width(x, 40)?;
        }
        for (r, row) in rows.iter().enumerate() {
            let y = (r + 1) as u32;
            ws.write_string(y, 0, &row.external_key)?;
            for (i, col) in c.columns.iter().enumerate() {
                if let Some(v) = row.values.get(&col.locale) {
                    ws.write_string(y, (i + 1) as u16, v)?;
                }
            }
        }
        ws.set_freeze_panes(1, 1)?;
        wb.save(path)?;
        Ok(())
    }

    fn read(&self, ctx: &AdapterContext, path: &Path) -> Result<Vec<ExternalRow>> {
        let c = Self::config(&ctx.target.config)?;
        let rows = sheet::read_sheet(path, &c.sheet_name)?;
        // 머리글 행: 키 열 머리글이 처음 나오는 행
        let (header_row, key_col) = rows
            .iter()
            .enumerate()
            .find_map(|(r, row)| row.iter().position(|h| h.trim() == c.key_header.trim()).map(|k| (r, k)))
            .ok_or_else(|| Error::Invalid(format!("키 열 머리글 '{}'을 찾지 못함", c.key_header)))?;
        let header = &rows[header_row];
        let cols: Vec<(usize, &str)> = c
            .columns
            .iter()
            .filter_map(|col| {
                header
                    .iter()
                    .position(|h| h.trim() == col.header.trim())
                    .map(|i| (i, col.locale.as_str()))
            })
            .collect();
        let cell = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
        Ok(rows
            .iter()
            .skip(header_row + 1)
            .filter_map(|row| {
                let key = cell(row, key_col).trim().to_string();
                (!key.is_empty()).then(|| ExternalRow {
                    external_key: key,
                    values: cols.iter().map(|(i, l)| (l.to_string(), cell(row, *i))).collect(),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::apply;
    use crate::import::{plan_import, ImportOptions};
    use crate::model::{CellChange, CellKey, CellValue};
    use crate::{db, tokens};

    fn setup() -> (Connection, String) {
        let mut conn = db::open_in_memory().unwrap();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let a = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let b = tokens::create_token(&mut conn, "t", "common.ok", None).unwrap();
        let set = |t: &str, l: &str, v: &str| CellChange {
            key: CellKey::new(t, l),
            new: Some(CellValue::translated(v)),
        };
        apply(
            &mut conn,
            &ChangesetMeta::new("t", ChangesetKind::Edit),
            &[set(&a, "ko-KR", "제목"), set(&a, "en-US", "Title"), set(&b, "ko-KR", "확인")],
        )
        .unwrap();
        let target = save_target(
            &mut conn,
            "t",
            &ExportTargetInput {
                id: None,
                name: "제품 A".into(),
                adapter: "table".into(),
                config: json!({
                    "sheet_name": "Strings",
                    "key_header": "ID",
                    "columns": [
                        { "header": "Korean", "locale": "ko-KR" },
                        { "header": "English", "locale": "en-US" }
                    ],
                    "key_rule": { "prefix": "STR_", "case": "upper", "dot_replacement": "_" },
                    "fallback_to_source": true
                }),
            },
        )
        .unwrap();
        (conn, target)
    }

    #[test]
    fn suggests_keys_and_fixes_them_on_export() {
        let (mut conn, target) = setup();
        let p = plan_export(&conn, &target).unwrap();
        let keys: Vec<_> = p.rows.iter().map(|r| (r.external_key.as_str(), r.fixed)).collect();
        assert_eq!(keys, vec![("STR_COMMON_OK", false), ("STR_HOME_TITLE", false)]);
        // 원문으로 채우기: common.ok 의 en-US 는 원문 값
        assert_eq!(p.rows[0].values.get("en-US").map(String::as_str), Some("확인"));

        let path = std::env::temp_dir().join(format!("tms-export-{}.xlsx", new_id()));
        let report = export_to_file(&mut conn, "t", &Registry::builtin(), None, &target, &path, None).unwrap();
        assert_eq!((report.rows, report.new_keys), (2, 2));
        assert!(report.changeset_id.is_some());

        // 이후 토큰명을 바꿔도 제품 키는 그대로
        let a = tokens::find_token_by_key(&conn, "home.title").unwrap().unwrap();
        tokens::rename_token(&mut conn, "t", &a, "home.header").unwrap();
        let p = plan_export(&conn, &target).unwrap();
        let row = p.rows.iter().find(|r| r.token_key == "home.header").unwrap();
        assert_eq!((row.external_key.as_str(), row.fixed), ("STR_HOME_TITLE", true));
        assert_eq!(p.new_keys, 0);

        // 새 토큰이 같은 제안 키를 만들면 번호를 붙인다
        tokens::create_token(&mut conn, "t", "home_title", None).unwrap();
        let p = plan_export(&conn, &target).unwrap();
        let row = p.rows.iter().find(|r| r.token_key == "home_title").unwrap();
        assert_eq!(row.external_key, "STR_HOME_TITLE_2");

        // 고정된 키가 있으면 대상 삭제 거부
        assert!(delete_target(&mut conn, "t", &target).is_err());

        // 역가져오기: 내보낸 파일을 고쳐 다시 읽으면 제품 키로 토큰을 찾는다
        let t = get_target(&conn, &target).unwrap();
        let rows = TABLE.read(&AdapterContext::new(&conn, &t), &path).unwrap();
        assert_eq!(rows.len(), 2);
        let table = read_target_file(&conn, &target, &path).unwrap();
        assert_eq!(table.keys, vec!["common.ok", "home.header"]);
        assert!(table.unresolved.is_empty());
        // 원문으로 채웠던 common.ok 의 en-US("확인")는 번역으로 들어오지 않는다
        let plan = plan_import(&conn, &table, &ImportOptions::default()).unwrap();
        assert!(plan.changes.is_empty(), "{:?}", plan.changes);
        assert_eq!(plan.unchanged, 3);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn validation_gate_blocks_errors_and_needs_reason_for_warnings() {
        use crate::validate::{save_constraint, ConstraintInput, Scope};
        let (mut conn, target) = setup();
        let reg = Registry::builtin();
        let path = std::env::temp_dir().join(format!("tms-export-{}.xlsx", new_id()));
        // common.ok 의 en-US 가 비어 있다
        let mut required = ConstraintInput {
            id: None,
            validator: "text.required".into(),
            params: json!({}),
            scope: Scope::Global,
            locale: None,
            severity: Severity::Error,
            enabled: true,
            description: None,
        };
        let id = save_constraint(&mut conn, "t", &reg, &required).unwrap();
        let check = check_export(&conn, &reg, None, &target).unwrap();
        assert_eq!(check.errors.len(), 1);
        assert_eq!((check.errors[0].token_key.as_str(), check.errors[0].locale.as_str()), ("common.ok", "en-US"));
        assert!(export_to_file(&mut conn, "t", &reg, None, &target, &path, Some("사유")).is_err());
        assert!(!path.exists());

        required.id = Some(id);
        required.severity = Severity::Warning;
        save_constraint(&mut conn, "t", &reg, &required).unwrap();
        assert!(export_to_file(&mut conn, "t", &reg, None, &target, &path, None).is_err());
        let report = export_to_file(&mut conn, "t", &reg, None, &target, &path, Some("영어는 다음 배포에")).unwrap();
        let message: String = conn
            .query_row("SELECT message FROM changeset WHERE id = ?1", [report.changeset_id.unwrap()], |r| r.get(0))
            .unwrap();
        assert!(message.contains("1 warnings") && message.contains("영어는 다음 배포에"), "{message}");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn skipped_render_checks_need_a_reason() {
        use crate::render::tests::{fake_engines, profile_input};
        use crate::render::{add_font_bytes, register_engine, save_profile};
        use crate::validate::{save_constraint, ConstraintInput, Scope};
        let (mut conn, target) = setup();
        let reg = Registry::builtin();
        let (mut engines, _) = fake_engines();
        let dir = std::env::temp_dir().join("lingua-tms-test").join(format!("gate-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dll = dir.join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();
        register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();
        let font = add_font_bytes(&mut conn, "t", "a.ttf", b"a").unwrap();
        let profile = save_profile(&mut conn, "t", &profile_input("v1", &font.id)).unwrap();
        let rule = ConstraintInput {
            id: None,
            validator: "render.max_width".into(),
            params: json!({ "profile": profile, "max_width_px": 1000 }),
            scope: Scope::Global,
            locale: None,
            severity: Severity::Error,
            enabled: true,
            description: None,
        };
        save_constraint(&mut conn, "t", &reg, &rule).unwrap();

        // 엔진으로 재면 통과
        assert!(!check_export(&conn, &reg, Some(&mut engines), &target).unwrap().needs_note());
        // 엔진을 쓸 수 없으면 값이 있는 내보낼 셀 3개의 렌더 검사를 건너뛰고, 사유가 있어야 내보낸다
        let check = check_export(&conn, &reg, None, &target).unwrap();
        assert_eq!(check.skipped_render, 3);
        assert!(check.needs_note() && check.errors.is_empty());
        let path = dir.join("out.xlsx");
        assert!(export_to_file(&mut conn, "t", &reg, None, &target, &path, None).is_err());
        let report = export_to_file(&mut conn, "t", &reg, None, &target, &path, Some("엔진 DLL 없음")).unwrap();
        let message: String = conn
            .query_row("SELECT message FROM changeset WHERE id = ?1", [report.changeset_id.unwrap()], |r| r.get(0))
            .unwrap();
        assert!(message.contains("3 render checks skipped"), "{message}");
    }

    #[test]
    fn manual_key_and_config_checks() {
        let (mut conn, target) = setup();
        let a = tokens::find_token_by_key(&conn, "home.title").unwrap().unwrap();
        fix_external_key(&mut conn, "t", &target, &a, "LEGACY_TITLE").unwrap();
        assert!(fix_external_key(&mut conn, "t", &target, &a, "OTHER").is_err());
        let b = tokens::find_token_by_key(&conn, "common.ok").unwrap().unwrap();
        assert!(fix_external_key(&mut conn, "t", &target, &b, "LEGACY_TITLE").is_err());
        let p = plan_export(&conn, &target).unwrap();
        assert!(p.rows.iter().any(|r| r.external_key == "LEGACY_TITLE" && r.fixed));

        let bad = ExportTargetInput {
            id: None,
            name: "x".into(),
            adapter: "table".into(),
            config: json!({ "columns": [{ "header": "French", "locale": "fr-FR" }] }),
        };
        assert!(save_target(&mut conn, "t", &bad).is_err());
    }
}
