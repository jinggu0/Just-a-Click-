//! The validation database: lecture segments and one index per method.
//!
//! This schema exists to compare search methods. The product schema is decided later
//! (roadmap item 10), so nothing here is a migration target.
use std::path::Path;

use rusqlite::Connection;

pub const SEGMENT_CHARACTERS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Trigram,
    Unicode61,
    Like,
}

impl Method {
    pub fn all() -> [Method; 3] {
        [Method::Trigram, Method::Unicode61, Method::Like]
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::Trigram => "trigram",
            Method::Unicode61 => "unicode61",
            Method::Like => "like",
        }
    }

    pub fn from_label(label: &str) -> Option<Method> {
        Method::all().into_iter().find(|method| method.label() == label)
    }
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub lecture: i64,
    pub body: String,
}

pub fn open(path: &Path) -> Result<Connection, String> {
    Connection::open(path).map_err(|error| format!("database failed: {error}"))
}

/// One table of segments plus the index the method needs. `like` needs no index.
pub fn create_schema(connection: &Connection, method: Method) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS segment(
                 id INTEGER PRIMARY KEY,
                 lecture_id INTEGER NOT NULL,
                 body TEXT NOT NULL);",
        )
        .map_err(|error| format!("schema failed: {error}"))?;
    let index = match method {
        Method::Trigram => Some(
            "CREATE VIRTUAL TABLE IF NOT EXISTS segment_index USING fts5(
                 body, tokenize='trigram', content='segment', content_rowid='id');",
        ),
        Method::Unicode61 => Some(
            "CREATE VIRTUAL TABLE IF NOT EXISTS segment_index USING fts5(
                 body, tokenize='unicode61', content='segment', content_rowid='id');",
        ),
        Method::Like => None,
    };
    if let Some(statement) = index {
        connection
            .execute_batch(statement)
            .map_err(|error| format!("index failed: {error}"))?;
    }
    Ok(())
}

pub fn insert_segments(connection: &Connection, rows: &[Segment]) -> Result<(), String> {
    connection.execute_batch("BEGIN").map_err(|error| error.to_string())?;
    {
        let mut insert = connection
            .prepare("INSERT INTO segment(id, lecture_id, body) VALUES (?1, ?2, ?3)")
            .map_err(|error| error.to_string())?;
        for (index, row) in rows.iter().enumerate() {
            insert
                .execute((index as i64 + 1, row.lecture, &row.body))
                .map_err(|error| format!("insert failed: {error}"))?;
        }
    }
    connection.execute_batch("COMMIT").map_err(|error| error.to_string())
}

/// External content tables do not index anything until they are rebuilt.
pub fn rebuild_index(connection: &Connection, method: Method) -> Result<(), String> {
    if method == Method::Like {
        return Ok(());
    }
    connection
        .execute_batch("INSERT INTO segment_index(segment_index) VALUES('rebuild')")
        .map_err(|error| format!("rebuild failed: {error}"))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Segment> {
        vec![
            Segment { lecture: 1, body: "확률변수의 기댓값은 분포에 따라 달라집니다".to_string() },
            Segment { lecture: 1, body: "정규분포에서 표준편차가 커지면 폭이 넓어집니다".to_string() },
            Segment { lecture: 2, body: "GPU 메모리가 부족하면 배치를 줄여야 합니다".to_string() },
        ]
    }

    fn loaded(method: Method) -> Connection {
        let connection = Connection::open_in_memory().expect("memory database");
        create_schema(&connection, method).expect("schema");
        insert_segments(&connection, &rows()).expect("insert");
        rebuild_index(&connection, method).expect("rebuild");
        connection
    }

    #[test]
    fn the_bundled_sqlite_is_new_enough_for_trigram() {
        let connection = Connection::open_in_memory().expect("memory database");
        let version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .expect("version");
        let numbers: Vec<u32> = version.split('.').filter_map(|part| part.parse().ok()).collect();
        assert!(numbers[0] > 3 || (numbers[0] == 3 && numbers[1] >= 34), "sqlite {version} has no trigram");
    }

    #[test]
    fn every_method_loads_the_same_rows() {
        for method in Method::all() {
            let connection = loaded(method);
            let count: i64 = connection
                .query_row("SELECT count(*) FROM segment", [], |row| row.get(0))
                .expect("count");
            assert_eq!(count, 3, "{} lost rows", method.label());
        }
    }

    #[test]
    fn the_label_round_trips() {
        for method in Method::all() {
            assert_eq!(Method::from_label(method.label()), Some(method));
        }
        assert_eq!(Method::from_label("none"), None);
    }
}
