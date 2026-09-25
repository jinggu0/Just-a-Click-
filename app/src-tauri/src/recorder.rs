//! Owns the capture thread, the chunk writer and the status the interface polls.
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde::Serialize;

use crate::audio::{self, Signals, Source};
use crate::chunker::Chunker;
use crate::power::KeepAwake;
use crate::wav::SAMPLE_RATE;

/// Blocks buffered between the capture thread and the writer thread.
pub const CHANNEL_BLOCKS: usize = 64;

pub type CaptureFn = fn(Source, SyncSender<Vec<i16>>, Signals) -> Result<(), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Recording,
    Paused,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub source: Option<Source>,
    pub directory: Option<String>,
    pub recorded_seconds: f64,
    pub chunks: usize,
    pub last_chunk: Option<String>,
    pub discontinuities: u32,
    pub silence_seconds: f64,
    pub error: Option<String>,
}

impl Status {
    fn idle() -> Self {
        Self {
            phase: Phase::Idle,
            source: None,
            directory: None,
            recorded_seconds: 0.0,
            chunks: 0,
            last_chunk: None,
            discontinuities: 0,
            silence_seconds: 0.0,
            error: None,
        }
    }
}

struct Session {
    signals: Signals,
    capture: JoinHandle<()>,
    writer: JoinHandle<()>,
    keep_awake: KeepAwake,
}

pub struct Recorder {
    status: Arc<Mutex<Status>>,
    session: Option<Session>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(Status::idle())),
            session: None,
        }
    }

    pub fn status(&self) -> Status {
        let mut status = self.status.lock().expect("status lock").clone();
        if let Some(session) = &self.session {
            status.discontinuities = session.signals.discontinuities.load(Ordering::Relaxed);
            status.silence_seconds = seconds_from(&session.signals);
        }
        status
    }

    pub fn start(&mut self, source: Source, directory: PathBuf) -> Result<Status, String> {
        self.start_with(source, directory, audio::capture)
    }

    /// The capture function is injected so the recorder can be tested without a device.
    pub fn start_with(
        &mut self,
        source: Source,
        directory: PathBuf,
        capture: CaptureFn,
    ) -> Result<Status, String> {
        if matches!(self.status().phase, Phase::Recording | Phase::Paused) {
            return Err("recording already running".to_string());
        }
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let signals = Signals::new();
        let (sender, receiver) = sync_channel::<Vec<i16>>(CHANNEL_BLOCKS);
        set_status(&self.status, |status| {
            *status = Status {
                phase: Phase::Recording,
                source: Some(source),
                directory: Some(directory.display().to_string()),
                ..Status::idle()
            };
        });
        let writer_status = Arc::clone(&self.status);
        let writer_directory = directory.clone();
        let writer = std::thread::Builder::new()
            .name("chunk-writer".to_string())
            .spawn(move || write_chunks(receiver, writer_directory, writer_status))
            .map_err(|error| error.to_string())?;
        let capture_status = Arc::clone(&self.status);
        let capture_signals = signals.clone();
        let capture = std::thread::Builder::new()
            .name("audio-capture".to_string())
            .spawn(move || {
                if let Err(message) = capture(source, sender, capture_signals) {
                    set_status(&capture_status, |status| {
                        status.phase = Phase::Failed;
                        status.error = Some(message);
                    });
                }
            })
            .map_err(|error| error.to_string())?;
        self.session = Some(Session {
            signals,
            capture,
            writer,
            keep_awake: KeepAwake::new(),
        });
        Ok(self.status())
    }

    pub fn pause(&mut self) -> Result<Status, String> {
        let session = self.session.as_ref().ok_or("no recording to pause")?;
        session.signals.paused.store(true, Ordering::Relaxed);
        set_status(&self.status, |status| {
            if status.phase == Phase::Recording {
                status.phase = Phase::Paused;
            }
        });
        Ok(self.status())
    }

    pub fn resume(&mut self) -> Result<Status, String> {
        let session = self.session.as_ref().ok_or("no recording to resume")?;
        session.signals.paused.store(false, Ordering::Relaxed);
        set_status(&self.status, |status| {
            if status.phase == Phase::Paused {
                status.phase = Phase::Recording;
            }
        });
        Ok(self.status())
    }

    /// Stops capture, closes the open chunk and releases the sleep request.
    pub fn stop(&mut self) -> Result<Status, String> {
        let session = self.session.take().ok_or("no recording to stop")?;
        session.signals.stop.store(true, Ordering::Relaxed);
        let discontinuities = session.signals.discontinuities.load(Ordering::Relaxed);
        let silence_seconds = seconds_from(&session.signals);
        let _ = session.capture.join();
        let _ = session.writer.join();
        drop(session.keep_awake);
        set_status(&self.status, |status| {
            status.discontinuities = discontinuities;
            status.silence_seconds = silence_seconds;
            if status.phase != Phase::Failed {
                status.phase = Phase::Stopped;
            }
        });
        Ok(self.status())
    }
}

/// Silence inserted to keep the recording aligned with the clock, in seconds.
fn seconds_from(signals: &Signals) -> f64 {
    let samples = signals.silence_samples.load(Ordering::Relaxed) as f64;
    (samples / SAMPLE_RATE as f64 * 100.0).round() / 100.0
}

fn set_status<F: FnOnce(&mut Status)>(status: &Arc<Mutex<Status>>, change: F) {
    if let Ok(mut guard) = status.lock() {
        change(&mut guard);
    }
}

fn write_chunks(receiver: Receiver<Vec<i16>>, directory: PathBuf, status: Arc<Mutex<Status>>) {
    let mut chunker = Chunker::new(directory);
    while let Ok(block) = receiver.recv() {
        if let Err(error) = chunker.push(&block) {
            set_status(&status, |status| {
                status.phase = Phase::Failed;
                status.error = Some(format!("chunk write failed: {error}"));
            });
            return;
        }
        report(&status, &chunker);
    }
    if let Err(error) = chunker.close() {
        set_status(&status, |status| {
            status.phase = Phase::Failed;
            status.error = Some(format!("closing the chunk failed: {error}"));
        });
        return;
    }
    report(&status, &chunker);
}

fn report(status: &Arc<Mutex<Status>>, chunker: &Chunker) {
    let chunks = chunker.chunks().len();
    let last = chunker
        .chunks()
        .last()
        .map(|path| path.display().to_string())
        .or_else(|| chunker.open_chunk().map(|path| path.display().to_string()));
    let seconds = chunker.total_samples() as f64 / SAMPLE_RATE as f64;
    set_status(status, |status| {
        status.chunks = chunks;
        status.last_chunk = last;
        status.recorded_seconds = (seconds * 100.0).round() / 100.0;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::CHUNK_SAMPLES;
    use std::time::{Duration, Instant};

    fn temp_dir(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("jac-recorder-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn one_chunk_then_wait(
        _source: Source,
        samples: SyncSender<Vec<i16>>,
        signals: Signals,
    ) -> Result<(), String> {
        let block = vec![0i16; SAMPLE_RATE as usize];
        for _ in 0..30 {
            if samples.send(block.clone()).is_err() {
                return Ok(());
            }
        }
        let _ = samples.send(vec![0i16; 5]);
        while !signals.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn silent_until_stop(
        _source: Source,
        _samples: SyncSender<Vec<i16>>,
        signals: Signals,
    ) -> Result<(), String> {
        while !signals.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn missing_device(
        _source: Source,
        _samples: SyncSender<Vec<i16>>,
        _signals: Signals,
    ) -> Result<(), String> {
        Err("no default device".to_string())
    }

    fn wait_for(recorder: &Recorder, predicate: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = recorder.status();
            if predicate(&status) || Instant::now() > deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn recording_writes_chunks_and_reports_seconds() {
        let directory = temp_dir("chunks");
        let mut recorder = Recorder::new();
        let started = recorder
            .start_with(Source::Microphone, directory.clone(), one_chunk_then_wait)
            .unwrap();
        assert_eq!(started.phase, Phase::Recording);
        let progress = wait_for(&recorder, |status| status.chunks >= 1);
        assert_eq!(progress.chunks, 1);
        let stopped = recorder.stop().unwrap();
        assert_eq!(stopped.phase, Phase::Stopped);
        assert_eq!(stopped.chunks, 2);
        assert_eq!(
            stopped.recorded_seconds,
            ((CHUNK_SAMPLES as f64 + 5.0) / SAMPLE_RATE as f64 * 100.0).round() / 100.0
        );
        assert!(directory.join("chunk-0001.wav").exists());
        assert!(directory.join("chunk-0002.wav").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn pause_and_resume_change_the_phase() {
        let directory = temp_dir("pause");
        let mut recorder = Recorder::new();
        recorder
            .start_with(Source::SystemSound, directory.clone(), silent_until_stop)
            .unwrap();
        assert_eq!(recorder.pause().unwrap().phase, Phase::Paused);
        assert_eq!(recorder.resume().unwrap().phase, Phase::Recording);
        assert_eq!(recorder.stop().unwrap().phase, Phase::Stopped);
        assert!(recorder.pause().is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn capture_failure_is_reported_as_failed() {
        let directory = temp_dir("failure");
        let mut recorder = Recorder::new();
        recorder
            .start_with(Source::Microphone, directory.clone(), missing_device)
            .unwrap();
        let status = wait_for(&recorder, |status| status.phase == Phase::Failed);
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.error.as_deref(), Some("no default device"));
        assert_eq!(recorder.stop().unwrap().phase, Phase::Failed);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    #[ignore = "needs an audio device; run with cargo test -- --ignored"]
    fn a_real_microphone_recording_writes_a_chunk_file() {
        let directory = temp_dir("device");
        let mut recorder = Recorder::new();
        recorder
            .start(Source::Microphone, directory.clone())
            .unwrap();
        std::thread::sleep(Duration::from_secs(4));
        let status = recorder.stop().unwrap();
        assert_eq!(status.phase, Phase::Stopped, "error: {:?}", status.error);
        assert_eq!(status.chunks, 1);
        assert!(
            status.recorded_seconds >= 3.0,
            "recorded {} s",
            status.recorded_seconds
        );
        let bytes = std::fs::read(directory.join("chunk-0001.wav")).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            SAMPLE_RATE
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_second_start_is_refused_while_recording() {
        let directory = temp_dir("second");
        let mut recorder = Recorder::new();
        recorder
            .start_with(Source::Microphone, directory.clone(), silent_until_stop)
            .unwrap();
        assert!(recorder
            .start_with(Source::Microphone, directory.clone(), silent_until_stop)
            .is_err());
        recorder.stop().unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
