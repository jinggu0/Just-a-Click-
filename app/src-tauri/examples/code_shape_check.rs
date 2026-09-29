//! Applies the code-shape check to code lists earlier runs saved, without calling a model.
//!
//! Usage: cargo run --release --example code_shape_check -- <out json> <run dir>...
//!
//! A run dir holds `lecture-contract.json` (synthetic lecture) or `lecture-pipeline.json`
//! (a recorded lecture). Every draft code item is checked; the report counts what stays and
//! what would be dropped, and for the synthetic lecture whether the commands it must keep
//! (`chmod 755 run.sh`, `gcc -o banker banker.c`, `ps aux`) all stay.
use std::collections::BTreeMap;
use std::path::PathBuf;

use app_lib::lecture::{code_problem, collapse, Code, Language};

const MUST_STAY: [&str; 3] = ["chmod 755 run.sh", "gcc -o banker banker.c", "ps aux"];

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 2 {
        return Err("usage: code_shape_check <out json> <run dir>...".into());
    }
    let mut runs = Vec::new();
    for directory in &arguments[1..] {
        let directory = PathBuf::from(directory);
        let path = ["lecture-contract.json", "lecture-pipeline.json"]
            .iter()
            .map(|name| directory.join(name))
            .find(|path| path.exists())
            .ok_or(format!("{}: no saved run", directory.display()))?;
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let report: serde_json::Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;

        let mut items = Vec::new();
        let mut reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut must_stay_lost = Vec::new();
        for draft in report["drafts"].as_array().ok_or("drafts missing")? {
            for code in draft["value"]["code"].as_array().ok_or("code missing")? {
                let item = Code {
                    code: code["code"].as_str().unwrap_or_default().to_string(),
                    language: Language::Other,
                    explanation: code["explanation"].as_str().unwrap_or_default().to_string(),
                    from_transcript: code["from_transcript"].as_bool().unwrap_or(false),
                    source_refs: code["source_refs"]
                        .as_array()
                        .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                };
                let problem = code_problem(&item);
                if let Some(kind) = problem {
                    *reasons.entry(kind).or_default() += 1;
                    if MUST_STAY.contains(&collapse(&item.code).as_str()) {
                        must_stay_lost.push(item.code.clone());
                    }
                }
                items.push(serde_json::json!({
                    "run": draft["run"],
                    "window": draft["window"],
                    "code": item.code,
                    "source_refs": item.source_refs,
                    "dropped": problem,
                }));
            }
        }
        let kept: Vec<String> = items
            .iter()
            .filter(|item| item["dropped"].is_null())
            .map(|item| item["code"].as_str().unwrap_or_default().to_string())
            .collect();
        println!(
            "{}: {} items, {} stay, dropped {:?}, must-stay lost {:?}",
            directory.display(),
            items.len(),
            kept.len(),
            reasons,
            must_stay_lost
        );
        let mut distinct: Vec<String> = kept.clone();
        distinct.sort();
        distinct.dedup();
        println!("  stay: {}", distinct.join(" | "));
        runs.push(serde_json::json!({
            "dir": directory.display().to_string(),
            "items": items.len(),
            "kept": kept.len(),
            "dropped": reasons,
            "must_stay_lost": must_stay_lost,
            "distinct_kept": distinct,
            "details": items,
        }));
    }
    std::fs::write(&arguments[0], serde_json::to_string_pretty(&runs).unwrap_or_default()).map_err(|error| error.to_string())?;
    Ok(())
}
