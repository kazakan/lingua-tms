//! TMS 핵심 로직. 화면(Tauri, React)과 무관한 라이브러리다.
//!
//! - [`db`]: 프로젝트 파일(SQLite) 열기, 연결 설정, 마이그레이션
//! - [`history`]: 문구값 쓰기의 유일한 경로(`apply`)와 이력 조회·diff·롤백
//! - [`tokens`]: 토큰·언어 관리(토큰명 변경 이력 포함)
//! - [`validate`]: 검증기 레지스트리, 내장 검증기, 제약 해석
//! - [`sheet`]: 엑셀·ODS 읽기와 열 매핑, [`values`]: 값 표(토큰명 × 언어)와 비교
//! - [`import`]: 가져오기 계획(미리보기)과 적용
//! - [`export`]: 내보내기 대상, 제품 키 고정, 포맷 어댑터(내보내기·역가져오기)

pub mod db;
pub mod error;
pub mod export;
pub mod grid;
pub mod history;
pub mod ids;
pub mod import;
pub mod model;
pub mod sheet;
pub mod tokens;
pub mod validate;
pub mod values;

pub use error::{Error, Result};
/// 앱 크레이트가 같은 버전을 쓰도록 다시 내보낸다.
pub use rusqlite;
