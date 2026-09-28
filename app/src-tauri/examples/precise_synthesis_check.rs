//! Replays only the precise note's synthesis call on notes a lecture run saved.
//!
//! Usage: cargo run --release --example precise_synthesis_check -- <repository root> <out dir> <lecture-contract.json>...
//!
//! Each saved precise note's concepts, examples and terms are read back as the merged body,
//! and the synthesis is asked again with the names-and-sources input. Every note's concepts
//! strung together, and that list twice over, stand in for a long lecture. The same doubled
//! list with explanations, the input format this replaces, is checked for size and must be
//! refused without a call.
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use app_lib::contract::{generation_schema, Segment, Violation, PRECISE_SYNTHESIS_SCHEMA};
use app_lib::lecture::{precise_synthesis_prompt, validate_precise_synthesis, Accepted, Concept, Item, PreciseSynthesis, Term};
use app_lib::lecture_fixture::Fixture;
use app_lib::lecture_merge::{cited_by, fits_context, synthesis_input, PreciseBody};
use app_lib::llm::{complete_json, count_tokens, stream_draft, Server, ServerSettings};
use app_lib::power::KeepAwake;
use app_lib::process::ProcessGroup;

const CONTEXT_TOKENS: u32 = 8_192;
const MAX_TOKENS: u32 = 2_048;

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn list<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Result<Vec<T>, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

/// The merged body a saved precise note was assembled from. `term_en_source` is set by the
/// app and refused on input, so it is dropped before reading.
fn body_of(note: &serde_json::Value) -> Result<PreciseBody, String> {
    let mut terms = note["terms"].clone();
    for term in terms.as_array_mut().ok_or("terms missing")? {
        term.as_object_mut().ok_or("term is not an object")?.remove("term_en_source");
    }
    Ok(PreciseBody {
        concepts: list::<Concept>(&note["concepts"])?,
        examples: list::<Item>(&note["examples"])?,
        terms: list::<Term>(&terms)?,
    })
}

struct Outcome {
    prompt_tokens: usize,
    seconds: f64,
    attempts: usize,
    accepted: Option<Accepted<PreciseSynthesis>>,
    violations: Vec<Violation>,
}

/// Asks once, and once more with the violations if the answer is refused.
fn synthesize(base: &str, key: &str, course: &str, body: &PreciseBody, all: &[Segment]) -> Result<Outcome, String> {
    let system = precise_synthesis_prompt();
    let user = format!("과목: {course}\n\n개념 목록:\n{}", synthesis_input(body));
    let prompt_tokens = count_tokens(base, key, &format!("{system}\n{user}"))?;
    let cited = cited_by(body, all);
    let ids: Vec<String> = cited.iter().map(|segment| segment.id.clone()).collect();
    let schema = generation_schema(PRECISE_SYNTHESIS_SCHEMA, &ids)?;
    let mut request = user.clone();
    let mut seconds = 0.0;
    let mut violations = Vec::new();
    for attempt in 1..=2 {
        let completion = complete_json(base, key, &system, &request, &schema, MAX_TOKENS)?;
        seconds += completion.seconds;
        match validate_precise_synthesis(&completion.content, &completion.finish_reason, &cited) {
            Ok(accepted) => {
                return Ok(Outcome { prompt_tokens, seconds, attempts: attempt, accepted: Some(accepted), violations })
            }
            Err(found) => {
                let listed: Vec<String> =
                    found.iter().map(|violation| format!("- {} {}: {}", violation.path, violation.rule, violation.detail)).collect();
                request = format!("{user}\n\n이전 답이 다음 규칙을 어겼다. 규칙을 지켜 처음부터 다시 쓴다.\n{}", listed.join("\n"));
                violations = found;
            }
        }
    }
    Ok(Outcome { prompt_tokens, seconds, attempts: 2, accepted: None, violations })
}

fn cites_chatter(synthesis: &PreciseSynthesis, chatter: &[String]) -> bool {
    std::iter::once(&synthesis.topic)
        .chain(&synthesis.review)
        .flat_map(|item| &item.source_refs)
        .any(|id| chatter.contains(id))
}

fn record(outcome: &Outcome, chatter: &[String]) -> serde_json::Value {
    serde_json::json!({
        "prompt_tokens": outcome.prompt_tokens,
        "seconds": round(outcome.seconds),
        "attempts": outcome.attempts,
        "accepted": outcome.accepted.is_some(),
        "violations": outcome.violations,
        "cites_chatter": outcome.accepted.as_ref().map(|accepted| cites_chatter(&accepted.value, chatter)),
        "topic": outcome.accepted.as_ref().map(|accepted| &accepted.value.topic),
        "review": outcome.accepted.as_ref().map(|accepted| &accepted.value.review),
    })
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: precise_synthesis_check <repository root> <out dir> <lecture-contract.json>...".into());
    }
    let root = PathBuf::from(&arguments[0]);
    let out_dir = PathBuf::from(&arguments[1]);
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    let fixture = Fixture::load(&root.join("evaluation/fixtures/lecture-synthetic-v1.json"))?;
    let all = fixture.all_segments()?;
    let chatter = fixture.traps.chatter.clone();

    let mut saved = Vec::new();
    for path in &arguments[2..] {
        let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
        let report: serde_json::Value = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
        for note in report["notes"].as_array().ok_or("notes missing")? {
            if note["kind"] == "note_precise" {
                saved.push((path.clone(), note["run"].clone(), note["value"].clone()));
            }
        }
    }

    let _awake = KeepAwake::new();
    let group = ProcessGroup::new()?;
    let settings = ServerSettings {
        executable: root.join("runtimes/b10994/vulkan/llama-server.exe"),
        model: root.join("models/Qwen3-8B-Q5_K_M.gguf"),
        log: out_dir.join("server.log"),
        context_tokens: CONTEXT_TOKENS,
        threads: 2,
    };
    let mut server = Server::start(&group, &settings)?;
    println!("server ready in {:.1}s", server.ready_seconds);
    stream_draft(&server.base(), server.key(), "안녕하세요.", 8, &AtomicBool::new(false))?;
    let (base, key) = (server.base(), server.key().to_string());

    let mut replays = Vec::new();
    let mut every_concept = Vec::new();
    for (path, run, note) in &saved {
        let body = body_of(note)?;
        every_concept.extend(body.concepts.clone());
        let outcome = synthesize(&base, &key, &fixture.course, &body, &all)?;
        println!(
            "replay {path} run {run}: {} concepts, {} tokens, accepted {} in {:.1}s",
            body.concepts.len(),
            outcome.prompt_tokens,
            outcome.accepted.is_some(),
            outcome.seconds
        );
        let mut entry = record(&outcome, &chatter);
        entry["source"] = serde_json::json!(path);
        entry["run"] = run.clone();
        entry["concepts"] = serde_json::json!(body.concepts.len());
        entry["before"] = serde_json::json!({"topic": note["topic"], "review": note["review"]});
        replays.push(entry);
    }

    let mut scale = Vec::new();
    for copies in [1, 2] {
        let concepts: Vec<Concept> = (0..copies).flat_map(|_| every_concept.clone()).collect();
        let body = PreciseBody { concepts, examples: Vec::new(), terms: Vec::new() };
        let outcome = synthesize(&base, &key, &fixture.course, &body, &all)?;
        println!(
            "scale {} concepts: {} tokens, accepted {} in {:.1}s",
            body.concepts.len(),
            outcome.prompt_tokens,
            outcome.accepted.is_some(),
            outcome.seconds
        );
        let mut entry = record(&outcome, &chatter);
        entry["concepts"] = serde_json::json!(body.concepts.len());
        scale.push(entry);
    }

    // The input this replaces: every concept with its explanation, for the doubled list.
    let explained: Vec<String> = (0..2)
        .flat_map(|_| every_concept.iter())
        .map(|concept| format!("- {} [{}]: {}", concept.name, concept.source_refs.join(", "), concept.explanation))
        .collect();
    let old_user = format!("과목: {}\n\n개념 목록:\n{}", fixture.course, explained.join("\n"));
    let old_tokens = count_tokens(&base, &key, &format!("{}\n{old_user}", precise_synthesis_prompt()))?;
    let fits = fits_context(old_tokens, MAX_TOKENS, CONTEXT_TOKENS);
    println!("size check: {} explained concepts, {old_tokens} tokens, fits {fits}", explained.len());
    let size_check = serde_json::json!({
        "concepts": explained.len(),
        "prompt_tokens": old_tokens,
        "fits": fits,
        "outcome": if fits { "would be called" } else { "input_too_large, not called" },
    });
    server.stop()?;

    let report = serde_json::json!({
        "prompt": precise_synthesis_prompt(),
        "budget_tokens": CONTEXT_TOKENS as usize - MAX_TOKENS as usize - app_lib::lecture_merge::TEMPLATE_TOKENS,
        "replays": replays,
        "scale": scale,
        "size_check": size_check,
    });
    let path = out_dir.join("precise-synthesis.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default()).map_err(|error| error.to_string())?;
    println!("written to {}", path.display());
    Ok(())
}
