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
}

pub type Result<T> = std::result::Result<T, Error>;
