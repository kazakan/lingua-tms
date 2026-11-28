//! 렌더 측정 설정(엔진 DLL·폰트·렌더 프로필)과 측정 엔진 연결.
//!
//! - 측정 엔진(`measure_engine`): 엔진 DLL 경로. id 는 DLL 이 알려 주는 엔진 버전(`tms_engine_version`)이다.
//!   .tms 폴더 안의 DLL 은 그 폴더 기준 상대 경로로 저장해, 폴더째 옮겨도 찾게 한다.
//! - 폰트(`font_asset`): .ttf 바이트를 프로젝트 파일에 담는다(sha256 으로 중복 방지).
//! - 렌더 프로필(`render_profile`): 엔진 + 스타일 + 언어별 폰트 폴백 순서.
//!   렌더 제약은 파라미터 `profile`(프로필 id)로 프로필을 고른다.
//! - [`EngineSet`]: 프로젝트를 연 동안 불러 둔 엔진과 폰트. 폭·줄 수는 모두 엔진이 잰다(TMS 가 따로 계산하지 않음).
//!
//! 엔진 DLL 은 없을 수 있다. 이 PC 에서 DLL 을 못 쓰면 렌더 검사는 이유와 함께 건너뛰고([`SetupError::Unavailable`]),
//! 편집과 텍스트 검증은 그대로 동작한다. 프로필·폰트 설정이 잘못된 경우([`SetupError::Config`])는 검증기 오류로 보고한다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tms_measure::{EngineDll, FontId, Layout, MeasureError, Measurer, Style, WrapMode};

use crate::history::{insert_changeset, record_entity};
use crate::ids::new_id;
use crate::model::{CellKey, ChangesetKind, ChangesetMeta};
use crate::tokens::{list_locales, source_locale, token_key};
use crate::validate::{effective_constraints, load_constraints, Finding, Registry, RenderCtx, RenderInfo, TextCtx};
use crate::{db, history, Error, Result};

/// 렌더 프로필의 기본 폰트 목록 키. 언어별 목록이 없는 언어는 이 목록을 쓴다.
pub const DEFAULT_FONTS: &str = "*";
/// 렌더 제약 파라미터에서 프로필 id 를 담는 키.
pub const PROFILE_PARAM: &str = "profile";

/// 렌더 제약 파라미터의 프로필 id.
pub fn profile_id_of(params: &Value) -> Option<&str> {
    params.get(PROFILE_PARAM).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn admin(author: &str, message: String) -> ChangesetMeta {
    ChangesetMeta::new(author, ChangesetKind::Admin).with_message(message)
}

// ── 측정 엔진 ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasureEngine {
    /// 엔진 버전(DLL 의 `tms_engine_version`).
    pub id: String,
    /// 저장된 경로. .tms 폴더 안이면 그 폴더 기준 상대 경로다.
    pub dll_path: String,
    pub note: Option<String>,
}

/// 화면용 엔진 정보.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineInfo {
    pub id: String,
    pub dll_path: String,
    pub note: Option<String>,
    /// 실제로 여는 경로(상대 경로를 .tms 폴더 기준으로 푼 것).
    pub resolved_path: String,
    /// 그 경로에 파일이 있는지.
    pub exists: bool,
    /// 지금 불러 둔 상태인지.
    pub loaded: bool,
}

pub fn list_engines(conn: &Connection) -> Result<Vec<MeasureEngine>> {
    let mut stmt = conn.prepare("SELECT id, dll_path, note FROM measure_engine ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(MeasureEngine {
            id: r.get(0)?,
            dll_path: r.get(1)?,
            note: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn find_engine(conn: &Connection, id: &str) -> Result<Option<MeasureEngine>> {
    Ok(list_engines(conn)?.into_iter().find(|e| e.id == id))
}

pub fn engine_infos(conn: &Connection, engines: &EngineSet) -> Result<Vec<EngineInfo>> {
    Ok(list_engines(conn)?
        .into_iter()
        .map(|e| {
            let resolved = resolve_dll_path(conn, &e.dll_path);
            EngineInfo {
                exists: resolved.is_file(),
                resolved_path: resolved.display().to_string(),
                loaded: engines.loaded.contains_key(&e.id),
                id: e.id,
                dll_path: e.dll_path,
                note: e.note,
            }
        })
        .collect())
}

/// 저장된 DLL 경로를 실제 경로로. 상대 경로는 .tms 폴더 기준이다.
pub fn resolve_dll_path(conn: &Connection, stored: &str) -> PathBuf {
    let p = Path::new(stored);
    match db::project_dir(conn) {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p.to_path_buf(),
    }
}

/// 저장할 경로: .tms 폴더 안이면 그 폴더 기준 상대 경로, 아니면 그대로.
fn stored_dll_path(conn: &Connection, path: &Path) -> String {
    db::project_dir(conn)
        .and_then(|dir| path.strip_prefix(&dir).ok().map(Path::to_path_buf))
        .filter(|rel| !rel.as_os_str().is_empty())
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// 엔진 DLL 등록. DLL 을 불러 엔진 버전을 읽고, 그 버전을 id 로 저장한다.
///
/// 같은 버전이 이미 있으면 경로만 바꾼다(DLL 을 옮겼을 때). 불러 둔 엔진은 이번에 연 것으로 바뀐다.
pub fn register_engine(
    conn: &mut Connection,
    author: &str,
    engines: &mut EngineSet,
    path: &Path,
    note: Option<&str>,
) -> Result<MeasureEngine> {
    let path = if path.is_relative() {
        resolve_dll_path(conn, &path.to_string_lossy())
    } else {
        path.to_path_buf()
    };
    if !path.is_file() {
        return Err(Error::NotFound(format!("엔진 DLL {}", path.display())));
    }
    let measurer = (engines.loader)(&path).map_err(|e| Error::Invalid(format!("엔진 DLL을 불러오지 못함: {e}")))?;
    let id = measurer.engine_version().trim().to_string();
    if id.is_empty() {
        return Err(Error::Invalid("엔진이 버전 문자열을 돌려주지 않음(tms_engine_version)".into()));
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);

    let tx = conn.transaction()?;
    let old = find_engine(&tx, &id)?;
    let new = MeasureEngine {
        id: id.clone(),
        dll_path: stored_dll_path(&tx, &path),
        note: note.or_else(|| old.as_ref().and_then(|o| o.note.clone())),
    };
    if old.as_ref() != Some(&new) {
        let changeset_id = insert_changeset(&tx, &admin(author, format!("register measure engine {id}")))?;
        tx.execute(
            "INSERT INTO measure_engine (id, dll_path, note) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET dll_path = excluded.dll_path, note = excluded.note",
            params![new.id, new.dll_path, new.note],
        )?;
        record_entity(&tx, changeset_id, "measure_engine", &id, old.as_ref(), Some(&new))?;
    }
    tx.commit()?;
    engines.loaded.insert(id, LoadedEngine::new(measurer));
    Ok(new)
}

/// 엔진 등록 삭제. 이 엔진을 쓰는 렌더 프로필이 있으면 거부한다.
pub fn delete_engine(conn: &mut Connection, author: &str, engines: &mut EngineSet, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let old = find_engine(&tx, id)?.ok_or_else(|| Error::NotFound(format!("측정 엔진 {id}")))?;
    let users: Vec<String> = list_profiles(&tx)?
        .into_iter()
        .filter(|p| p.engine_id == id)
        .map(|p| p.name)
        .collect();
    if !users.is_empty() {
        return Err(Error::Invalid(format!("렌더 프로필이 이 엔진을 씀: {}", users.join(", "))));
    }
    let changeset_id = insert_changeset(&tx, &admin(author, format!("delete measure engine {id}")))?;
    tx.execute("DELETE FROM measure_engine WHERE id = ?1", [id])?;
    record_entity(&tx, changeset_id, "measure_engine", id, Some(&old), None)?;
    tx.commit()?;
    engines.loaded.remove(id);
    Ok(())
}

// ── 폰트 ─────────────────────────────────────

/// 폰트 정보(바이트는 빼고). 이력(entity_rev)에도 이것만 남는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontAsset {
    pub id: String,
    pub family: String,
    pub file_name: String,
    pub sha256: String,
    /// 바이트 수.
    pub size: i64,
}

pub fn list_fonts(conn: &Connection) -> Result<Vec<FontAsset>> {
    let mut stmt =
        conn.prepare("SELECT id, family, file_name, sha256, length(data) FROM font_asset ORDER BY family, file_name")?;
    let rows = stmt.query_map([], |r| {
        Ok(FontAsset {
            id: r.get(0)?,
            family: r.get(1)?,
            file_name: r.get(2)?,
            sha256: r.get(3)?,
            size: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 폰트 파일 추가. [`add_font_bytes`] 참고.
pub fn add_font(conn: &mut Connection, author: &str, path: &Path) -> Result<FontAsset> {
    let data = std::fs::read(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    add_font_bytes(conn, author, &file_name, &data)
}

/// 폰트 바이트 추가. 같은 내용(sha256)이 이미 있으면 새로 넣지 않고 그것을 돌려준다.
/// 이름(family)은 파일 이름에서 확장자를 뺀 것이다. 폰트 내용이 맞는지는 엔진이 처음 쓸 때 확인한다.
pub fn add_font_bytes(conn: &mut Connection, author: &str, file_name: &str, data: &[u8]) -> Result<FontAsset> {
    if data.is_empty() {
        return Err(Error::Invalid(format!("빈 폰트 파일: {file_name}")));
    }
    let sha256: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
    if let Some(f) = list_fonts(conn)?.into_iter().find(|f| f.sha256 == sha256) {
        return Ok(f);
    }
    let family = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "font".into());
    let font = FontAsset {
        id: new_id(),
        family,
        file_name: file_name.to_string(),
        sha256,
        size: data.len() as i64,
    };
    let tx = conn.transaction()?;
    let changeset_id = insert_changeset(&tx, &admin(author, format!("add font {}", font.file_name)))?;
    tx.execute(
        "INSERT INTO font_asset (id, family, file_name, sha256, data) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![font.id, font.family, font.file_name, font.sha256, data],
    )?;
    record_entity(&tx, changeset_id, "font_asset", &font.id, None, Some(&font))?;
    tx.commit()?;
    Ok(font)
}

/// 폰트 삭제. 렌더 프로필이 쓰는 폰트는 거부한다.
pub fn delete_font(conn: &mut Connection, author: &str, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let old = list_fonts(&tx)?
        .into_iter()
        .find(|f| f.id == id)
        .ok_or_else(|| Error::NotFound(format!("폰트 {id}")))?;
    let users: Vec<String> = list_profiles(&tx)?
        .into_iter()
        .filter(|p| p.fonts.values().flatten().any(|f| f == id))
        .map(|p| p.name)
        .collect();
    if !users.is_empty() {
        return Err(Error::Invalid(format!("렌더 프로필이 이 폰트를 씀: {}", users.join(", "))));
    }
    let changeset_id = insert_changeset(&tx, &admin(author, format!("delete font {}", old.file_name)))?;
    tx.execute("DELETE FROM font_asset WHERE id = ?1", [id])?;
    record_entity(&tx, changeset_id, "font_asset", id, Some(&old), None)?;
    tx.commit()?;
    Ok(())
}

/// (이름, 바이트)
fn font_data(conn: &Connection, id: &str) -> Result<(String, Vec<u8>)> {
    conn.query_row("SELECT family, data FROM font_asset WHERE id = ?1", [id], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("폰트 {id}")))
}

// ── 렌더 프로필 ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderProfile {
    pub id: String,
    pub name: String,
    /// 측정 엔진 id(엔진 버전).
    pub engine_id: String,
    pub size_px: f64,
    pub letter_spacing: f64,
    /// 0 = 엔진 기본값.
    pub line_height: f64,
    pub wrap: WrapMode,
    /// 언어별 폰트 폴백 순서(폰트 id). `"*"`는 기본 목록이고, 언어 목록이 있으면 기본 대신 그것만 쓴다.
    pub fonts: BTreeMap<String, Vec<String>>,
}

impl RenderProfile {
    pub fn style(&self) -> Style {
        Style {
            size_px: self.size_px as f32,
            letter_spacing: self.letter_spacing as f32,
            line_height: self.line_height as f32,
            wrap: self.wrap,
        }
    }

    /// 이 언어에 쓸 폰트 폴백 순서.
    pub fn fonts_for(&self, locale: &str) -> &[String] {
        self.fonts
            .get(locale)
            .filter(|l| !l.is_empty())
            .or_else(|| self.fonts.get(DEFAULT_FONTS))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// 프로필 저장 입력. id 가 없으면 새로 만든다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderProfileInput {
    pub id: Option<String>,
    pub name: String,
    pub engine_id: String,
    pub size_px: f64,
    #[serde(default)]
    pub letter_spacing: f64,
    #[serde(default)]
    pub line_height: f64,
    pub wrap: WrapMode,
    pub fonts: BTreeMap<String, Vec<String>>,
}

pub fn list_profiles(conn: &Connection) -> Result<Vec<RenderProfile>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, engine_id, size_px, letter_spacing, line_height, wrap_mode, fonts
         FROM render_profile ORDER BY name",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(RenderProfile {
            id: r.get(0)?,
            name: r.get(1)?,
            engine_id: r.get(2)?,
            size_px: r.get(3)?,
            letter_spacing: r.get(4)?,
            line_height: r.get(5)?,
            wrap: if r.get::<_, i64>(6)? == 0 { WrapMode::None } else { WrapMode::Engine },
            fonts: serde_json::from_str(&r.get::<_, String>(7)?)?,
        });
    }
    Ok(out)
}

pub fn get_profile(conn: &Connection, id: &str) -> Result<RenderProfile> {
    list_profiles(conn)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::NotFound(format!("렌더 프로필 {id}")))
}

/// 입력을 확인하고 정리한다(저장하지 않음). 언어별 목록이 비어 있으면 기본 목록을 쓰는 것으로 보고 뺀다.
pub fn check_profile(conn: &Connection, input: &RenderProfileInput) -> Result<RenderProfile> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("프로필 이름이 비어 있음".into()));
    }
    if find_engine(conn, &input.engine_id)?.is_none() {
        return Err(Error::Invalid(format!("등록되지 않은 측정 엔진: {}", input.engine_id)));
    }
    if !(input.size_px.is_finite() && input.size_px > 0.0) {
        return Err(Error::Invalid("글자 크기는 0보다 커야 함".into()));
    }
    if !input.letter_spacing.is_finite() {
        return Err(Error::Invalid("자간이 숫자가 아님".into()));
    }
    if !(input.line_height.is_finite() && input.line_height >= 0.0) {
        return Err(Error::Invalid("줄 높이는 0 이상이어야 함(0 = 엔진 기본값)".into()));
    }
    let locales: HashSet<String> = list_locales(conn)?.into_iter().map(|l| l.code).collect();
    let known: HashMap<String, String> = list_fonts(conn)?.into_iter().map(|f| (f.id, f.family)).collect();
    let mut fonts = BTreeMap::new();
    for (key, list) in &input.fonts {
        let label = if key == DEFAULT_FONTS { "기본" } else { key.as_str() };
        if key != DEFAULT_FONTS && !locales.contains(key) {
            return Err(Error::Invalid(format!("프로젝트에 없는 언어: {key}")));
        }
        if list.is_empty() {
            continue;
        }
        let mut seen = HashSet::new();
        for f in list {
            let Some(family) = known.get(f) else {
                return Err(Error::Invalid(format!("없는 폰트: {f}")));
            };
            if !seen.insert(f) {
                return Err(Error::Invalid(format!("{label} 폰트 목록에 '{family}'가 두 번 있음")));
            }
        }
        fonts.insert(key.clone(), list.clone());
    }
    if !fonts.contains_key(DEFAULT_FONTS) {
        return Err(Error::Invalid("기본 폰트(모든 언어)를 하나 이상 골라야 함".into()));
    }
    Ok(RenderProfile {
        id: input.id.clone().unwrap_or_default(),
        name: name.to_string(),
        engine_id: input.engine_id.clone(),
        size_px: input.size_px,
        letter_spacing: input.letter_spacing,
        line_height: input.line_height,
        wrap: input.wrap,
        fonts,
    })
}

/// 프로필 저장. 바뀐 게 없으면 변경 묶음을 만들지 않는다. 기록은 entity_rev 에 남는다.
pub fn save_profile(conn: &mut Connection, author: &str, input: &RenderProfileInput) -> Result<String> {
    let mut new = check_profile(conn, input)?;
    let tx = conn.transaction()?;
    let old = match &input.id {
        Some(id) => Some(get_profile(&tx, id)?),
        None => None,
    };
    if list_profiles(&tx)?
        .iter()
        .any(|p| p.name == new.name && Some(&p.id) != input.id.as_ref())
    {
        return Err(Error::Invalid(format!("같은 이름의 렌더 프로필이 있음: {}", new.name)));
    }
    new.id = input.id.clone().unwrap_or_else(new_id);
    if old.as_ref() == Some(&new) {
        return Ok(new.id);
    }
    let changeset_id = insert_changeset(&tx, &admin(author, format!("save render profile {}", new.name)))?;
    let row = params![
        new.id,
        new.name,
        new.engine_id,
        new.size_px,
        new.letter_spacing,
        new.line_height,
        i64::from(new.wrap == WrapMode::Engine),
        serde_json::to_string(&new.fonts)?,
    ];
    if old.is_some() {
        tx.execute(
            "UPDATE render_profile SET name = ?2, engine_id = ?3, size_px = ?4, letter_spacing = ?5,
                    line_height = ?6, wrap_mode = ?7, fonts = ?8
             WHERE id = ?1",
            row,
        )?;
    } else {
        tx.execute(
            "INSERT INTO render_profile (id, name, engine_id, size_px, letter_spacing, line_height, wrap_mode, fonts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            row,
        )?;
    }
    record_entity(&tx, changeset_id, "render_profile", &new.id, old.as_ref(), Some(&new))?;
    tx.commit()?;
    Ok(new.id)
}

/// 프로필 삭제. 이 프로필을 쓰는 검증 규칙이 있으면 거부한다.
pub fn delete_profile(conn: &mut Connection, author: &str, id: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let old = get_profile(&tx, id)?;
    let users = load_constraints(&tx)?
        .iter()
        .filter(|c| profile_id_of(&c.params) == Some(id))
        .count();
    if users > 0 {
        return Err(Error::Invalid(format!("검증 규칙 {users}개가 이 프로필을 씀")));
    }
    let changeset_id = insert_changeset(&tx, &admin(author, format!("delete render profile {}", old.name)))?;
    tx.execute("DELETE FROM render_profile WHERE id = ?1", [id])?;
    record_entity(&tx, changeset_id, "render_profile", id, Some(&old), None)?;
    tx.commit()?;
    Ok(())
}

// ── 엔진 불러 두기 ─────────────────────────────

/// DLL 경로로 측정 엔진을 연다. 테스트에서는 가짜 측정기를 돌려주는 함수를 넣는다.
pub type EngineLoader =
    Box<dyn Fn(&Path) -> std::result::Result<Box<dyn Measurer + Send>, MeasureError> + Send>;

/// 프로젝트를 연 동안 불러 둔 측정 엔진과 폰트. 엔진은 처음 쓸 때 연다.
///
/// 측정 호출은 한 번에 하나씩만 한다(앱에서는 프로젝트 연결과 같은 Mutex 안에 둔다).
pub struct EngineSet {
    loader: EngineLoader,
    loaded: HashMap<String, LoadedEngine>,
}

struct LoadedEngine {
    measurer: Box<dyn Measurer + Send>,
    /// 폰트 id → 엔진에 불러 둔 폰트. 폰트 바이트는 id 별로 바뀌지 않는다.
    fonts: HashMap<String, FontId>,
}

/// 렌더 측정을 준비하지 못한 이유.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupError {
    /// 이 PC 에서 엔진 DLL 을 쓸 수 없음(파일 없음, 불러오기 실패, 버전 다름). 렌더 검사를 건너뛴다.
    Unavailable(String),
    /// 설정 오류(프로필 미지정, 없는 프로필, 엔진이 폰트를 못 읽음 ...). 검증기 오류로 보고한다.
    Config(String),
}

impl SetupError {
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(m) | Self::Config(m) => m,
        }
    }
}

impl EngineSet {
    /// 엔진 DLL(`engine/include/tms_measure.h`)을 쓰는 기본 구성.
    pub fn dll() -> Self {
        Self::with_loader(Box::new(|path| {
            Ok(Box::new(EngineDll::load(path)?) as Box<dyn Measurer + Send>)
        }))
    }

    pub fn with_loader(loader: EngineLoader) -> Self {
        Self {
            loader,
            loaded: HashMap::new(),
        }
    }

    /// 엔진을 돌려준다. 아직 안 열었으면 등록된 경로에서 연다.
    fn engine(&mut self, conn: &Connection, id: &str) -> std::result::Result<&mut LoadedEngine, SetupError> {
        if !self.loaded.contains_key(id) {
            let row = find_engine(conn, id)
                .map_err(|e| SetupError::Config(e.to_string()))?
                .ok_or_else(|| SetupError::Config(format!("등록되지 않은 측정 엔진: {id}")))?;
            let path = resolve_dll_path(conn, &row.dll_path);
            if !path.is_file() {
                return Err(SetupError::Unavailable(format!(
                    "엔진 {id}의 DLL이 없음: {}",
                    path.display()
                )));
            }
            let measurer = (self.loader)(&path)
                .map_err(|e| SetupError::Unavailable(format!("엔진 {id}의 DLL을 불러오지 못함: {e}")))?;
            let found = measurer.engine_version();
            if found.trim() != id {
                return Err(SetupError::Unavailable(format!(
                    "{}의 엔진 버전이 {found}임(등록된 버전 {id}). 렌더 탭에서 DLL을 다시 등록하세요",
                    path.display()
                )));
            }
            self.loaded.insert(id.to_string(), LoadedEngine::new(measurer));
        }
        Ok(self.loaded.get_mut(id).expect("바로 위에서 넣음"))
    }
}

impl LoadedEngine {
    fn new(measurer: Box<dyn Measurer + Send>) -> Self {
        Self {
            measurer,
            fonts: HashMap::new(),
        }
    }

    /// 프로필의 이 언어 폰트 폴백 순서를 엔진 폰트로. 처음 쓰는 폰트는 DB 에서 읽어 엔진에 불러 둔다.
    fn fonts(
        &mut self,
        conn: &Connection,
        profile: &RenderProfile,
        locale: &str,
    ) -> std::result::Result<Vec<FontId>, SetupError> {
        let ids = profile.fonts_for(locale);
        if ids.is_empty() {
            return Err(SetupError::Config(format!(
                "렌더 프로필 '{}'에 {locale}에 쓸 폰트가 없음",
                profile.name
            )));
        }
        ids.iter()
            .map(|id| {
                if let Some(f) = self.fonts.get(id) {
                    return Ok(*f);
                }
                let (family, data) = font_data(conn, id).map_err(|e| SetupError::Config(e.to_string()))?;
                let f = self
                    .measurer
                    .load_font(&data)
                    .map_err(|e| SetupError::Config(format!("엔진이 폰트 '{family}'를 읽지 못함: {e}")))?;
                self.fonts.insert(id.clone(), f);
                Ok(f)
            })
            .collect()
    }
}

/// 측정 준비가 된 상태: 검증기에 넘길 [`RenderCtx`]와 결과에 남길 정보.
pub struct Prepared<'s> {
    pub ctx: RenderCtx<'s>,
    pub info: RenderInfo,
}

/// 검증 한 번(전체 검증, 셀 하나 측정) 동안 쓰는 측정 준비.
/// 프로필은 처음에 한 번 읽고, 준비 실패는 (프로필, 언어)별로 기억해 같은 오류로 다시 시도하지 않는다.
pub struct RenderSession<'a> {
    conn: &'a Connection,
    engines: Option<&'a mut EngineSet>,
    profiles: HashMap<String, RenderProfile>,
    failed: HashMap<(String, String), SetupError>,
}

impl<'a> RenderSession<'a> {
    /// `engines`가 None 이면 렌더 검사는 모두 건너뛴다.
    pub fn new(conn: &'a Connection, engines: Option<&'a mut EngineSet>) -> Result<Self> {
        Ok(Self {
            conn,
            engines,
            profiles: list_profiles(conn)?.into_iter().map(|p| (p.id.clone(), p)).collect(),
            failed: HashMap::new(),
        })
    }

    /// 렌더 제약의 파라미터(`profile`)로 측정을 준비한다.
    pub fn for_params(&mut self, params: &Value, locale: &str) -> std::result::Result<Prepared<'_>, SetupError> {
        let id = profile_id_of(params).ok_or_else(|| {
            SetupError::Config("렌더 프로필이 지정되지 않음(검증 탭에서 규칙을 열어 프로필을 고르세요)".into())
        })?;
        let profile = self
            .profiles
            .get(id)
            .ok_or_else(|| SetupError::Config(format!("없는 렌더 프로필: {id}")))?;
        prepare(self.conn, self.engines.as_deref_mut(), &mut self.failed, profile, locale)
    }

    /// 주어진 프로필(저장하지 않은 것도 됨)로 측정을 준비한다.
    pub fn for_profile(
        &mut self,
        profile: &RenderProfile,
        locale: &str,
    ) -> std::result::Result<Prepared<'_>, SetupError> {
        prepare(self.conn, self.engines.as_deref_mut(), &mut self.failed, profile, locale)
    }
}

fn prepare<'s>(
    conn: &Connection,
    engines: Option<&'s mut EngineSet>,
    failed: &mut HashMap<(String, String), SetupError>,
    profile: &RenderProfile,
    locale: &str,
) -> std::result::Result<Prepared<'s>, SetupError> {
    let engines = engines.ok_or_else(|| SetupError::Unavailable("측정 엔진 없이 검증함".into()))?;
    let key = (profile.id.clone(), locale.to_string());
    if let Some(e) = failed.get(&key) {
        return Err(e.clone());
    }
    let engine = match engines.engine(conn, &profile.engine_id) {
        Ok(e) => e,
        Err(e) => {
            failed.insert(key, e.clone());
            return Err(e);
        }
    };
    let fonts = match engine.fonts(conn, profile, locale) {
        Ok(f) => f,
        Err(e) => {
            failed.insert(key, e.clone());
            return Err(e);
        }
    };
    let info = RenderInfo {
        profile: profile.name.clone(),
        engine: engine.measurer.engine_version(),
    };
    Ok(Prepared {
        ctx: RenderCtx {
            measurer: engine.measurer.as_mut(),
            fonts,
            style: profile.style(),
        },
        info,
    })
}

// ── 측정 결과 보기 ─────────────────────────────

/// 화면에 보여 줄 측정 결과.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutView {
    /// 줄바꿈 박스 폭. 0 이면 줄바꿈 없이 한 줄로 잰 것.
    pub box_width: f32,
    /// 비교할 한도(박스 폭 또는 최대 폭).
    pub limit: Option<f32>,
    pub width: f32,
    pub height: f32,
    pub missing_glyphs: u32,
    pub word_overflow: bool,
    pub lines: Vec<LineView>,
    pub engine: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineView {
    pub text: String,
    pub width: f32,
}

fn layout_view(text: &str, layout: &Layout, box_width: f32, limit: Option<f32>, engine: String) -> LayoutView {
    let bytes = text.as_bytes();
    LayoutView {
        box_width,
        limit,
        width: layout.width,
        height: layout.height,
        missing_glyphs: layout.missing_glyphs,
        word_overflow: layout.word_overflow,
        lines: layout
            .lines
            .iter()
            .map(|l| {
                // 엔진이 준 바이트 위치. 범위를 벗어나거나 글자 중간이어도 죽지 않게 한다.
                let end = l.end.min(bytes.len());
                let start = l.start.min(end);
                LineView {
                    text: String::from_utf8_lossy(&bytes[start..end]).into_owned(),
                    width: l.width,
                }
            })
            .collect(),
        engine,
    }
}

/// 박스 폭에 맞춰 잰다. 박스 폭이 있으면 엔진 줄바꿈을 켠다.
fn measure(p: &mut Prepared<'_>, text: &str, box_width: f32) -> std::result::Result<Layout, MeasureError> {
    let mut style = p.ctx.style;
    if box_width > 0.0 {
        style.wrap = WrapMode::Engine;
    }
    p.ctx.measurer.layout(&p.ctx.fonts, &style, text, box_width)
}

/// 프로필(저장하지 않은 것도 됨)로 문구 하나를 잰다. 렌더 탭의 미리 보기용. `box_width`가 0 이면 한 줄.
pub fn measure_text(
    conn: &Connection,
    engines: &mut EngineSet,
    profile: &RenderProfileInput,
    locale: &str,
    text: &str,
    box_width: f32,
) -> Result<LayoutView> {
    let mut input = profile.clone();
    if input.name.trim().is_empty() {
        input.name = "(미리 보기)".into();
    }
    let profile = check_profile(conn, &input)?;
    let mut session = RenderSession::new(conn, Some(engines))?;
    let mut p = session
        .for_profile(&profile, locale)
        .map_err(|e| Error::Invalid(e.message().to_string()))?;
    let layout = measure(&mut p, text, box_width).map_err(|e| Error::Invalid(e.to_string()))?;
    let limit = (box_width > 0.0).then_some(box_width);
    Ok(layout_view(text, &layout, box_width, limit, p.info.engine))
}

/// 셀 하나의 렌더 검사 결과(통과 포함)와 측정 결과.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderCheck {
    pub finding: Finding,
    /// 측정 엔진을 쓸 수 없어 건너뛴 이유.
    pub skipped: Option<String>,
    pub layout: Option<LayoutView>,
}

/// (박스 폭, 한도). 줄 수·넘침 검사는 박스 폭에서 줄바꿈하고, 최대 폭 검사는 한 줄로 잰다.
fn preview_box(params: &Value) -> (f32, Option<f32>) {
    let num = |k: &str| params.get(k).and_then(Value::as_f64).map(|v| v as f32);
    match (num("box_width_px"), num("max_width_px")) {
        (Some(b), _) => (b, Some(b)),
        (None, Some(m)) => (0.0, Some(m)),
        (None, None) => (0.0, None),
    }
}

/// 셀(토큰 × 언어)에 걸린 렌더 제약을 `text`로 실행한다. 저장하지 않은 문구를 고치는 동안 폭·줄 수를 보여 줄 때 쓴다.
pub fn measure_cell(
    conn: &Connection,
    registry: &Registry,
    engines: &mut EngineSet,
    token_id: &str,
    locale: &str,
    text: &str,
) -> Result<Vec<RenderCheck>> {
    let key = token_key(conn, token_id)?;
    let tags: Vec<String> = conn
        .prepare("SELECT tag FROM token_tag WHERE token_id = ?1 ORDER BY tag")?
        .query_map([token_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let source = source_locale(conn)?.filter(|s| s != locale);
    let source_value = match &source {
        Some(s) => history::read_cell(conn, &CellKey::new(token_id, s))?.map(|c| c.value),
        None => None,
    };
    let constraints = load_constraints(conn)?;
    let mut session = RenderSession::new(conn, Some(engines))?;
    let mut out = Vec::new();
    for c in effective_constraints(&constraints, token_id, &tags, locale) {
        let Some(v) = registry.get(&c.validator).filter(|v| v.needs_render()) else {
            continue;
        };
        let mut finding = Finding::new(c);
        let (skipped, layout) = match session.for_params(&c.params, locale) {
            Err(e @ SetupError::Unavailable(_)) => (Some(e.message().to_string()), None),
            Err(e) => {
                finding.error = Some(e.message().to_string());
                (None, None)
            }
            Ok(mut p) => {
                let ctx = TextCtx {
                    token_key: &key,
                    locale,
                    value: text,
                    source_value: source_value.as_deref(),
                };
                match v.check(&ctx, &c.params, Some(&mut p.ctx)) {
                    Ok(o) => finding.outcome = Some(o),
                    Err(e) => finding.error = Some(e.to_string()),
                }
                let (box_width, limit) = preview_box(&c.params);
                let layout = measure(&mut p, text, box_width)
                    .ok()
                    .map(|l| layout_view(text, &l, box_width, limit, p.info.engine.clone()));
                finding.render = Some(p.info);
                (None, layout)
            }
        };
        out.push(RenderCheck {
            finding,
            skipped,
            layout,
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tms_measure::Line;

    use super::*;
    use crate::tokens;

    /// 글자당 크기 × 1, 공백에서만 줄바꿈하는 가짜 측정기. '□'는 폰트에 없는 글자로 본다.
    pub(crate) struct Fake {
        pub version: String,
        pub fonts_loaded: Arc<Mutex<usize>>,
    }

    impl Measurer for Fake {
        fn engine_version(&self) -> String {
            self.version.clone()
        }
        fn load_font(&mut self, ttf: &[u8]) -> std::result::Result<FontId, MeasureError> {
            if ttf == b"broken" {
                return Err(MeasureError::FontLoad);
            }
            let mut n = self.fonts_loaded.lock().unwrap();
            *n += 1;
            Ok(FontId(*n))
        }
        fn layout(
            &mut self,
            fonts: &[FontId],
            style: &Style,
            text: &str,
            box_width: f32,
        ) -> std::result::Result<Layout, MeasureError> {
            assert!(!fonts.is_empty());
            let w = |s: &str| s.chars().count() as f32 * style.size_px;
            let mut lines = Vec::new();
            let mut overflow = false;
            if box_width <= 0.0 || style.wrap == WrapMode::None {
                lines.push(Line { start: 0, end: text.len(), width: w(text) });
            } else {
                // 단어 = 공백으로 나눈 구간(바이트 위치)
                let mut words = Vec::new();
                let mut pos = 0;
                for word in text.split(' ') {
                    words.push((pos, pos + word.len()));
                    pos += word.len() + 1;
                }
                let (mut start, mut end) = words[0];
                for &(s, e) in &words {
                    if w(&text[s..e]) > box_width {
                        overflow = true;
                    }
                    if s == start {
                        continue;
                    }
                    if w(&text[start..e]) <= box_width {
                        end = e;
                    } else {
                        lines.push(Line { start, end, width: w(&text[start..end]) });
                        (start, end) = (s, e);
                    }
                }
                lines.push(Line { start, end, width: w(&text[start..end]) });
            }
            Ok(Layout {
                width: lines.iter().map(|l| l.width).fold(0.0, f32::max),
                height: lines.len() as f32 * style.size_px * 1.2,
                missing_glyphs: text.chars().filter(|c| *c == '□').count() as u32,
                word_overflow: overflow,
                lines,
            })
        }
    }

    /// 가짜 엔진을 쓰는 EngineSet. 경로 이름이 곧 엔진 버전이다(파일 이름에서 확장자를 뺀 것).
    pub(crate) fn fake_engines() -> (EngineSet, Arc<Mutex<usize>>) {
        let fonts_loaded = Arc::new(Mutex::new(0));
        let counter = fonts_loaded.clone();
        let set = EngineSet::with_loader(Box::new(move |path| {
            let version = path.file_stem().unwrap().to_string_lossy().into_owned();
            if version == "bad" {
                return Err(MeasureError::Load("가짜 오류".into()));
            }
            Ok(Box::new(Fake { version, fonts_loaded: counter.clone() }) as Box<dyn Measurer + Send>)
        }));
        (set, fonts_loaded)
    }

    /// 임시 폴더에 프로젝트 파일과 가짜 DLL 파일을 만든다.
    pub(crate) fn temp_project(name: &str) -> (PathBuf, Connection) {
        let dir = std::env::temp_dir().join("lingua-tms-test").join(format!("{name}-{}", new_id()));
        std::fs::create_dir_all(dir.join("engines")).unwrap();
        let conn = db::open(dir.join("p.tms")).unwrap();
        (dir, conn)
    }

    pub(crate) fn profile_input(engine: &str, font: &str) -> RenderProfileInput {
        RenderProfileInput {
            id: None,
            name: "버튼".into(),
            engine_id: engine.into(),
            size_px: 10.0,
            letter_spacing: 0.0,
            line_height: 0.0,
            wrap: WrapMode::None,
            fonts: BTreeMap::from([(DEFAULT_FONTS.to_string(), vec![font.to_string()])]),
        }
    }

    #[test]
    fn registers_engine_with_relative_path_and_moves_it() {
        let (dir, mut conn) = temp_project("engine");
        let (mut engines, _) = fake_engines();
        let dll = dir.join("engines").join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();

        let e = register_engine(&mut conn, "t", &mut engines, &dll, Some("첫 버전")).unwrap();
        assert_eq!(e.id, "v1");
        assert_eq!(Path::new(&e.dll_path), Path::new("engines").join("v1.dll"));
        let info = &engine_infos(&conn, &engines).unwrap()[0];
        assert!(info.exists && info.loaded);
        assert_eq!(Path::new(&info.resolved_path), dll);

        // 밖으로 옮겨 다시 등록하면 경로만 바뀌고 메모는 남는다
        let outside = std::env::temp_dir().join("lingua-tms-test").join(format!("v1-{}", new_id()));
        std::fs::create_dir_all(&outside).unwrap();
        let moved = outside.join("v1.dll");
        std::fs::write(&moved, b"x").unwrap();
        let e = register_engine(&mut conn, "t", &mut engines, &moved, None).unwrap();
        assert_eq!(Path::new(&e.dll_path), moved);
        assert_eq!(e.note.as_deref(), Some("첫 버전"));
        assert_eq!(list_engines(&conn).unwrap().len(), 1);

        let bad = dir.join("engines").join("bad.dll");
        std::fs::write(&bad, b"x").unwrap();
        assert!(register_engine(&mut conn, "t", &mut engines, &bad, None).is_err());
        assert!(register_engine(&mut conn, "t", &mut engines, &dir.join("none.dll"), None).is_err());
    }

    #[test]
    fn fonts_dedupe_and_profiles_check_references() {
        let (dir, mut conn) = temp_project("profile");
        let (mut engines, _) = fake_engines();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        let dll = dir.join("engines").join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();
        register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();

        let a = add_font_bytes(&mut conn, "t", "Noto Sans.ttf", b"font-a").unwrap();
        assert_eq!(a.family, "Noto Sans");
        assert_eq!(add_font_bytes(&mut conn, "t", "copy.ttf", b"font-a").unwrap().id, a.id);
        assert_eq!(list_fonts(&conn).unwrap().len(), 1);

        // 확인: 기본 폰트 필수, 없는 언어·폰트·엔진, 같은 폰트 두 번
        let mut p = profile_input("v1", &a.id);
        p.fonts = BTreeMap::new();
        assert!(save_profile(&mut conn, "t", &p).is_err());
        let mut p = profile_input("v1", &a.id);
        p.fonts.insert("xx-XX".into(), vec![a.id.clone()]);
        assert!(save_profile(&mut conn, "t", &p).is_err());
        let mut p = profile_input("v1", &a.id);
        p.fonts.insert(DEFAULT_FONTS.into(), vec![a.id.clone(), a.id.clone()]);
        assert!(save_profile(&mut conn, "t", &p).is_err());
        assert!(save_profile(&mut conn, "t", &profile_input("v9", &a.id)).is_err());
        assert!(save_profile(&mut conn, "t", &profile_input("v1", "none")).is_err());

        // 빈 언어 목록은 기본 목록을 쓰는 것으로 보고 뺀다
        let mut p = profile_input("v1", &a.id);
        p.fonts.insert("ko-KR".into(), vec![]);
        let id = save_profile(&mut conn, "t", &p).unwrap();
        let saved = get_profile(&conn, &id).unwrap();
        assert_eq!(saved.fonts.len(), 1);
        assert_eq!(saved.fonts_for("ko-KR"), std::slice::from_ref(&a.id));

        // 같은 이름은 거부, 바뀐 게 없으면 변경 묶음을 만들지 않음
        assert!(save_profile(&mut conn, "t", &profile_input("v1", &a.id)).is_err());
        let count = |conn: &Connection| -> i64 { conn.query_row("SELECT count(*) FROM changeset", [], |r| r.get(0)).unwrap() };
        let before = count(&conn);
        let mut same = p.clone();
        same.id = Some(id.clone());
        save_profile(&mut conn, "t", &same).unwrap();
        assert_eq!(count(&conn), before);

        // 쓰는 곳이 있으면 지울 수 없음
        assert!(delete_font(&mut conn, "t", &a.id).is_err());
        assert!(delete_engine(&mut conn, "t", &mut engines, "v1").is_err());
        crate::validate::save_constraint(
            &mut conn,
            "t",
            &Registry::builtin(),
            &crate::validate::ConstraintInput {
                id: None,
                validator: "render.glyph_coverage".into(),
                params: json!({ "profile": id }),
                scope: crate::validate::Scope::Global,
                locale: None,
                severity: crate::validate::Severity::Error,
                enabled: true,
                description: None,
            },
        )
        .unwrap();
        assert!(delete_profile(&mut conn, "t", &id).is_err());
    }

    #[test]
    fn measures_text_and_loads_each_font_once() {
        let (dir, mut conn) = temp_project("measure");
        let (mut engines, fonts_loaded) = fake_engines();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "ja-JP", "日本語", false).unwrap();
        let dll = dir.join("engines").join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();
        register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();
        let a = add_font_bytes(&mut conn, "t", "a.ttf", b"a").unwrap();
        let b = add_font_bytes(&mut conn, "t", "b.ttf", b"b").unwrap();
        let broken = add_font_bytes(&mut conn, "t", "broken.ttf", b"broken").unwrap();

        let mut p = profile_input("v1", &a.id);
        p.fonts.insert("ja-JP".into(), vec![b.id.clone(), a.id.clone()]);
        let v = measure_text(&conn, &mut engines, &p, "ko-KR", "확인 취소", 0.0).unwrap();
        assert_eq!((v.width, v.lines.len()), (50.0, 1));
        let v = measure_text(&conn, &mut engines, &p, "ko-KR", "확인 취소", 30.0).unwrap();
        assert_eq!(v.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["확인", "취소"]);
        assert_eq!(v.limit, Some(30.0));
        measure_text(&conn, &mut engines, &p, "ja-JP", "確認", 0.0).unwrap();
        // a, b 를 한 번씩만 불러 둔다
        assert_eq!(*fonts_loaded.lock().unwrap(), 2);

        // 엔진이 못 읽는 폰트는 설정 오류
        let p = profile_input("v1", &broken.id);
        let err = measure_text(&conn, &mut engines, &p, "ko-KR", "x", 0.0).unwrap_err();
        assert!(err.to_string().contains("broken"), "{err}");
    }

    #[test]
    fn measure_cell_runs_render_rules_on_unsaved_text() {
        use crate::validate::{save_constraint, ConstraintInput, Scope, Severity, Verdict};
        let (dir, mut conn) = temp_project("cell");
        let (mut engines, _) = fake_engines();
        let reg = Registry::builtin();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        let dll = dir.join("engines").join("v1.dll");
        std::fs::write(&dll, b"x").unwrap();
        register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();
        let font = add_font_bytes(&mut conn, "t", "a.ttf", b"a").unwrap();
        let profile = save_profile(&mut conn, "t", &profile_input("v1", &font.id)).unwrap();
        let token = tokens::create_token(&mut conn, "t", "common.ok", None).unwrap();
        let rule = |validator: &str, params: Value| ConstraintInput {
            id: None,
            validator: validator.into(),
            params,
            scope: Scope::Token(token.clone()),
            locale: None,
            severity: Severity::Error,
            enabled: true,
            description: None,
        };
        save_constraint(&mut conn, "t", &reg, &rule("render.line_count", json!({ "profile": profile, "box_width_px": 30, "max": 1 }))).unwrap();
        // 텍스트 검증기는 여기서 보지 않는다
        save_constraint(&mut conn, "t", &reg, &rule("text.required", json!({}))).unwrap();

        let checks = measure_cell(&conn, &reg, &mut engines, &token, "ko-KR", "확인 취소").unwrap();
        assert_eq!(checks.len(), 1);
        let c = &checks[0];
        assert_eq!(c.finding.outcome.as_ref().map(|o| o.verdict), Some(Verdict::Fail));
        let layout = c.layout.as_ref().unwrap();
        assert_eq!(layout.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["확인", "취소"]);
        assert_eq!(layout.limit, Some(30.0));
        assert_eq!(c.finding.render.as_ref().map(|r| r.engine.as_str()), Some("v1"));

        let checks = measure_cell(&conn, &reg, &mut engines, &token, "ko-KR", "확인").unwrap();
        assert_eq!(checks[0].finding.outcome.as_ref().map(|o| o.verdict), Some(Verdict::Pass));

        // 이 PC 에 DLL 이 없으면 건너뛴 이유를 돌려준다
        std::fs::remove_file(&dll).unwrap();
        let (mut fresh, _) = fake_engines();
        let checks = measure_cell(&conn, &reg, &mut fresh, &token, "ko-KR", "확인").unwrap();
        assert!(checks[0].skipped.as_deref().is_some_and(|s| s.contains("DLL이 없음")));
    }
}
