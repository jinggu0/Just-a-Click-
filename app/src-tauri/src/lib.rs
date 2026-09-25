//! Tauri commands for the recording validation build.
pub mod audio;
pub mod chunker;
pub mod convert;
pub mod inference;
pub mod llm;
pub mod power;
pub mod process;
pub mod recorder;
pub mod search;
pub mod secrets;
pub mod store;
pub mod stt;
pub mod wav;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::State;

use crate::audio::{describe_sources, Source, SourceInfo};
use crate::recorder::{Recorder, Status};

type Shared<'a> = State<'a, Mutex<Recorder>>;

fn with_recorder<F>(recorder: Shared<'_>, action: F) -> Result<Status, String>
where
    F: FnOnce(&mut Recorder) -> Result<Status, String>,
{
    let mut guard = recorder
        .lock()
        .map_err(|_| "recorder state is poisoned".to_string())?;
    action(&mut guard)
}

#[tauri::command]
fn list_audio_sources() -> Vec<SourceInfo> {
    describe_sources()
}

#[tauri::command]
fn start_recording(
    source: Source,
    directory: String,
    recorder: Shared<'_>,
) -> Result<Status, String> {
    with_recorder(recorder, |inner| {
        inner.start(source, PathBuf::from(directory))
    })
}

#[tauri::command]
fn pause_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.pause())
}

#[tauri::command]
fn resume_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.resume())
}

#[tauri::command]
fn stop_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.stop())
}

#[tauri::command]
fn recording_status(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| Ok(inner.status()))
}

#[tauri::command]
fn start_inference(root: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    let root = std::path::PathBuf::from(root);
    let settings = llm::ServerSettings {
        executable: root.join("runtimes/b10994/vulkan/llama-server.exe"),
        model: root.join("models/Qwen3-8B-Q5_K_M.gguf"),
        log: root.join("artifacts/app-inference/screen-server.log"),
        context_tokens: 8192,
        threads: 2,
    };
    state.lock().map_err(|error| error.to_string())?.start(settings);
    Ok(())
}

#[tauri::command]
fn inference_status(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<inference::Status, String> {
    Ok(state.lock().map_err(|error| error.to_string())?.shared.read())
}

#[tauri::command]
fn run_draft(prompt: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state.lock().map_err(|error| error.to_string())?.draft(prompt, 300);
    Ok(())
}

#[tauri::command]
fn cancel_draft(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state
        .lock()
        .map_err(|error| error.to_string())?
        .draft_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn run_transcribe(root: String, chunk: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    let root = std::path::PathBuf::from(root);
    let chunk = std::path::PathBuf::from(chunk);
    // whisper-cli appends `.txt` to this name, so the name itself must carry no extension.
    let stem = chunk
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let settings = stt::TranscribeSettings {
        executable: root.join("runtimes/whisper-b5130/blas/Release/whisper-cli.exe"),
        model: root.join("models/whisper/ggml-large-v3-turbo.bin"),
        output: chunk.with_file_name(format!("{stem}-screen")),
        chunk,
        threads: 8,
    };
    state.lock().map_err(|error| error.to_string())?.transcribe(settings);
    Ok(())
}

#[tauri::command]
fn cancel_transcribe(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state
        .lock()
        .map_err(|error| error.to_string())?
        .transcribe_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn stop_inference(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state.lock().map_err(|error| error.to_string())?.stop();
    Ok(())
}

#[tauri::command]
fn search_notes(database: String, query: String, method: String, limit: usize) -> Result<Vec<i64>, String> {
    let method = store::Method::from_label(&method).ok_or("알 수 없는 검색 방식입니다")?;
    let connection = store::open(std::path::Path::new(&database))?;
    search::search(&connection, method, &query, limit)
}

#[tauri::command]
fn save_notion_token(value: String) -> Result<(), String> {
    secrets::save(&secrets::target("notion"), &value)
}

#[tauri::command]
fn notion_token_saved() -> Result<bool, String> {
    Ok(secrets::load(&secrets::target("notion"))?.is_some())
}

#[tauri::command]
fn delete_notion_token() -> Result<(), String> {
    secrets::delete(&secrets::target("notion"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(Recorder::new()))
        .manage(Mutex::new(inference::Inference::default()))
        .invoke_handler(tauri::generate_handler![
            list_audio_sources,
            start_recording,
            pause_recording,
            resume_recording,
            stop_recording,
            recording_status,
            start_inference,
            inference_status,
            run_draft,
            cancel_draft,
            run_transcribe,
            cancel_transcribe,
            stop_inference,
            search_notes,
            save_notion_token,
            notion_token_saved,
            delete_notion_token
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
