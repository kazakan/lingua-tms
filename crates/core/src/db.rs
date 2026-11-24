//! 프로젝트 파일(SQLite) 열기와 마이그레이션.
//!
//! 프로젝트 하나 = DB 파일 하나. 문구, 이력, 폰트, 제약이 모두 이 파일에 들어간다.

use std::path::Path;

use rusqlite::Connection;

use crate::Result;

/// (버전, SQL). 추가만 한다. 이미 배포된 항목은 고치지 않는다.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

/// 프로젝트 파일을 연다. 없으면 만든다. 스키마를 최신으로 올린다.
pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 테스트용 메모리 DB.
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // 외래 키는 연결마다 켜야 한다(기본값 OFF).
    conn.pragma_update(None, "foreign_keys", true)?;
    // journal_mode 는 결과 행을 돌려주므로 query_row 로 설정한다. 메모리 DB는 "memory"가 된다.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn migrate(conn: &Connection) -> Result<()> {
    let current = schema_version(conn)?;
    for (version, sql) in MIGRATIONS {
        if *version > current {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_to_latest() {
        let conn = open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), MIGRATIONS.last().unwrap().0);
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1);
    }
}
