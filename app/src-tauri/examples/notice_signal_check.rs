//! Applies the operational-word check to notices earlier runs saved, without calling a model.
//!
//! Usage: cargo run --release --example notice_signal_check -- <repository root> <out json> <run dir>...
//!
//! A run dir holds either `lecture-contract.json` (the synthetic lecture; segments come from
//! the fixture) or `lecture-pipeline.json` (a recorded lecture; segments come from its
//! `transcript.txt`). For every draft notice it reports whether the cited segments carry an
//! operational word, and for every window whether the notices call would be skipped.
use std::collections::HashMap;
use std::path::PathBuf;

use app_lib::contract::{parse_transcript, Segment};
use app_lib::lecture::has_notice_signal;
use app_lib::lecture_fixture::Fixture;

fn text_of(segments: &[Segment], ids: &[String]) -> String {
    let texts: HashMap<&str, &str> = segments.iter().map(|segment| (segment.id.as_str(), segment.text.as_str())).collect();
    ids.iter().filter_map(|id| texts.get(id.as_str()).copied()).collect::<Vec<_>>().join("\n")
}

/// The segments from `first` to `last`, in transcript order.
fn between(segments: &[Segment], first: &str, last: &str) -> Vec<Segment> {
    let start = segments.iter().position(|segment| segment.id == first).unwrap_or(segments.len());
    let end = segments.iter().position(|segment| segment.id == last).unwrap_or(start);
    segments[start..=end.max(start).min(segments.len().saturating_sub(1))].to_vec()
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: notice_signal_check <repository root> <out json> <run dir>...".into());
    }
    let root = PathBuf::from(&arguments[0]);
    let fixture = Fixture::load(&root.join("evaluation/fixtures/lecture-synthetic-v1.json"))?;
    let fixture_segments = fixture.all_segments()?;

    let mut runs = Vec::new();
    for directory in &arguments[2..] {
        let directory = PathBuf::from(directory);
        let (report_path, segments) = if directory.join("lecture-contract.json").exists() {
            (directory.join("lecture-contract.json"), fixture_segments.clone())
        } else {
            let transcript = std::fs::read_to_string(directory.join("transcript.txt"))
                .map_err(|error| format!("{}: {error}", directory.display()))?;
            (directory.join("lecture-pipeline.json"), parse_transcript(&transcript)?)
        };
        let text = std::fs::read_to_string(&report_path).map_err(|error| format!("{}: {error}", report_path.display()))?;
        let report: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;

        let mut notices = Vec::new();
        let mut windows = 0;
        let mut skipped_windows = 0;
        for draft in report["drafts"].as_array().ok_or("drafts missing")? {
            let value = &draft["value"];
            let first = value["window"]["first"].as_str().unwrap_or_default();
            let last = value["window"]["last"].as_str().unwrap_or_default();
            let window = between(&segments, first, last);
            windows += 1;
            let window_text = window.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join("\n");
            if !has_notice_signal(&window_text) {
                skipped_windows += 1;
            }
            for notice in value["notices"].as_array().ok_or("notices missing")? {
                let refs: Vec<String> = notice["source_refs"]
                    .as_array()
                    .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let kept = has_notice_signal(&text_of(&segments, &refs));
                notices.push(serde_json::json!({
                    "run": draft["run"],
                    "window": draft["window"],
                    "kind": notice["kind"],
                    "status": notice["status"],
                    "content": notice["content"],
                    "source_refs": refs,
                    "kept": kept,
                }));
            }
        }
        let kept = notices.iter().filter(|notice| notice["kept"] == true).count();
        println!(
            "{}: {} notices, {kept} kept, {} dropped; notices call skipped in {skipped_windows} of {windows} windows",
            directory.display(),
            notices.len(),
            notices.len() - kept
        );
        for notice in notices.iter().filter(|notice| notice["kept"] == false) {
            println!("  dropped {} {} {}", notice["kind"], notice["source_refs"], notice["content"]);
        }
        runs.push(serde_json::json!({
            "dir": directory.display().to_string(),
            "notices": notices.len(),
            "kept": kept,
            "windows": windows,
            "skipped_windows": skipped_windows,
            "details": notices,
        }));
    }
    std::fs::write(&arguments[1], serde_json::to_string_pretty(&runs).unwrap_or_default()).map_err(|error| error.to_string())?;
    Ok(())
}
