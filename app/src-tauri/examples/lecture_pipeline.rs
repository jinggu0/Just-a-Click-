//! Records a real lecture and processes it while it is being recorded, the way the app will.
//!
//! Usage: cargo run --release --example lecture_pipeline -- <repository root> <out dir> <seconds>
//!
//! The recorder closes a 30-second chunk at a time; each closed chunk is transcribed at once
//! and becomes one segment. Every ten segments (five minutes) make a window whose points,
//! notices and code are asked for while the recording goes on. When the recording stops, at
//! `<seconds>` or when `<out dir>/stop` appears, the remaining chunks and window are
//! processed, then the five-minute note and the precise note. The time from the stop to the
//! saved five-minute note is the after-recording time of decision 0009.
//!
//! The course name is read from `<out dir>/course.txt` before every call, so it can be given
//! after the recording has started. Audio, transcript and notes stay under `<out dir>`.
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use app_lib::audio::Source;
use app_lib::contract::{
    generation_schema, Segment, Violation, CODE_SCHEMA, NOTE_BODY_SCHEMA, NOTICES_SCHEMA, POINTS_SCHEMA,
    PRECISE_SYNTHESIS_SCHEMA, PRECISE_WINDOW_SCHEMA,
};
use app_lib::lecture::{
    code_prompt, note_body_prompt, notices_prompt, points_prompt, precise_synthesis_prompt, precise_window_prompt,
    uncovered_points, validate_code, validate_note_body, validate_notices, validate_points,
    validate_precise_synthesis, validate_precise_window, Accepted, Item, PreciseWindow, Repair, PRECISE_WINDOW_VERSION,
};
use app_lib::lecture_merge::{
    assemble_draft, assemble_note, assemble_precise_note, cited_by, fitting_synthesis_input, merge_precise,
    precise_checklist, Draft, Fitting,
};
use app_lib::llm::{complete_json, count_tokens, stream_draft, Completion, Server, ServerSettings};
use app_lib::power::KeepAwake;
use app_lib::process::ProcessGroup;
use app_lib::recorder::Recorder;
use app_lib::stt::{self, TranscribeSettings};
use serde::Serialize;

const CHUNK_SECONDS: usize = 30;
const SEGMENTS_PER_WINDOW: usize = 10;
const CONTEXT_TOKENS: u32 = 8_192;
const WINDOW_MAX_TOKENS: u32 = 2_048;
const NOTE_MAX_TOKENS: u32 = 4_096;

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
    window: Option<usize>,
    accepted: bool,
    seconds: f64,
    /// Seconds since the recording started when the call ended.
    at: f64,
    attempts: Vec<Attempt>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    uncovered: Vec<usize>,
}

#[derive(Serialize)]
struct Transcribed {
    chunk: usize,
    id: String,
    seconds: f64,
    at: f64,
    text: String,
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
                seconds: round(completion.seconds),
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

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn empty<T>() -> Accepted<Vec<T>> {
    Accepted { value: Vec::new(), repairs: Vec::new() }
}

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

fn ids(segments: &[Segment]) -> Vec<String> {
    segments.iter().map(|segment| segment.id.clone()).collect()
}

fn lines(segments: &[Segment]) -> String {
    segments.iter().map(Segment::line).collect::<Vec<_>>().join("\n")
}

fn listed(points: &[Item]) -> String {
    points
        .iter()
        .map(|point| format!("- {} [{}]", point.content, point.source_refs.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn course(out_dir: &Path) -> String {
    std::fs::read_to_string(out_dir.join("course.txt"))
        .map(|text| text.trim().to_string())
        .ok()
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "미입력".to_string())
}

fn clock(seconds: usize) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn call(stage: &'static str, window: Option<usize>, attempts: Vec<Attempt>, accepted: bool, at: f64) -> Call {
    let seconds = round(attempts.iter().map(|attempt| attempt.seconds).sum());
    Call { stage, window, accepted, seconds, at: round(at), attempts, uncovered: Vec::new() }
}

struct Pipeline {
    root: PathBuf,
    out_dir: PathBuf,
    audio_dir: PathBuf,
    text_dir: PathBuf,
    group: ProcessGroup,
    endpoint: Endpoint,
    started: Instant,
    segments: Vec<Segment>,
    transcribed: Vec<Transcribed>,
    skipped_chunks: Vec<usize>,
    windows: Vec<Vec<Segment>>,
    drafts: Vec<Draft>,
    draft_records: Vec<serde_json::Value>,
    calls: Vec<Call>,
    next_chunk: usize,
    next_segment: usize,
}

impl Pipeline {
    fn at(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Transcribes closed chunks in order. An empty transcript leaves no segment.
    fn transcribe_up_to(&mut self, closed: usize) -> Result<(), String> {
        while self.next_chunk <= closed {
            let number = self.next_chunk;
            let chunk = self.audio_dir.join(format!("chunk-{number:04}.wav"));
            let settings = TranscribeSettings {
                executable: self.root.join("runtimes/whisper-b5130/blas/Release/whisper-cli.exe"),
                model: self.root.join("models/whisper/ggml-large-v3-turbo.bin"),
                chunk,
                output: self.text_dir.join(format!("chunk-{number:04}")),
                threads: 8,
            };
            let begun = Instant::now();
            let text = match stt::run(&self.group, &settings, &AtomicBool::new(false))? {
                stt::Outcome::Text(text) => text.split_whitespace().collect::<Vec<_>>().join(" "),
                stt::Outcome::Cancelled => String::new(),
            };
            let seconds = begun.elapsed().as_secs_f64();
            if text.is_empty() {
                self.skipped_chunks.push(number);
            } else {
                self.next_segment += 1;
                let segment = Segment {
                    id: format!("s{}", self.next_segment),
                    time: clock((number - 1) * CHUNK_SECONDS),
                    text: text.clone(),
                };
                println!("{:>7.1}s chunk {number} -> {} in {seconds:.1}s", self.at(), segment.id);
                self.transcribed.push(Transcribed {
                    chunk: number,
                    id: segment.id.clone(),
                    seconds: round(seconds),
                    at: round(self.at()),
                    text,
                });
                self.segments.push(segment);
            }
            self.next_chunk += 1;
        }
        Ok(())
    }

    /// Segments not yet in a window.
    fn pending(&self) -> Vec<Segment> {
        let used: usize = self.windows.iter().map(Vec::len).sum();
        self.segments[used..].to_vec()
    }

    /// One window's three calls and its draft.
    fn draft(&mut self, window: Vec<Segment>) -> Result<(), String> {
        let index = self.windows.len() + 1;
        let user = format!("과목: {}\n\n전사:\n{}", course(&self.out_dir), lines(&window));
        let window_ids = ids(&window);
        let label = format!("window{index}");
        let begun = self.at();
        let (points, attempts) = self.endpoint.ask(
            &format!("{label}-points"),
            &points_prompt(),
            &user,
            &generation_schema(POINTS_SCHEMA, &window_ids)?,
            WINDOW_MAX_TOKENS,
            |completion| validate_points(&completion.content, &completion.finish_reason, &window),
        )?;
        self.calls.push(call("points", Some(index), attempts, points.is_some(), self.at()));
        let (notices, attempts) = self.endpoint.ask(
            &format!("{label}-notices"),
            &notices_prompt(),
            &user,
            &generation_schema(NOTICES_SCHEMA, &window_ids)?,
            WINDOW_MAX_TOKENS,
            |completion| validate_notices(&completion.content, &completion.finish_reason, &window),
        )?;
        self.calls.push(call("notices", Some(index), attempts, notices.is_some(), self.at()));
        let (code, attempts) = self.endpoint.ask(
            &format!("{label}-code"),
            &code_prompt(),
            &user,
            &generation_schema(CODE_SCHEMA, &window_ids)?,
            WINDOW_MAX_TOKENS,
            |completion| validate_code(&completion.content, &completion.finish_reason, &window),
        )?;
        self.calls.push(call("code", Some(index), attempts, code.is_some(), self.at()));
        let complete = points.is_some() && notices.is_some() && code.is_some();
        let draft = assemble_draft(
            &window,
            points.unwrap_or_else(empty),
            notices.unwrap_or_else(empty),
            code.unwrap_or_else(empty),
        );
        let seconds = self.at() - begun;
        println!("{:>7.1}s window {index} complete {complete} in {seconds:.1}s", self.at());
        self.draft_records.push(serde_json::json!({
            "window": index,
            "complete": complete,
            "seconds": round(seconds),
            "ended_at": round(self.at()),
            "repairs": draft.repairs,
            "value": draft.value,
        }));
        self.drafts.push(draft.value);
        self.windows.push(window);
        Ok(())
    }
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: lecture_pipeline <repository root> <out dir> <seconds>".into());
    }
    let root = PathBuf::from(&arguments[0]);
    let out_dir = PathBuf::from(&arguments[1]);
    let limit: f64 = arguments[2].parse().map_err(|_| "seconds must be a number")?;
    let audio_dir = out_dir.join("audio");
    let text_dir = out_dir.join("text");
    let raw_dir = out_dir.join("raw");
    for directory in [&audio_dir, &text_dir, &raw_dir] {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
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
    let endpoint = Endpoint { base: server.base(), key: server.key().to_string(), raw_dir };

    let mut recorder = Recorder::new();
    recorder.start(Source::Microphone, audio_dir.clone())?;
    let mut pipeline = Pipeline {
        root: root.clone(),
        out_dir: out_dir.clone(),
        audio_dir,
        text_dir,
        group,
        endpoint,
        started: Instant::now(),
        segments: Vec::new(),
        transcribed: Vec::new(),
        skipped_chunks: Vec::new(),
        windows: Vec::new(),
        drafts: Vec::new(),
        draft_records: Vec::new(),
        calls: Vec::new(),
        next_chunk: 1,
        next_segment: 0,
    };
    println!("recording started");

    // While recording: transcribe each closed chunk, draft each full window.
    loop {
        let status = recorder.status();
        if let Some(error) = &status.error {
            println!("recorder error: {error}");
            break;
        }
        if status.recorded_seconds >= limit || out_dir.join("stop").exists() {
            break;
        }
        pipeline.transcribe_up_to(status.chunks)?;
        let pending = pipeline.pending();
        if pending.len() >= SEGMENTS_PER_WINDOW {
            pipeline.draft(pending[..SEGMENTS_PER_WINDOW].to_vec())?;
            continue;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let final_status = recorder.stop()?;
    let stopped = pipeline.at();
    println!(
        "{stopped:>7.1}s recording stopped: {:.1}s in {} chunks, {} discontinuities",
        final_status.recorded_seconds, final_status.chunks, final_status.discontinuities
    );

    // After recording: the rest of the transcript and windows, then the notes.
    pipeline.transcribe_up_to(final_status.chunks)?;
    loop {
        let pending = pipeline.pending();
        if pending.is_empty() {
            break;
        }
        let take = pending.len().min(SEGMENTS_PER_WINDOW);
        pipeline.draft(pending[..take].to_vec())?;
    }
    let all = pipeline.segments.clone();
    let transcript_path = out_dir.join("transcript.txt");
    std::fs::write(&transcript_path, lines(&all)).map_err(|error| error.to_string())?;

    let cited: Vec<Segment> = {
        let refs: std::collections::BTreeSet<&String> = pipeline
            .drafts
            .iter()
            .flat_map(|draft| draft.points.iter().flat_map(|item| &item.source_refs))
            .collect();
        all.iter().filter(|segment| refs.contains(&segment.id)).cloned().collect()
    };
    let points_json: Vec<String> = pipeline
        .drafts
        .iter()
        .map(|draft| serde_json::json!({"window": draft.window, "points": draft.points}).to_string())
        .collect();
    let user = format!(
        "과목: {}\n\n구간 요점:\n{}\n\n요점이 인용한 전사 구간:\n{}",
        course(&out_dir),
        points_json.join("\n"),
        lines(&cited)
    );
    let (body, attempts) = pipeline.endpoint.ask(
        "note_from_drafts",
        &note_body_prompt(),
        &user,
        &generation_schema(NOTE_BODY_SCHEMA, &ids(&cited))?,
        NOTE_MAX_TOKENS,
        |completion| validate_note_body(&completion.content, &completion.finish_reason, &all),
    )?;
    let at = pipeline.at();
    pipeline.calls.push(call("note_body_from_drafts", None, attempts, body.is_some(), at));
    let five_minute_note = body.map(|body| assemble_note(body, &pipeline.drafts));
    let after_recording = pipeline.at() - stopped;
    println!("{:>7.1}s five-minute note saved {} ({after_recording:.1}s after the stop)", pipeline.at(), five_minute_note.is_some());

    // The precise note: each window with its draft points as a checklist, merged, synthesized.
    let precise_started = pipeline.at();
    let mut precise_windows = Vec::new();
    let windows = pipeline.windows.clone();
    for (index, window) in windows.iter().enumerate() {
        let checklist = precise_checklist(&pipeline.drafts[index]);
        let user = if checklist.is_empty() {
            format!("과목: {}\n\n전사:\n{}", course(&out_dir), lines(window))
        } else {
            format!("과목: {}\n\n이 구간의 요점 목록:\n{}\n\n전사:\n{}", course(&out_dir), listed(&checklist), lines(window))
        };
        let label = format!("precise-window{}", index + 1);
        let schema = generation_schema(PRECISE_WINDOW_SCHEMA, &ids(window))?;
        let validate =
            |completion: &Completion| validate_precise_window(&completion.content, &completion.finish_reason, window);
        let (answer, mut attempts) =
            pipeline.endpoint.ask(&label, &precise_window_prompt(), &user, &schema, WINDOW_MAX_TOKENS, validate)?;
        let mut uncovered = Vec::new();
        let answer = match answer {
            Some(first) => {
                let missing = uncovered_points(&first.value, &checklist);
                uncovered.push(missing.len());
                if missing.is_empty() {
                    Some(first)
                } else {
                    let request = format!(
                        "{user}\n\n이전 답이 다음 요점을 다루지 않았다. 빠진 요점까지 포함해 처음부터 다시 쓴다.\n{}",
                        listed(&missing)
                    );
                    let (second, more) = pipeline.endpoint.ask(
                        &format!("{label}-cover"),
                        &precise_window_prompt(),
                        &request,
                        &schema,
                        WINDOW_MAX_TOKENS,
                        validate,
                    )?;
                    attempts.extend(more);
                    match second {
                        Some(second) => {
                            let still = uncovered_points(&second.value, &checklist).len();
                            uncovered.push(still);
                            Some(if still < missing.len() { second } else { first })
                        }
                        None => Some(first),
                    }
                }
            }
            None => None,
        };
        let at = pipeline.at();
        let mut record = call("precise_window", Some(index + 1), attempts, answer.is_some(), at);
        record.uncovered = uncovered;
        println!("{:>7.1}s precise window {} accepted {} uncovered {:?}", at, index + 1, record.accepted, record.uncovered);
        pipeline.calls.push(record);
        precise_windows.push(answer.unwrap_or_else(empty_window));
    }
    let body = merge_precise(precise_windows, &all);
    let merged_cited = cited_by(&body.value, &all);
    let mut synthesis_input_record = serde_json::Value::Null;
    let mut precise_note = None;
    if !merged_cited.is_empty() {
        let course_line = format!("과목: {}\n\n개념 목록:\n", course(&out_dir));
        let system = precise_synthesis_prompt();
        let fitting = fitting_synthesis_input(&body.value, WINDOW_MAX_TOKENS, CONTEXT_TOKENS, |list| {
            count_tokens(&pipeline.endpoint.base, &pipeline.endpoint.key, &format!("{system}\n{course_line}{list}"))
        })?;
        match fitting {
            Fitting::Input { form, text, tokens } => {
                synthesis_input_record = serde_json::json!({"form": form, "prompt_tokens": tokens});
                let (synthesis, attempts) = pipeline.endpoint.ask(
                    "precise-synthesis",
                    &system,
                    &format!("{course_line}{text}"),
                    &generation_schema(PRECISE_SYNTHESIS_SCHEMA, &ids(&merged_cited))?,
                    WINDOW_MAX_TOKENS,
                    |completion| validate_precise_synthesis(&completion.content, &completion.finish_reason, &merged_cited),
                )?;
                let at = pipeline.at();
                pipeline.calls.push(call("precise_synthesis", None, attempts, synthesis.is_some(), at));
                precise_note = synthesis.map(|synthesis| assemble_precise_note(body, synthesis, &pipeline.drafts));
            }
            Fitting::TooLarge { tokens } => {
                synthesis_input_record = serde_json::json!({"form": null, "prompt_tokens": tokens, "reason": "input_too_large"});
            }
        }
    }
    let precise_seconds = pipeline.at() - precise_started;
    println!("{:>7.1}s precise note saved {} in {precise_seconds:.1}s", pipeline.at(), precise_note.is_some());
    server.stop()?;

    let stt_seconds: f64 = pipeline.transcribed.iter().map(|chunk| chunk.seconds).sum();
    let report = serde_json::json!({
        "recorded_seconds": final_status.recorded_seconds,
        "chunks": final_status.chunks,
        "discontinuities": final_status.discontinuities,
        "silence_seconds": final_status.silence_seconds,
        "segments": all.len(),
        "skipped_chunks": pipeline.skipped_chunks,
        "stt_seconds": round(stt_seconds),
        "stt_real_time_factor": round(stt_seconds / final_status.recorded_seconds.max(1.0)),
        "stopped_at": round(stopped),
        "after_recording_seconds": round(after_recording),
        "precise_seconds": round(precise_seconds),
        "synthesis_input": synthesis_input_record,
        "all_calls_accepted": pipeline.calls.iter().all(|call| call.accepted),
        "calls": pipeline.calls,
        "transcribed": pipeline.transcribed,
        "drafts": pipeline.draft_records,
        "five_minute_note": five_minute_note,
        "precise_note": precise_note,
    });
    let path = out_dir.join("lecture-pipeline.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default()).map_err(|error| error.to_string())?;
    println!("written to {}", path.display());
    Ok(())
}
