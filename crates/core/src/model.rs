use serde::{Deserialize, Serialize};

/// 셀 = 토큰 × 언어.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellKey {
    pub token_id: String,
    pub locale: String,
}

impl CellKey {
    pub fn new(token_id: impl Into<String>, locale: impl Into<String>) -> Self {
        Self {
            token_id: token_id.into(),
            locale: locale.into(),
        }
    }
}

/// 셀의 내용. 행이 없으면(None) 미번역이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellValue {
    pub value: String,
    /// translated | reviewed | approved
    pub status: String,
}

impl CellValue {
    pub fn translated(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            status: "translated".into(),
        }
    }
}

/// 셀 하나에 대한 변경. `new`가 None이면 삭제(미번역으로 되돌림).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellChange {
    pub key: CellKey,
    pub new: Option<CellValue>,
}

/// 화면에서 셀 하나를 고친 값. 빈 문자열은 미번역으로 되돌린다는 뜻이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellEdit {
    pub token_id: String,
    pub locale: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangesetKind {
    Edit,
    Import,
    Revert,
    Rename,
    Admin,
}

impl ChangesetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Import => "import",
            Self::Revert => "revert",
            Self::Rename => "rename",
            Self::Admin => "admin",
        }
    }
}

/// 변경 묶음을 만들 때 넘기는 정보. 단일 사용자 앱이므로 author 는 앱 설정의 작업자 이름이다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetMeta {
    pub author: String,
    pub kind: ChangesetKind,
    pub message: Option<String>,
    pub reverts_id: Option<i64>,
}

impl ChangesetMeta {
    pub fn new(author: impl Into<String>, kind: ChangesetKind) -> Self {
        Self {
            author: author.into(),
            kind,
            message: None,
            reverts_id: None,
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// 셀 이력 한 줄.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevRow {
    pub changeset_id: i64,
    pub op: String,
    pub old: Option<CellValue>,
    pub new: Option<CellValue>,
    pub author: String,
    pub created_at: String,
}

/// 두 시점 사이에 바뀐 셀.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellDiff {
    pub key: CellKey,
    pub old: Option<CellValue>,
    pub new: Option<CellValue>,
}
