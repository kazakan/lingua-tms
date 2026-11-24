//! Tauri 명령. 얇게 유지한다: 인자 받고 → tms-core 호출 → 결과를 JSON 으로.
//! 로직이 생기면 tms-core 로 옮기고 거기서 테스트한다.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::{Manager, State};
use tms_core::model::{CellChange, CellKey, CellValue, ChangesetKind, ChangesetMeta, RevRow};
use tms_core::rusqlite::Connection;
use tms_core::validate::Registry;
use tms_core::{db, grid, history, tokens};

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
fn create_token(state: State<'_, AppState>, key: String, description: Option<String>) -> CmdResult<String> {
    let author = state.author.clone();
    with_project(&state, |conn| tokens::create_token(conn, &author, &key, description.as_deref()))
}

#[tauri::command]
fn rename_token(state: State<'_, AppState>, token_id: String, new_key: String) -> CmdResult<Option<i64>> {
    let author = state.author.clone();
    with_project(&state, |conn| tokens::rename_token(conn, &author, &token_id, &new_key))
}

/// 셀 하나 편집. 빈 문자열이면 미번역으로 되돌린다. 바뀐 게 없으면 None.
#[tauri::command]
fn set_cell(state: State<'_, AppState>, token_id: String, locale: String, value: String) -> CmdResult<Option<i64>> {
    let meta = ChangesetMeta::new(state.author.clone(), ChangesetKind::Edit);
    let change = CellChange {
        key: CellKey::new(token_id, locale),
        new: (!value.is_empty()).then(|| CellValue::translated(value)),
    };
    with_project(&state, |conn| history::apply(conn, &meta, &[change]))
}

#[tauri::command]
fn cell_history(state: State<'_, AppState>, token_id: String, locale: String) -> CmdResult<Vec<RevRow>> {
    with_project(&state, |conn| history::cell_history(conn, &CellKey::new(token_id, locale)))
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
            create_token,
            rename_token,
            set_cell,
            cell_history,
            list_validators,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패");
}
