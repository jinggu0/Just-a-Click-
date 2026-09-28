//! Runs the split lecture contracts against the synthetic lecture with the real model.
//!
//! Usage: cargo run --release --example lecture_contract_check -- <repository root> <out dir> [runs]
//!
//! One run asks three times per five-minute window (points, notices, code) and assembles a
//! draft, then writes two notes. The five-minute note's body is written from the drafts'
//! points. The precise note is read one window at a time from the transcript, merged by the
//! app and finished by one call for topic and review. Notices and code in both notes come
//! from the drafts. A refused answer is retried once with its violations attached, and the
//! retry counts towards the time.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use app_lib::contract::{
    generation_schema, Segment, Violation, CODE_SCHEMA, NOTE_BODY_SCHEMA, NOTICES_SCHEMA, POINTS_SCHEMA,
    PRECISE_SYNTHESIS_SCHEMA, PRECISE_WINDOW_SCHEMA,
};
use app_lib::lecture::{
    code_prompt, note_body_prompt, notices_prompt, points_prompt, precise_synthesis_prompt, precise_window_prompt,
    validate_code, validate_note_body, validate_notices, validate_points, validate_precise_synthesis,
    validate_precise_window, Accepted, PreciseWindow, Repair, CODE_PROMPT_VERSION, NOTE_BODY_PROMPT_VERSION,
    NOTICES_PROMPT_VERSION, POINTS_PROMPT_VERSION, PRECISE_SYNTHESIS_PROMPT_VERSION, PRECISE_WINDOW_PROMPT_VERSION,
    PRECISE_WINDOW_VERSION,
};
use app_lib::lecture_fixture::{check_draft, check_note, Expectation, Fixture};
use app_lib::lecture_merge::{
    assemble_draft, assemble_note, assemble_precise_note, cited_by, merge_precise, name_key, synthesis_input, Draft,
    Note,
};
use app_lib::llm::{complete_json, stream_draft, Completion, Server, ServerSettings};
use app_lib::power::KeepAwake;
use app_lib::process::ProcessGroup;
use serde::Serialize;

/// Generous limits, so the measurement shows how long answers really are.
const WINDOW_MAX_TOKENS: u32 = 2_048;
const NOTE_MAX_TOKENS: u32 = 4_096;
/// Decision 0009: a window's draft must be ready within 150 s; after recording, the note
/// must be saved within 300 s.
const WINDOW_LIMIT_SECONDS: f64 = 150.0;
const AFTER_RECORDING_LIMIT_SECONDS: f64 = 300.0;

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
}

#[derive(Serialize)]
struct Assembled<T: Serialize> {
    kind: &'static str,
    run: usize,
    window: Option<usize>,
    complete: bool,
    seconds: f64,
    repairs: Vec<Repair>,
    expectations: Vec<Expectation>,
    value: T,
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
    ) -> Result<(Option<Accepted<T>>, Vec<Attempt>), String> {
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
                Ok(accepted) => return Ok((Some(accepted), attempts)),
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

/// A call that was refused twice contributes nothing to its draft.
fn empty<T>() -> Accepted<Vec<T>> {
    Accepted { value: Vec::new(), repairs: Vec::new() }
}

/// A precise window refused twice adds nothing, but keeps its place in the numbering.
fn empty_window() -> Accepted<PreciseWindow> {
    Accepted {
        value: PreciseWindow {
            schema_version: PRECISE_WINDOW_VERSION.into(),
            concepts: Vec::new(),
            examples: Vec::new(),
            terms: Vec::new(),
        },
        repairs: Vec::new(),
    }
}

/// Whether each precise note reaches every window, how many names repeat after merging, and
/// how many concepts each note has.
fn precise_summary(notes: &[Assembled<Note>], windows: &[Vec<Segment>]) -> serde_json::Value {
    let precise: Vec<&Assembled<Note>> = notes.iter().filter(|note| note.kind == "note_precise").collect();
    let covers = |note: &Note, window: &[Segment]| {
        note.concepts
            .iter()
            .flat_map(|concept| &concept.source_refs)
            .any(|id| window.iter().any(|segment| &segment.id == id))
    };
    let cover_every_window = precise
        .iter()
        .filter(|note| windows.iter().all(|window| covers(&note.value, window)))
        .count();
    let duplicate_names: usize = precise
        .iter()
        .map(|note| {
            let mut seen = BTreeSet::new();
            note.value
                .concepts
                .iter()
                .map(|concept| format!("concept:{}", name_key(&concept.name)))
                .chain(note.value.terms.iter().map(|term| format!("term:{}", name_key(&term.term_ko))))
                .filter(|key| !seen.insert(key.clone()))
                .count()
        })
        .sum();
    let concept_counts: Vec<usize> = precise.iter().map(|note| note.value.concepts.len()).collect();
    serde_json::json!({
        "notes": precise.len(),
        "cover_every_window": cover_every_window,
        "duplicate_names": duplicate_names,
        "concept_counts": concept_counts,
    })
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

fn call(stage: &'static str, run: usize, window: Option<usize>, attempts: Vec<Attempt>, accepted: bool) -> Call {
    let seconds = round(attempts.iter().map(|attempt| attempt.seconds).sum(), 2);
    Call { stage, run, window, accepted, seconds, attempts }
}

/// The segments the drafts' points cite, so the note can quote the transcript.
fn cited_segments(drafts: &[Draft], all: &[Segment]) -> Vec<Segment> {
    let cited: BTreeSet<&String> = drafts
        .iter()
        .flat_map(|draft| draft.points.iter().flat_map(|item| &item.source_refs))
        .collect();
    all.iter().filter(|segment| cited.contains(&segment.id)).cloned().collect()
}

fn count_expectations<'a>(
    table: &mut BTreeMap<&'static str, (bool, usize, usize)>,
    expectations: impl Iterator<Item = &'a Expectation>,
) {
    for expectation in expectations {
        let entry = table.entry(expectation.name).or_insert((expectation.gating, 0, 0));
        entry.0 |= expectation.gating;
        if let Some(passed) = expectation.passed {
            entry.2 += 1;
            if passed {
                entry.1 += 1;
            }
        }
    }
}

fn stage_summary(calls: &[Call], stage: &str) -> Option<serde_json::Value> {
    let selected: Vec<&Call> = calls.iter().filter(|call| call.stage == stage).collect();
    if selected.is_empty() {
        return None;
    }
    let first_clean = selected
        .iter()
        .filter(|call| call.attempts[0].violations.is_empty() && call.attempts[0].repairs.is_empty())
        .count();
    let first_valid = selected.iter().filter(|call| call.attempts[0].violations.is_empty()).count();
    let accepted = selected.iter().filter(|call| call.accepted).count();
    let mut seconds: Vec<f64> = selected.iter().map(|call| call.seconds).collect();
    seconds.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let mut tokens: Vec<u64> = selected
        .iter()
        .flat_map(|call| call.attempts.iter().map(|attempt| attempt.completion_tokens))
        .collect();
    tokens.sort();
    let mut repairs: BTreeMap<&str, usize> = BTreeMap::new();
    let mut violations: BTreeMap<&str, usize> = BTreeMap::new();
    for call in &selected {
        for attempt in &call.attempts {
            for repair in &attempt.repairs {
                *repairs.entry(repair.kind).or_default() += 1;
            }
        }
        for violation in &call.attempts[0].violations {
            *violations.entry(violation.rule).or_default() += 1;
        }
    }
    Some(serde_json::json!({
        "calls": selected.len(),
        "first_attempt_clean": first_clean,
        "first_attempt_valid": first_valid,
        "accepted": accepted,
        "median_seconds": seconds[seconds.len() / 2],
        "max_seconds": seconds[seconds.len() - 1],
        "median_completion_tokens": tokens[tokens.len() / 2],
        "max_completion_tokens": tokens[tokens.len() - 1],
        "repairs": repairs,
        "first_attempt_violations": violations,
    }))
}

fn summarize(
    calls: &[Call],
    drafts: &[Assembled<Draft>],
    notes: &[Assembled<Note>],
    runs: usize,
    windows: &[Vec<Segment>],
) -> serde_json::Value {
    let mut stages = serde_json::Map::new();
    for stage in ["points", "notices", "code", "note_body_from_drafts", "precise_window", "precise_synthesis"] {
        if let Some(summary) = stage_summary(calls, stage) {
            stages.insert(stage.to_string(), summary);
        }
    }
    let mut merges: BTreeMap<&str, usize> = BTreeMap::new();
    for repair in drafts.iter().flat_map(|draft| &draft.repairs).chain(notes.iter().flat_map(|note| &note.repairs)) {
        if repair.kind.ends_with("_merged") {
            *merges.entry(repair.kind).or_default() += 1;
        }
    }
    let mut table = BTreeMap::new();
    count_expectations(&mut table, drafts.iter().flat_map(|draft| &draft.expectations));
    count_expectations(&mut table, notes.iter().flat_map(|note| &note.expectations));
    let expectations: serde_json::Map<String, serde_json::Value> = table
        .into_iter()
        .map(|(name, (gating, passed, applicable))| {
            (name.to_string(), serde_json::json!({"gating": gating, "passed": passed, "applicable": applicable}))
        })
        .collect();
    let gating_all_passed = drafts
        .iter()
        .flat_map(|draft| &draft.expectations)
        .chain(notes.iter().flat_map(|note| &note.expectations))
        .all(|expectation| !expectation.gating || expectation.passed != Some(false));
    let per_run: Vec<serde_json::Value> = (1..=runs)
        .map(|run| {
            let window = drafts
                .iter()
                .filter(|draft| draft.run == run)
                .map(|draft| draft.seconds)
                .fold(0.0, f64::max);
            let note = notes
                .iter()
                .filter(|note| note.run == run && note.kind == "note_from_drafts")
                .map(|note| note.seconds)
                .fold(0.0, f64::max);
            let precise = notes
                .iter()
                .filter(|note| note.run == run && note.kind == "note_precise")
                .map(|note| note.seconds)
                .fold(0.0, f64::max);
            serde_json::json!({
                "run": run,
                "max_window_seconds": round(window, 1),
                "note_body_from_drafts_seconds": round(note, 1),
                "after_recording_estimate_seconds": round(2.0 * window + note, 1),
                "precise_seconds": round(precise, 1),
            })
        })
        .collect();
    let windows_within = drafts.iter().filter(|draft| draft.seconds <= WINDOW_LIMIT_SECONDS).count();
    let runs_within = per_run
        .iter()
        .filter(|run| {
            run["after_recording_estimate_seconds"].as_f64().unwrap_or(f64::MAX) <= AFTER_RECORDING_LIMIT_SECONDS
        })
        .count();
    let notes_missing = runs * 2 - notes.len();
    serde_json::json!({
        "stages": stages,
        "assembly_merges": merges,
        "expectations": expectations,
        "all_accepted": calls.iter().all(|call| call.accepted),
        "notes_missing": notes_missing,
        "gating_all_passed": gating_all_passed,
        "windows_within_limit": {"within": windows_within, "windows": drafts.len(), "limit_seconds": WINDOW_LIMIT_SECONDS},
        "runs_within_after_recording_limit": {"within": runs_within, "runs": runs, "limit_seconds": AFTER_RECORDING_LIMIT_SECONDS},
        "precise": precise_summary(notes, windows),
        "per_run": per_run,
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
    let prompts = [
        (POINTS_PROMPT_VERSION, points_prompt()),
        (NOTICES_PROMPT_VERSION, notices_prompt()),
        (CODE_PROMPT_VERSION, code_prompt()),
        (NOTE_BODY_PROMPT_VERSION, note_body_prompt()),
        (PRECISE_WINDOW_PROMPT_VERSION, precise_window_prompt()),
        (PRECISE_SYNTHESIS_PROMPT_VERSION, precise_synthesis_prompt()),
    ];
    let prompt_map: serde_json::Map<String, serde_json::Value> = prompts
        .iter()
        .map(|(version, text)| (version.to_string(), serde_json::json!(text)))
        .collect();
    std::fs::write(out_dir.join("prompts.json"), serde_json::to_string_pretty(&prompt_map).unwrap_or_default())
        .map_err(|error| error.to_string())?;

    let mut calls = Vec::new();
    let mut drafts_record = Vec::new();
    let mut notes_record = Vec::new();
    for run in 1..=runs {
        let mut drafts = Vec::new();
        for (index, window) in windows.iter().enumerate() {
            let label = format!("run{run}-window{}", index + 1);
            let user = format!("과목: {}\n\n전사:\n{}", fixture.course, lines(window));
            let window_ids = ids(window);
            let (points, attempts) = endpoint.ask(
                &format!("{label}-points"),
                &prompts[0].1,
                &user,
                &generation_schema(POINTS_SCHEMA, &window_ids)?,
                WINDOW_MAX_TOKENS,
                |completion| validate_points(&completion.content, &completion.finish_reason, window),
            )?;
            calls.push(call("points", run, Some(index + 1), attempts, points.is_some()));
            let (notices, attempts) = endpoint.ask(
                &format!("{label}-notices"),
                &prompts[1].1,
                &user,
                &generation_schema(NOTICES_SCHEMA, &window_ids)?,
                WINDOW_MAX_TOKENS,
                |completion| validate_notices(&completion.content, &completion.finish_reason, window),
            )?;
            calls.push(call("notices", run, Some(index + 1), attempts, notices.is_some()));
            let (code, attempts) = endpoint.ask(
                &format!("{label}-code"),
                &prompts[2].1,
                &user,
                &generation_schema(CODE_SCHEMA, &window_ids)?,
                WINDOW_MAX_TOKENS,
                |completion| validate_code(&completion.content, &completion.finish_reason, window),
            )?;
            calls.push(call("code", run, Some(index + 1), attempts, code.is_some()));

            let seconds: f64 = calls[calls.len() - 3..].iter().map(|call| call.seconds).sum();
            let complete = points.is_some() && notices.is_some() && code.is_some();
            let draft = assemble_draft(
                window,
                points.unwrap_or_else(empty),
                notices.unwrap_or_else(empty),
                code.unwrap_or_else(empty),
            );
            let expectations = check_draft(&draft.value, &fixture.traps, window);
            println!("run {run} window {} complete {complete} in {seconds:.1}s", index + 1);
            drafts.push(draft.value.clone());
            drafts_record.push(Assembled {
                kind: "draft",
                run,
                window: Some(index + 1),
                complete,
                seconds: round(seconds, 2),
                repairs: draft.repairs,
                expectations,
                value: draft.value,
            });
        }

        let cited = cited_segments(&drafts, &all);
        let points_json: Vec<String> = drafts
            .iter()
            .map(|draft| serde_json::json!({"window": draft.window, "points": draft.points}).to_string())
            .collect();
        let user = format!(
            "과목: {}\n\n구간 요점:\n{}\n\n요점이 인용한 전사 구간:\n{}",
            fixture.course,
            points_json.join("\n"),
            lines(&cited)
        );
        let (body, attempts) = endpoint.ask(
            &format!("run{run}-note_from_drafts"),
            &prompts[3].1,
            &user,
            &generation_schema(NOTE_BODY_SCHEMA, &ids(&cited))?,
            NOTE_MAX_TOKENS,
            |completion| validate_note_body(&completion.content, &completion.finish_reason, &all),
        )?;
        let accepted = body.is_some();
        let record = call("note_body_from_drafts", run, None, attempts, accepted);
        let seconds = record.seconds;
        calls.push(record);
        println!("run {run} note_from_drafts accepted {accepted} in {seconds:.1}s");
        if let Some(body) = body {
            let note = assemble_note(body, &drafts);
            let expectations = check_note(&note.value, &fixture.traps);
            notes_record.push(Assembled {
                kind: "note_from_drafts",
                run,
                window: None,
                complete: true,
                seconds,
                repairs: note.repairs,
                expectations,
                value: note.value,
            });
        }

        // The precise note: each window from its own transcript, merged by the app, then one
        // call for topic and review over the merged concepts.
        let mut precise_windows = Vec::new();
        let mut precise_seconds = 0.0;
        let mut precise_complete = true;
        for (index, window) in windows.iter().enumerate() {
            let (answer, attempts) = endpoint.ask(
                &format!("run{run}-precise-window{}", index + 1),
                &prompts[4].1,
                &format!("과목: {}\n\n전사:\n{}", fixture.course, lines(window)),
                &generation_schema(PRECISE_WINDOW_SCHEMA, &ids(window))?,
                WINDOW_MAX_TOKENS,
                |completion| validate_precise_window(&completion.content, &completion.finish_reason, window),
            )?;
            let record = call("precise_window", run, Some(index + 1), attempts, answer.is_some());
            precise_seconds += record.seconds;
            precise_complete &= record.accepted;
            println!("run {run} precise window {} accepted {} in {:.1}s", index + 1, record.accepted, record.seconds);
            calls.push(record);
            precise_windows.push(answer.unwrap_or_else(empty_window));
        }
        let body = merge_precise(precise_windows, &all);
        let merged_cited = cited_by(&body.value, &all);
        if merged_cited.is_empty() {
            println!("run {run} note_precise skipped: no window cited anything");
            continue;
        }
        let (synthesis, attempts) = endpoint.ask(
            &format!("run{run}-precise-synthesis"),
            &prompts[5].1,
            &format!("과목: {}\n\n개념 목록:\n{}", fixture.course, synthesis_input(&body.value)),
            &generation_schema(PRECISE_SYNTHESIS_SCHEMA, &ids(&merged_cited))?,
            WINDOW_MAX_TOKENS,
            |completion| validate_precise_synthesis(&completion.content, &completion.finish_reason, &merged_cited),
        )?;
        let record = call("precise_synthesis", run, None, attempts, synthesis.is_some());
        precise_seconds += record.seconds;
        calls.push(record);
        println!("run {run} note_precise accepted {} in {precise_seconds:.1}s", synthesis.is_some());
        if let Some(synthesis) = synthesis {
            let note = assemble_precise_note(body, synthesis, &drafts);
            let expectations = check_note(&note.value, &fixture.traps);
            notes_record.push(Assembled {
                kind: "note_precise",
                run,
                window: None,
                complete: precise_complete,
                seconds: round(precise_seconds, 2),
                repairs: note.repairs,
                expectations,
                value: note.value,
            });
        }
    }
    server.stop()?;

    let report = serde_json::json!({
        "fixture": fixture.fixture_id,
        "runs": runs,
        "prompts": prompts.iter().map(|(version, _)| *version).collect::<Vec<_>>(),
        "max_tokens": {"window_call": WINDOW_MAX_TOKENS, "note_body": NOTE_MAX_TOKENS,
                       "precise_window": WINDOW_MAX_TOKENS, "precise_synthesis": WINDOW_MAX_TOKENS},
        "summary": summarize(&calls, &drafts_record, &notes_record, runs, &windows),
        "calls": calls,
        "drafts": drafts_record,
        "notes": notes_record,
    });
    let path = out_dir.join("lecture-contract.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report["summary"]).unwrap_or_default());
    println!("written to {}", path.display());
    Ok(())
}
