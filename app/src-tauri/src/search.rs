//! Queries for the three methods and scores against the text itself.
//!
//! The truth set is a plain substring scan over the corpus, so no engine decides what a
//! correct answer is.
use rusqlite::Connection;

use crate::store::{Method, Segment};

/// Trigram indexes three characters at a time, so shorter queries cannot be answered.
pub const TRIGRAM_MINIMUM: usize = 3;

fn quoted(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

pub fn search(
    connection: &Connection,
    method: Method,
    query: &str,
    limit: usize,
) -> Result<Vec<i64>, String> {
    match method {
        Method::Like => rows(
            connection,
            "SELECT id FROM segment WHERE body LIKE ?1 ORDER BY id LIMIT ?2",
            (format!("%{query}%"), limit as i64),
        ),
        _ => rows(
            connection,
            "SELECT rowid FROM segment_index WHERE segment_index MATCH ?1 ORDER BY rowid LIMIT ?2",
            (quoted(query), limit as i64),
        ),
    }
}

/// The unicode61 fallback for a stem that carries a particle: match the token's prefix.
pub fn search_prefix(connection: &Connection, query: &str, limit: usize) -> Result<Vec<i64>, String> {
    rows(
        connection,
        "SELECT rowid FROM segment_index WHERE segment_index MATCH ?1 ORDER BY rowid LIMIT ?2",
        (format!("{}*", quoted(query)), limit as i64),
    )
}

fn rows(
    connection: &Connection,
    statement: &str,
    parameters: (String, i64),
) -> Result<Vec<i64>, String> {
    let mut prepared = connection.prepare(statement).map_err(|error| error.to_string())?;
    let found = prepared
        .query_map(rusqlite::params![parameters.0, parameters.1], |row| row.get(0))
        .map_err(|error| format!("query failed: {error}"))?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(|error| format!("query failed: {error}"))?;
    Ok(found)
}

/// Segment ids that really contain the query, numbered the way the database numbers them.
pub fn truth(rows: &[Segment], query: &str) -> Vec<i64> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.body.contains(query))
        .map(|(index, _)| index as i64 + 1)
        .collect()
}

/// How much of what the query should find came back, given the result limit.
pub fn recall(found: &[i64], truth: &[i64], limit: usize) -> f64 {
    let reachable = truth.len().min(limit);
    if reachable == 0 {
        return 1.0;
    }
    let hits = found.iter().filter(|id| truth.contains(id)).count();
    hits as f64 / reachable as f64
}

/// How much of what came back belongs there.
pub fn precision(found: &[i64], truth: &[i64]) -> f64 {
    if found.is_empty() {
        return 1.0;
    }
    let hits = found.iter().filter(|id| truth.contains(id)).count();
    hits as f64 / found.len() as f64
}

/// The rule the report recommends if the measurement confirms it: trigram for three
/// characters and more, a scan for the short queries trigram cannot index.
pub fn candidate_method(query: &str) -> Method {
    if query.chars().count() >= TRIGRAM_MINIMUM {
        Method::Trigram
    } else {
        Method::Like
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_schema, insert_segments, rebuild_index};
    use rusqlite::Connection;

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
    fn trigram_finds_a_stem_that_carries_a_particle() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "확률변수", 20).expect("search");
        assert_eq!(found, vec![1]);
    }

    #[test]
    fn trigram_finds_a_substring_inside_a_word() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "률변수", 20).expect("search");
        assert_eq!(found, vec![1]);
    }

    #[test]
    fn unicode61_misses_the_stem_but_a_prefix_query_finds_it() {
        let connection = loaded(Method::Unicode61);
        assert!(search(&connection, Method::Unicode61, "확률변수", 20).expect("search").is_empty());
        assert_eq!(search_prefix(&connection, "확률변수", 20).expect("prefix"), vec![1]);
    }

    #[test]
    fn like_finds_both_shapes() {
        let connection = loaded(Method::Like);
        assert_eq!(search(&connection, Method::Like, "확률변수", 20).expect("search"), vec![1]);
        assert_eq!(search(&connection, Method::Like, "률변수", 20).expect("search"), vec![1]);
    }

    #[test]
    fn a_two_character_query_is_out_of_reach_for_trigram() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "분포", 20).expect("search");
        assert!(found.is_empty(), "trigram indexes three characters at a time");
        let others = search(&loaded(Method::Like), Method::Like, "분포", 20).expect("search");
        assert_eq!(others, vec![1, 2]);
    }

    #[test]
    fn the_candidate_rule_sends_short_queries_to_like() {
        assert_eq!(candidate_method("분포"), Method::Like);
        assert_eq!(candidate_method("확률변수"), Method::Trigram);
    }

    #[test]
    fn truth_and_scores_come_from_the_text_itself() {
        let truth = truth(&rows(), "분포");
        assert_eq!(truth, vec![1, 2]);
        assert_eq!(recall(&[1, 2], &truth, 20), 1.0);
        assert_eq!(recall(&[1], &truth, 20), 0.5);
        assert_eq!(precision(&[1, 3], &truth), 0.5);
        assert_eq!(precision(&[], &truth), 1.0);
    }
}
