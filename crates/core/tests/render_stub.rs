//! 가짜 엔진 DLL(crates/engine-stub)로 렌더 검증 전체 경로를 확인한다:
//! DLL 등록 → 폰트·렌더 프로필 → 렌더 규칙 → 전체 검증·셀 측정.
//!
//! DLL 찾는 법은 `crates/measure/tests/stub_engine.rs`와 같다(TMS_ENGINE_DLL, target/debug, target/debug/deps).
//! 엔진 DLL 은 경우에 따라 없을 수 있으므로, 못 찾으면 실패하지 않고 건너뛴다.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::json;
use tms_core::history::apply;
use tms_core::model::{CellChange, CellKey, CellValue, ChangesetKind, ChangesetMeta};
use tms_core::render::{self, EngineSet, RenderProfileInput, DEFAULT_FONTS};
use tms_core::validate::{self, ConstraintInput, Registry, Scope, Severity};
use tms_core::{db, tokens};
use tms_measure::WrapMode;

fn stub_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TMS_ENGINE_DLL") {
        return Some(PathBuf::from(p));
    }
    let file = if cfg!(windows) {
        "tms_measure_stub.dll"
    } else if cfg!(target_os = "macos") {
        "libtms_measure_stub.dylib"
    } else {
        "libtms_measure_stub.so"
    };
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug");
    [debug.join(file), debug.join("deps").join(file)]
        .into_iter()
        .find(|p| p.exists())
}

#[test]
fn render_rules_through_stub_dll() {
    let Some(dll) = stub_path() else {
        eprintln!("엔진 스텁 DLL이 없어 건너뜀: cargo build -p tms-engine-stub");
        return;
    };
    eprintln!("엔진 DLL: {}", dll.display());
    let dir = std::env::temp_dir()
        .join("lingua-tms-test")
        .join(format!("stub-{}", tms_core::ids::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut conn = db::open(dir.join("p.tms")).unwrap();
    tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
    tokens::add_locale(&conn, "en-US", "English", false).unwrap();

    let mut engines = EngineSet::dll();
    let engine = render::register_engine(&mut conn, "t", &mut engines, &dll, None).unwrap();
    assert!(engine.id.starts_with("stub-"), "{}", engine.id);
    // 스텁은 폰트 내용을 보지 않는다
    let font = render::add_font_bytes(&mut conn, "t", "dummy.ttf", &[0u8; 16]).unwrap();
    let profile = render::save_profile(
        &mut conn,
        "t",
        &RenderProfileInput {
            id: None,
            name: "본문 20px".into(),
            engine_id: engine.id.clone(),
            size_px: 20.0,
            letter_spacing: 0.0,
            line_height: 0.0,
            wrap: WrapMode::Engine,
            fonts: BTreeMap::from([(DEFAULT_FONTS.to_string(), vec![font.id.clone()])]),
        },
    )
    .unwrap();

    let token = tokens::create_token(&mut conn, "t", "common.cancel", None).unwrap();
    let set = |l: &str, v: &str| CellChange {
        key: CellKey::new(&token, l),
        new: Some(CellValue::translated(v)),
    };
    apply(
        &mut conn,
        &ChangesetMeta::new("t", ChangesetKind::Edit),
        &[set("ko-KR", "확인 취소"), set("en-US", "Cancel")],
    )
    .unwrap();
    let reg = Registry::builtin();
    validate::save_constraint(
        &mut conn,
        "t",
        &reg,
        &ConstraintInput {
            id: None,
            validator: "render.line_count".into(),
            params: json!({ "profile": profile, "box_width_px": 50, "max": 1 }),
            scope: Scope::Global,
            locale: None,
            severity: Severity::Error,
            enabled: true,
            description: None,
        },
    )
    .unwrap();

    // 스텁: 한글 20px, 공백 6px. "확인 취소"는 박스 50px 에서 두 줄, "Cancel"은 한 줄(단어가 넘치지만 줄 수는 1)
    let report = validate::validate_all(&conn, &reg, Some(&mut engines)).unwrap();
    assert_eq!(report.summary.skipped_render, 0, "{:?}", report.render_skipped);
    let failed: Vec<_> = report.cells.iter().map(|c| c.locale.as_str()).collect();
    assert_eq!(failed, vec!["ko-KR"]);
    let f = &report.cells[0].findings[0];
    assert_eq!(f.outcome.as_ref().and_then(|o| o.message.as_deref()), Some("2줄 (기대: 1줄 이하)"));
    assert_eq!(f.render.as_ref().map(|r| r.engine.as_str()), Some(engine.id.as_str()));

    // 고치는 중인 문구로 재기
    let checks = render::measure_cell(&conn, &reg, &mut engines, &token, "ko-KR", "확인").unwrap();
    let layout = checks[0].layout.as_ref().unwrap();
    assert_eq!(layout.lines.len(), 1);
    assert!((layout.width - 40.0).abs() < 0.01);
}
