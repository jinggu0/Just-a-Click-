//! Tauri commands for the recording validation build.
pub mod audio;
pub mod chunker;
pub mod convert;
pub mod llm;
pub mod power;
pub mod process;
pub mod recorder;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(Recorder::new()))
        .invoke_handler(tauri::generate_handler![
            list_audio_sources,
            start_recording,
            pause_recording,
            resume_recording,
            stop_recording,
            recording_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
