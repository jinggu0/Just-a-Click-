//! Runs the lecture contracts against the synthetic lecture with the real model.
//!
//! Usage: cargo run --release --example lecture_contract_check -- <repository root> <out dir> [runs]
//!
//! One run makes a draft for each five-minute window, then a note from those drafts (the
//! five-minute note) and a note from the whole transcript (the full pass). A rejected answer
//! is retried once with its violations attached, and the retry counts towards the time.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use app_lib::contract::{generation_schema, Segment, Violation, DRAFT_SCHEMA, NOTE_SCHEMA};
use app_lib::lecture::{
    draft_prompt, note_prompt, validate_draft, validate_note, Accepted, Draft, Repair, DRAFT_PROMPT_VERSION,
    NOTE_PROMPT_VERSION,
};
use app_lib::lecture_fixture::{check_draft, check_note, Expectation, Fixture};
use app_lib::llm::{complete_json, stream_draft, Completion, Server, ServerSettings};
use app_lib::power::KeepAwake;
use app_lib::process::ProcessGroup;
use serde::Serialize;

/// Generous limits, so the measurement shows how long answers really are instead of
/// cutting them off. Decision 0009 assumed 300 tokens per draft.
const DRAFT_MAX_TOKENS: u32 = 2_048;
const NOTE_MAX_TOKENS: u32 = 4_096;
const ASSUMED_DRAFT_TOKENS: u64 = 300;

#[derive(Serialize)]
struct Attempt {
    seconds: f64,
    prompt_tokens: u64,
    completion_tokens: u64,
    finish_reason: String,
    violations: Vec<Violation>,
    repairs: Vec<Repair>,
}

#[derive(Serialize)]
struct Call {
    stage: &'static str,
    run: usize,
    window: Option<usize>,
    accepted: bool,
    seconds: f64,
    attempts: Vec<Attempt>,
    expectations: Vec<Expectation>,
}

struct Endpoint {
    base: String,
    key: String,
    raw_dir: PathBuf,
}

impl Endpoint {
    /// Asks once, and once more with the violations if the first answer is refused.
    fn ask<T>(
        &self,
        label: &str,
        system: &str,
        user: &str,
        schema: &serde_json::Value,
        max_tokens: u32,
        validate: impl Fn(&Completion) -> Result<Accepted<T>, Vec<Violation>>,
    ) -> Result<(Option<T>, Vec<Attempt>), String> {
        let mut attempts = Vec::new();
        let mut request = user.to_string();
        for attempt in 1..=2 {
            let completion = complete_json(&self.base, &self.key, system, &request, schema, max_tokens)?;
            std::fs::write(self.raw_dir.join(format!("{label}-{attempt}.json")), &completion.content)
                .map_err(|error| error.to_string())?;
            let outcome = validate(&completion);
            let violations = outcome.as_ref().err().cloned().unwrap_or_default();
            let repairs = outcome.as_ref().map(|accepted| accepted.repairs.clone()).unwrap_or_default();
            attempts.push(Attempt {
                seconds: round(completion.seconds, 2),
                prompt_tokens: completion.prompt_tokens,
                completion_tokens: completion.completion_tokens,
                finish_reason: completion.finish_reason.clone(),
                violations: violations.clone(),
                repairs,
            });
            match outcome {
                Ok(accepted) => return Ok((Some(accepted.value), attempts)),
                Err(_) if attempt == 1 => {
                    let listed: Vec<String> = violations
                        .iter()
                        .map(|violation| format!("- {} {}: {}", violation.path, violation.rule, violation.detail))
                        .collect();
                    request = format!(
                        "{user}\n\n이전 답이 다음 규칙을 어겼다. 규칙을 지켜 처음부터 다시 쓴다.\n{}",
                        listed.join("\n")
                    );
                }
                Err(_) => {}
            }
        }
        Ok((None, attempts))
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

fn ids(segments: &[Segment]) -> Vec<String> {
    segments.iter().map(|segment| segment.id.clone()).collect()
}

fn lines(segments: &[Segment]) -> String {
    segments.iter().map(Segment::line).collect::<Vec<_>>().join("\n")
}

fn call(stage: &'static str, run: usize, window: Option<usize>, attempts: Vec<Attempt>, accepted: bool,
        expectations: Vec<Expectation>) -> Call {
    let seconds = round(attempts.iter().map(|attempt| attempt.seconds).sum(), 2);
    Call { stage, run, window, accepted, seconds, attempts, expectations }
}

/// The drafts' own sources, so the note can quote the transcript without seeing all of it.
fn cited_segments(drafts: &[Draft], all: &[Segment]) -> Vec<Segment> {
    let mut cited = std::collections::BTreeSet::new();
    for draft in drafts {
        let lists = draft
            .points
            .iter()
            .chain(&draft.examples)
            .map(|item| &item.source_refs)
            .chain(draft.concepts.iter().map(|concept| &concept.source_refs))
            .chain(draft.code.iter().map(|item| &item.source_refs))
            .chain(draft.notices.iter().map(|notice| &notice.source_refs));
        for list in lists {
            cited.extend(list.iter().cloned());
        }
    }
    all.iter().filter(|segment| cited.contains(&segment.id)).cloned().collect()
}

fn summarize(calls: &[Call]) -> serde_json::Value {
    let mut stages = serde_json::Map::new();
    for stage in ["draft", "note_from_drafts", "note_from_transcript"] {
        let selected: Vec<&Call> = calls.iter().filter(|call| call.stage == stage).collect();
        if selected.is_empty() {
            continue;
        }
        let first_valid = selected.iter().filter(|call| call.attempts[0].violations.is_empty()).count();
        let first_clean = selected
            .iter()
            .filter(|call| call.attempts[0].violations.is_empty() && call.attempts[0].repairs.is_empty())
            .count();
        let mut repairs: BTreeMap<&str, usize> = BTreeMap::new();
        for call in &selected {
            for attempt in &call.attempts {
                for repair in &attempt.repairs {
                    *repairs.entry(repair.kind).or_default() += 1;
                }
            }
        }
        let accepted = selected.iter().filter(|call| call.accepted).count();
        let mut seconds: Vec<f64> = selected.iter().map(|call| call.seconds).collect();
        seconds.sort_by(|left, right| left.partial_cmp(right).unwrap());
        let tokens = selected
            .iter()
            .flat_map(|call| call.attempts.iter().map(|attempt| attempt.completion_tokens))
            .max()
            .unwrap_or(0);
        let mut rules: BTreeMap<&str, usize> = BTreeMap::new();
        for call in &selected {
            for violation in &call.attempts[0].violations {
                *rules.entry(violation.rule).or_default() += 1;
            }
        }
        stages.insert(stage.to_string(), serde_json::json!({
            "calls": selected.len(),
            "first_attempt_clean": first_clean,
            "first_attempt_valid": first_valid,
            "repairs_in_accepted_answers": repairs,
            "accepted": accepted,
            "median_seconds": seconds[seconds.len() / 2],
            "max_seconds": seconds[seconds.len() - 1],
            "max_completion_tokens": tokens,
            "first_attempt_violations": rules,
        }));
    }
    let mut expectations: BTreeMap<&str, (bool, usize, usize)> = BTreeMap::new();
    for call in calls {
        for expectation in &call.expectations {
            let entry = expectations.entry(expectation.name).or_insert((expectation.gating, 0, 0));
            if let Some(passed) = expectation.passed {
                entry.2 += 1;
                if passed {
                    entry.1 += 1;
                }
            }
        }
    }
    let expectations: serde_json::Map<String, serde_json::Value> = expectations
        .into_iter()
        .map(|(name, (gating, passed, applicable))| {
            (name.to_string(), serde_json::json!({"gating": gating, "passed": passed, "applicable": applicable}))
        })
        .collect();
    let max_of = |stage: &str| {
        calls.iter().filter(|call| call.stage == stage).map(|call| call.seconds).fold(0.0, f64::max)
    };
    let draft_tokens = calls
        .iter()
        .filter(|call| call.stage == "draft")
        .flat_map(|call| call.attempts.iter().map(|attempt| attempt.completion_tokens))
        .max()
        .unwrap_or(0);
    serde_json::json!({
        "stages": stages,
        "expectations": expectations,
        "all_accepted": calls.iter().all(|call| call.accepted),
        "gating_all_passed": calls.iter().all(|call| call.expectations.iter()
            .all(|expectation| !expectation.gating || expectation.passed != Some(false))),
        "draft_tokens_vs_assumption": {"max": draft_tokens, "assumed": ASSUMED_DRAFT_TOKENS},
        "post_recording_estimate_seconds": round(2.0 * max_of("draft") + max_of("note_from_drafts"), 1),
    })
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 2 {
        return Err("usage: lecture_contract_check <repository root> <out dir> [runs]".into());
    }
    let root = PathBuf::from(&arguments[0]);
    let out_dir = PathBuf::from(&arguments[1]);
    let runs: usize = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(5);
    let raw_dir = out_dir.join("raw");
    std::fs::create_dir_all(&raw_dir).map_err(|error| error.to_string())?;

    let fixture = Fixture::load(&root.join("evaluation/fixtures/lecture-synthetic-v1.json"))?;
    let all = fixture.all_segments()?;
    let windows: Vec<Vec<Segment>> = (0..fixture.windows.len())
        .map(|index| fixture.window(index))
        .collect::<Result<_, _>>()?;

    // A run takes about an hour; standby would stall it and distort the timings.
    let _awake = KeepAwake::new();
    let group = ProcessGroup::new()?;
    let settings = ServerSettings {
        executable: root.join("runtimes/b10994/vulkan/llama-server.exe"),
        model: root.join("models/Qwen3-8B-Q5_K_M.gguf"),
        log: out_dir.join("server.log"),
        context_tokens: 8192,
        threads: 2,
    };
    let mut server = Server::start(&group, &settings)?;
    println!("server ready in {:.1}s", server.ready_seconds);
    stream_draft(&server.base(), server.key(), "안녕하세요.", 8, &AtomicBool::new(false))?;
    let endpoint = Endpoint { base: server.base(), key: server.key().to_string(), raw_dir };
    let (draft_system, note_system) = (draft_prompt(), note_prompt());
    let prompts = serde_json::json!({
        DRAFT_PROMPT_VERSION: draft_system,
        NOTE_PROMPT_VERSION: note_system,
    });
    std::fs::write(out_dir.join("prompts.json"), serde_json::to_string_pretty(&prompts).unwrap_or_default())
        .map_err(|error| error.to_string())?;

    let mut calls = Vec::new();
    for run in 1..=runs {
        let mut drafts = Vec::new();
        for (index, window) in windows.iter().enumerate() {
            let schema = generation_schema(DRAFT_SCHEMA, &ids(window))?;
            let user = format!("과목: {}\n\n전사:\n{}", fixture.course, lines(window));
            let (draft, attempts) = endpoint.ask(
                &format!("run{run}-draft{}", index + 1),
                &draft_system,
                &user,
                &schema,
                DRAFT_MAX_TOKENS,
                |completion| validate_draft(&completion.content, &completion.finish_reason, window),
            )?;
            let expectations = draft
                .as_ref()
                .map(|draft| check_draft(draft, &fixture.traps, window))
                .unwrap_or_default();
            println!("run {run} draft {} accepted {} in {:.1}s", index + 1, draft.is_some(),
                     attempts.iter().map(|attempt| attempt.seconds).sum::<f64>());
            calls.push(call("draft", run, Some(index + 1), attempts, draft.is_some(), expectations));
            drafts.extend(draft);
        }

        let cited = cited_segments(&drafts, &all);
        let drafts_json: Vec<String> = drafts
            .iter()
            .map(|draft| serde_json::to_string(draft).unwrap_or_default())
            .collect();
        let user = format!(
            "과목: {}\n\n구간 초안:\n{}\n\n초안이 인용한 전사 구간:\n{}",
            fixture.course,
            drafts_json.join("\n"),
            lines(&cited)
        );
        let schema = generation_schema(NOTE_SCHEMA, &ids(&cited))?;
        let (note, attempts) = endpoint.ask(
            &format!("run{run}-note-drafts"),
            &note_system,
            &user,
            &schema,
            NOTE_MAX_TOKENS,
            |completion| validate_note(&completion.content, &completion.finish_reason, &all),
        )?;
        let expectations = note.as_ref().map(|note| check_note(note, &fixture.traps)).unwrap_or_default();
        println!("run {run} note from drafts accepted {}", note.is_some());
        calls.push(call("note_from_drafts", run, None, attempts, note.is_some(), expectations));

        let user = format!("과목: {}\n\n전사:\n{}", fixture.course, lines(&all));
        let schema = generation_schema(NOTE_SCHEMA, &ids(&all))?;
        let (note, attempts) = endpoint.ask(
            &format!("run{run}-note-transcript"),
            &note_system,
            &user,
            &schema,
            NOTE_MAX_TOKENS,
            |completion| validate_note(&completion.content, &completion.finish_reason, &all),
        )?;
        let expectations = note.as_ref().map(|note| check_note(note, &fixture.traps)).unwrap_or_default();
        println!("run {run} note from transcript accepted {}", note.is_some());
        calls.push(call("note_from_transcript", run, None, attempts, note.is_some(), expectations));
    }
    server.stop()?;

    let report = serde_json::json!({
        "fixture": fixture.fixture_id,
        "runs": runs,
        "prompts": {"draft": DRAFT_PROMPT_VERSION, "note": NOTE_PROMPT_VERSION},
        "max_tokens": {"draft": DRAFT_MAX_TOKENS, "note": NOTE_MAX_TOKENS},
        "summary": summarize(&calls),
        "calls": calls,
    });
    let path = out_dir.join("lecture-contract.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report["summary"]).unwrap_or_default());
    println!("written to {}", path.display());
    Ok(())
}

