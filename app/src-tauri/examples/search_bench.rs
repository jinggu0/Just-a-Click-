//! Builds a lecture-sized Korean corpus, indexes it three ways and measures each.
//!
//! Usage: cargo run --release --example search_bench -- <fleurs test.tsv> <out dir> [lectures]
//!
//! The corpus reuses public FLEURS sentences, so it is not real lecture speech. It is only
//! used to compare methods on the same text.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use app_lib::search::{precision, recall, search, search_prefix, truth};
use app_lib::store::{create_schema, insert_segments, open, rebuild_index, Method, Segment, SEGMENT_CHARACTERS};

const LECTURE_CHARACTERS: usize = 40_000;
const LIMIT: usize = 20;
const REPEATS: usize = 10;
const PARTICLES: [char; 8] = ['은', '는', '이', '가', '을', '를', '의', '에'];

/// A fixed sequence so a rerun builds the same corpus.
struct Picker(u64);

impl Picker {
    fn next(&mut self, limit: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize % limit
    }
}

fn sentences(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("fleurs tsv")
        .lines()
        .filter_map(|line| line.split('\t').nth(2).map(str::to_string))
        .filter(|text| text.chars().count() > 20)
        .collect()
}

fn corpus(pool: &[String], lectures: usize) -> (Vec<Segment>, Vec<String>) {
    let mut picker = Picker(20_260_925);
    let mut rows = Vec::new();
    let mut terms = Vec::new();
    for lecture in 0..lectures {
        let term = format!("제{lecture}장특강용어");
        let mut text = format!("{term}에 대한 강의입니다. ");
        while text.chars().count() < LECTURE_CHARACTERS {
            text.push_str(&pool[picker.next(pool.len())]);
            text.push(' ');
        }
        let characters: Vec<char> = text.chars().collect();
        for chunk in characters.chunks(SEGMENT_CHARACTERS) {
            rows.push(Segment { lecture: lecture as i64, body: chunk.iter().collect() });
        }
        terms.push(term);
    }
    (rows, terms)
}

fn hangul(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|character| ('가'..='힣').contains(&character))
}

/// Five query kinds taken from the corpus itself, four of each where possible.
fn queries(rows: &[Segment], terms: &[String]) -> Vec<(String, String)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        for word in row.body.split_whitespace() {
            let word: String = word
                .chars()
                .filter(|character| ('가'..='힣').contains(character) || character.is_ascii_alphanumeric())
                .collect();
            if !word.is_empty() {
                *counts.entry(word).or_default() += 1;
            }
        }
    }
    let mut words: Vec<(&String, &usize)> = counts.iter().collect();
    words.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    let mut picked: Vec<(String, String)> = Vec::new();
    let count = |picked: &Vec<(String, String)>, kind: &str| {
        picked.iter().filter(|(existing, _)| existing == kind).count()
    };
    for (word, _) in &words {
        let characters: Vec<char> = word.chars().collect();
        if count(&picked, "어간+조사") < 4
            && characters.len() >= 4
            && hangul(word)
            && PARTICLES.contains(&characters[characters.len() - 1])
        {
            picked.push(("어간+조사".into(), characters[..characters.len() - 1].iter().collect()));
        } else if count(&picked, "어중") < 4 && characters.len() >= 5 && hangul(word) {
            picked.push(("어중".into(), characters[1..4].iter().collect()));
        } else if count(&picked, "두 글자") < 4 && characters.len() == 2 && hangul(word) {
            picked.push(("두 글자".into(), word.to_string()));
        } else if count(&picked, "영문") < 2
            && characters.len() >= 3
            && word.chars().all(|character| character.is_ascii_alphanumeric())
        {
            picked.push(("영문".into(), word.to_string()));
        }
    }
    for term in terms.iter().take(2) {
        picked.push(("고유 용어".into(), term.clone()));
    }
    picked
}

fn milliseconds(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 2 {
        return Err("usage: search_bench <fleurs test.tsv> <out dir> [lectures]".into());
    }
    let pool = sentences(Path::new(&arguments[0]));
    let out_dir = PathBuf::from(&arguments[1]);
    let lectures: usize = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(100);
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    let (rows, terms) = corpus(&pool, lectures);
    let characters: usize = rows.iter().map(|row| row.body.chars().count()).sum();
    println!("sentences {} segments {} characters {}", pool.len(), rows.len(), characters);

    let set = queries(&rows, &terms);
    let mut indexes = serde_json::Map::new();
    let mut results = Vec::new();
    for method in Method::all() {
        let path = out_dir.join(format!("corpus-{}.db", method.label()));
        let _ = std::fs::remove_file(&path);
        let connection = open(&path)?;
        create_schema(&connection, method)?;
        let started = Instant::now();
        insert_segments(&connection, &rows)?;
        rebuild_index(&connection, method)?;
        let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        drop(connection);
        let bytes = std::fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
        indexes.insert(
            method.label().to_string(),
            serde_json::json!({"index_seconds": seconds, "file_mib": (bytes as f64 / 1_048_576.0 * 10.0).round() / 10.0}),
        );
        println!("{} index {seconds:.1}s file {:.1} MiB", method.label(), bytes as f64 / 1_048_576.0);

        let connection = open(&path)?;
        for (kind, query) in &set {
            let expected = truth(&rows, query);
            let mut names: Vec<(&str, Vec<i64>, Vec<f64>)> = Vec::new();
            let mut found = Vec::new();
            let mut times = Vec::new();
            for _ in 0..REPEATS {
                let started = Instant::now();
                found = search(&connection, method, query, LIMIT)?;
                times.push(milliseconds(started));
            }
            names.push((method.label(), found, times));
            if method == Method::Unicode61 {
                let mut prefix_found = Vec::new();
                let mut prefix_times = Vec::new();
                for _ in 0..REPEATS {
                    let started = Instant::now();
                    prefix_found = search_prefix(&connection, query, LIMIT)?;
                    prefix_times.push(milliseconds(started));
                }
                names.push(("unicode61-prefix", prefix_found, prefix_times));
            }
            for (label, found, mut times) in names {
                times.sort_by(|left, right| left.partial_cmp(right).unwrap());
                results.push(serde_json::json!({
                    "kind": kind,
                    "query": query,
                    "method": label,
                    "truth": expected.len(),
                    "found": found.len(),
                    "recall": (recall(&found, &expected, LIMIT) * 1000.0).round() / 1000.0,
                    "precision": (precision(&found, &expected) * 1000.0).round() / 1000.0,
                    "median_ms": times[times.len() / 2],
                    "max_ms": times[times.len() - 1],
                }));
            }
        }
    }

    let report = serde_json::json!({
        "lectures": lectures,
        "segments": rows.len(),
        "characters": characters,
        "limit": LIMIT,
        "repeats": REPEATS,
        "indexes": indexes,
        "queries": results,
    });
    let path = out_dir.join("search-bench.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    println!("written to {}", path.display());
    Ok(())
}
