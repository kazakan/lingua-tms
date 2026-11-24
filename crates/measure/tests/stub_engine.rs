//! 가짜 엔진 DLL(crates/engine-stub)로 FFI 경로 전체를 확인한다.
//!
//! DLL은 `cargo test --workspace`(target/debug/deps 에 생김)나
//! `cargo build -p tms-engine-stub`(target/debug 에 생김)로 만든다.
//! DLL 위치는 TMS_ENGINE_DLL 환경 변수로 줄 수 있고, 없으면 위 두 곳에서 찾는다.
//! 엔진 DLL은 경우에 따라 없을 수 있으므로, 못 찾으면 실패하지 않고 건너뛴다.

use std::path::PathBuf;

use tms_measure::{EngineDll, Measurer, Style, WrapMode};

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
    // cargo build 는 cdylib 를 target/debug 로 올려 두지만, cargo test 는 deps 에만 남긴다.
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug");
    [debug.join(file), debug.join("deps").join(file)]
        .into_iter()
        .find(|p| p.exists())
}

#[test]
fn layout_through_stub_dll() {
    let Some(path) = stub_path() else {
        eprintln!("엔진 스텁 DLL이 없어 건너뜀: cargo build -p tms-engine-stub");
        return;
    };
    eprintln!("엔진 DLL: {}", path.display());
    let mut engine = EngineDll::load(&path).expect("DLL 로드");
    assert!(engine.engine_version().starts_with("stub-"));

    let font = engine.load_font(&[0u8; 16]).expect("폰트 로드");
    let style = Style {
        size_px: 20.0,
        letter_spacing: 0.0,
        line_height: 0.0,
        wrap: WrapMode::Engine,
    };

    // 한 줄 측정: 한글 2자 = 20px * 2
    let one = engine.layout(&[font], &style, "확인", 0.0).unwrap();
    assert_eq!(one.line_count(), 1);
    assert!((one.width - 40.0).abs() < 0.01);

    // 박스 폭 50px: "확인 취소" 는 두 줄
    let two = engine.layout(&[font], &style, "확인 취소", 50.0).unwrap();
    assert_eq!(two.line_count(), 2);
    assert!(!two.word_overflow);

    // 박스보다 긴 단어는 넘침으로 보고
    let over = engine.layout(&[font], &style, "가나다라마", 50.0).unwrap();
    assert!(over.word_overflow);
}
