//! One whisper-cli run over one chunk, with a cancel that leaves no partial file behind.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::process::{stop, ProcessGroup};

pub const STOP_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TranscribeSettings {
    pub executable: PathBuf,
    pub model: PathBuf,
    pub chunk: PathBuf,
    /// Without a suffix; whisper-cli appends `.txt`.
    pub output: PathBuf,
    pub threads: u32,
}

pub fn output_file(settings: &TranscribeSettings) -> PathBuf {
    settings.output.with_extension("txt")
}

/// Decision 0008: Korean, eight threads, no timestamps.
pub fn transcribe_command(settings: &TranscribeSettings) -> Command {
    let mut command = Command::new(&settings.executable);
    command
        .args(["-m".to_string(), settings.model.to_string_lossy().to_string()])
        .args(["-f".to_string(), settings.chunk.to_string_lossy().to_string()])
        .args(["-l".to_string(), "ko".to_string()])
        .args(["-t".to_string(), settings.threads.to_string()])
        .arg("-nt")
        .arg("-otxt")
        .args(["-of".to_string(), settings.output.to_string_lossy().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub fn run(
    group: &ProcessGroup,
    settings: &TranscribeSettings,
    cancel: &AtomicBool,
) -> Result<Outcome, String> {
    run_with(group, settings, cancel, transcribe_command)
}

/// The command builder is injected so tests can stand in for whisper-cli.
pub fn run_with(
    group: &ProcessGroup,
    settings: &TranscribeSettings,
    cancel: &AtomicBool,
    build: fn(&TranscribeSettings) -> Command,
) -> Result<Outcome, String> {
    let mut command = build(settings);
    let mut child = group.spawn(&mut command)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            stop(&mut child, STOP_GRACE)?;
            let _ = std::fs::remove_file(output_file(settings));
            return Ok(Outcome::Cancelled);
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => break,
            Some(status) => return Err(format!("whisper-cli exited {status}")),
            None => std::thread::sleep(POLL),
        }
    }
    let text = std::fs::read_to_string(output_file(settings))
        .map_err(|error| format!("result file missing: {error}"))?;
    Ok(Outcome::Text(text.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn settings(command: &str, output: PathBuf) -> TranscribeSettings {
        TranscribeSettings {
            executable: PathBuf::from("cmd"),
            model: PathBuf::from(command),
            chunk: PathBuf::from("chunk-0001.wav"),
            output,
            threads: 8,
        }
    }

    #[test]
    fn the_command_asks_for_korean_without_timestamps() {
        let settings = TranscribeSettings {
            executable: PathBuf::from("whisper-cli.exe"),
            model: PathBuf::from("model.bin"),
            chunk: PathBuf::from("chunk-0001.wav"),
            output: PathBuf::from("out"),
            threads: 8,
        };
        let arguments: Vec<String> = transcribe_command(&settings)
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect();
        for expected in ["-l", "ko", "-nt", "-otxt", "-t", "8"] {
            assert!(arguments.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn the_output_file_gets_the_txt_suffix() {
        let settings = settings("unused", PathBuf::from("run/out"));
        assert_eq!(output_file(&settings), PathBuf::from("run/out.txt"));
    }

    #[test]
    fn a_cancelled_run_removes_the_partial_output() {
        let directory = std::env::temp_dir().join("jac-stt-cancel");
        let _ = std::fs::create_dir_all(&directory);
        let output = directory.join("out");
        std::fs::write(output.with_extension("txt"), "부분 결과").expect("write");
        let mut settings = settings("unused", output.clone());
        settings.executable = PathBuf::from("cmd");
        let group = crate::process::ProcessGroup::new().expect("group");
        let cancel = AtomicBool::new(true);
        let outcome = run_with(&group, &settings, &cancel, sleeper).expect("run");
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(!output.with_extension("txt").exists(), "partial output should be gone");
    }

    #[test]
    fn a_finished_run_returns_the_text() {
        let directory = std::env::temp_dir().join("jac-stt-done");
        let _ = std::fs::create_dir_all(&directory);
        let output = directory.join("out");
        let _ = std::fs::remove_file(output.with_extension("txt"));
        let settings = settings("unused", output.clone());
        let group = crate::process::ProcessGroup::new().expect("group");
        let cancel = AtomicBool::new(false);
        let outcome = run_with(&group, &settings, &cancel, writer).expect("run");
        assert_eq!(outcome, Outcome::Text("korean result".to_string()));
    }

    /// Stands in for whisper-cli: keeps running so the cancel path has something to stop.
    fn sleeper(_settings: &TranscribeSettings) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 30 127.0.0.1 > nul"]);
        command
    }

    /// Stands in for whisper-cli: writes the result file and exits. The text stays ASCII
    /// because `echo` writes in the console code page, not UTF-8, and `raw_arg` keeps
    /// Rust from escaping the quotes cmd.exe needs around the path.
    fn writer(settings: &TranscribeSettings) -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        command
            .arg("/c")
            .raw_arg(format!("echo korean result> \"{}\"", output_file(settings).display()));
        command
    }
}
