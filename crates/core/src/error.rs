#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("찾을 수 없음: {0}")]
    NotFound(String),
    #[error("잘못된 입력: {0}")]
    Invalid(String),
    /// 엑셀·ODS 읽기 또는 쓰기 실패.
    #[error("스프레드시트 오류: {0}")]
    Sheet(String),
}

impl From<calamine::Error> for Error {
    fn from(e: calamine::Error) -> Self {
        Self::Sheet(e.to_string())
    }
}

impl From<rust_xlsxwriter::XlsxError> for Error {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        Self::Sheet(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
