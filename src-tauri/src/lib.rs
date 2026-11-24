//! Tauri 명령. 얇게 유지한다: 인자 받고 → tms-core 호출 → 결과를 JSON 으로.
//! 로직이 생기면 tms-core 로 옮기고 거기서 테스트한다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{Manager, State};
use tms_core::import::{ImportOptions, ImportPlan, ImportResult, ImportSource};
use tms_core::model::{CellEdit, CellKey, ChangesetKind, ChangesetMeta, RevRow};
use tms_core::rusqlite::Connection;
use tms_core::validate::Registry;
use tms_core::{db, export, grid, history, import, sheet, tokens};

struct Project {
    path: PathBuf,
    conn: Connection,
}

struct AppState {
    project: Mutex<Option<Project>>,
    registry: Registry,
    author: String,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 열린 프로젝트 연결로 작업한다. 열린 프로젝트가 없으면 오류.
fn with_project<T>(state: &AppState, f: impl FnOnce(&mut Connection) -> tms_core::Result<T>) -> CmdResult<T> {
    let mut guard = state.project.lock().map_err(err)?;
    let project = guard.as_mut().ok_or("열린 프로젝트가 없음")?;
    f(&mut project.conn).map_err(err)
}

#[derive(Serialize)]
struct ProjectInfo {
    path: String,
    schema_version: i64,
}

/// 프로젝트 파일(.tms, SQLite) 열기. 없으면 만들고 마이그레이션한다.
#[tauri::command]
fn open_project(state: State<'_, AppState>, path: String) -> CmdResult<ProjectInfo> {
    let conn = db::open(&path).map_err(err)?;
    let schema_version = db::schema_version(&conn).map_err(err)?;
    *state.project.lock().map_err(err)? = Some(Project {
        path: PathBuf::from(&path),
        conn,
    });
    Ok(ProjectInfo { path, schema_version })
}

#[tauri::command]
fn project_info(state: State<'_, AppState>) -> CmdResult<Option<ProjectInfo>> {
    let guard = state.project.lock().map_err(err)?;
    guard
        .as_ref()
        .map(|p| {
            Ok(ProjectInfo {
                path: p.path.display().to_string(),
                schema_version: db::schema_version(&p.conn).map_err(err)?,
            })
        })
        .transpose()
}

#[tauri::command]
fn load_grid(state: State<'_, AppState>) -> CmdResult<grid::Grid> {
    with_project(&state, |conn| grid::load_grid(conn))
}

#[tauri::command]
fn add_locale(state: State<'_, AppState>, code: String, name: String, is_source: bool) -> CmdResult<()> {
    with_project(&state, |conn| tokens::add_locale(conn, &code, &name, is_source))
}

#[tauri::command]
fn list_locales(state: State<'_, AppState>) -> CmdResult<Vec<tokens::Locale>> {
    with_project(&state, |conn| tokens::list_locales(conn))
}

#[tauri::command]
fn list_tokens(state: State<'_, AppState>) -> CmdResult<Vec<tokens::Token>> {
    with_project(&state, |conn| tokens::list_tokens(conn))
}

#[tauri::command]
fn create_token(state: State<'_, AppState>, key: String, description: Option<String>) -> CmdResult<String> {
    let author = state.author.clone();
    with_project(&state, |conn| tokens::create_token(conn, &author, &key, description.as_deref()))
}

#[tauri::command]
fn rename_token(state: State<'_, AppState>, token_id: String, new_key: String) -> CmdResult<Option<i64>> {
    let author = state.author.clone();
    with_project(&state, |conn| tokens::rename_token(conn, &author, &token_id, &new_key))
}

/// 셀 편집(하나 또는 여러 셀 붙여넣기)을 변경 묶음 하나로 쓴다. 빈 문자열은 미번역. 바뀐 게 없으면 None.
#[tauri::command]
fn set_cells(state: State<'_, AppState>, edits: Vec<CellEdit>) -> CmdResult<Option<i64>> {
    let meta = ChangesetMeta::new(state.author.clone(), ChangesetKind::Edit);
    with_project(&state, |conn| history::apply_edits(conn, &meta, &edits))
}

#[tauri::command]
fn cell_history(state: State<'_, AppState>, token_id: String, locale: String) -> CmdResult<Vec<RevRow>> {
    with_project(&state, |conn| history::cell_history(conn, &CellKey::new(token_id, locale)))
}

/// 엑셀·ODS 파일의 시트 목록과 앞쪽 행(열 매핑 화면용).
#[tauri::command]
fn workbook_preview(path: String) -> CmdResult<Vec<sheet::SheetPreview>> {
    sheet::workbook_preview(Path::new(&path), 30).map_err(err)
}

/// 가져오기 미리보기. DB는 바꾸지 않는다.
#[tauri::command]
fn plan_import(state: State<'_, AppState>, source: ImportSource, options: ImportOptions) -> CmdResult<ImportPlan> {
    with_project(&state, |conn| {
        let table = source.load(conn)?;
        import::plan_import(conn, &table, &options)
    })
}

/// 가져오기 적용(변경 묶음 하나).
#[tauri::command]
fn apply_import(state: State<'_, AppState>, source: ImportSource, options: ImportOptions) -> CmdResult<ImportResult> {
    let author = state.author.clone();
    with_project(&state, |conn| {
        let table = source.load(conn)?;
        import::apply_import(conn, &author, &table, &options, &source.label())
    })
}

#[tauri::command]
fn list_adapters() -> Vec<export::AdapterInfo> {
    export::list_adapters()
}

#[tauri::command]
fn list_export_targets(state: State<'_, AppState>) -> CmdResult<Vec<export::ExportTarget>> {
    with_project(&state, |conn| export::list_targets(conn))
}

#[tauri::command]
fn save_export_target(state: State<'_, AppState>, target: export::ExportTargetInput) -> CmdResult<String> {
    let author = state.author.clone();
    with_project(&state, |conn| export::save_target(conn, &author, &target))
}

#[tauri::command]
fn delete_export_target(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let author = state.author.clone();
    with_project(&state, |conn| export::delete_target(conn, &author, &id))
}

/// 내보낼 내용 미리보기(제품 키 제안 포함). 파일을 쓰지 않는다.
#[tauri::command]
fn plan_export(state: State<'_, AppState>, target_id: String) -> CmdResult<export::ExportPreview> {
    with_project(&state, |conn| export::plan_export(conn, &target_id))
}

/// 파일로 내보내고 새 제품 키를 고정한다.
#[tauri::command]
fn export_to_file(
    state: State<'_, AppState>,
    target_id: String,
    path: String,
    note: Option<String>,
) -> CmdResult<export::ExportReport> {
    let author = state.author.clone();
    with_project(&state, |conn| {
        export::export_to_file(conn, &author, &target_id, Path::new(&path), note.as_deref())
    })
}

/// 아직 키가 없는 토큰에 제품 키를 직접 지정해 고정한다.
#[tauri::command]
fn fix_external_key(state: State<'_, AppState>, target_id: String, token_id: String, key: String) -> CmdResult<i64> {
    let author = state.author.clone();
    with_project(&state, |conn| export::fix_external_key(conn, &author, &target_id, &token_id, &key))
}

#[derive(Serialize)]
struct ValidatorInfo {
    id: &'static str,
    description: &'static str,
    params_schema: Value,
    needs_render: bool,
}

/// 검증기 목록. 화면은 params_schema 로 제약 입력 폼을 만든다.
#[tauri::command]
fn list_validators(state: State<'_, AppState>) -> Vec<ValidatorInfo> {
    state
        .registry
        .iter()
        .map(|v| ValidatorInfo {
            id: v.id(),
            description: v.description(),
            params_schema: v.params_schema(),
            needs_render: v.needs_render(),
        })
        .collect()
}

fn os_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 두 번째 실행은 새 창 대신 기존 창을 앞으로(같은 .tms 파일을 두 프로세스가 쓰지 않게).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            project: Mutex::new(None),
            registry: Registry::builtin(),
            author: os_user(),
        })
        .invoke_handler(tauri::generate_handler![
            open_project,
            project_info,
            load_grid,
            add_locale,
            list_locales,
            list_tokens,
            create_token,
            rename_token,
            set_cells,
            cell_history,
            workbook_preview,
            plan_import,
            apply_import,
            list_adapters,
            list_export_targets,
            save_export_target,
            delete_export_target,
            plan_export,
            export_to_file,
            fix_external_key,
            list_validators,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패");
}
