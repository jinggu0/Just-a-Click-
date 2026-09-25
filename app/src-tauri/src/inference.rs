//! Wires the inference modules to the screen: one worker thread per action, a status the
//! screen polls, and cancel flags the screen can set while the work runs.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::llm::{stream_draft, Server, ServerSettings, Stop};
use crate::process::ProcessGroup;
use crate::stt::{run, Outcome, TranscribeSettings};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub phase: String,
    pub ready_seconds: f64,
    pub port: u16,
    pub last_answer: String,
    pub error: Option<String>,
}

/// The status the screen reads while a worker thread writes it.
#[derive(Clone)]
pub struct Shared(Arc<Mutex<Status>>);

impl Default for Shared {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Status {
            phase: "idle".to_string(),
            ..Status::default()
        })))
    }
}

impl Shared {
    pub fn read(&self) -> Status {
        self.0.lock().map(|status| status.clone()).unwrap_or_default()
    }

    pub fn phase(&self, phase: &str) {
        if let Ok(mut status) = self.0.lock() {
            status.phase = phase.to_string();
            status.error = None;
        }
    }

    pub fn ready(&self, seconds: f64, port: u16) {
        if let Ok(mut status) = self.0.lock() {
            status.phase = "ready".to_string();
            status.ready_seconds = seconds;
            status.port = port;
            status.error = None;
        }
    }

    pub fn answer(&self, text: &str) {
        if let Ok(mut status) = self.0.lock() {
            status.last_answer = text.to_string();
        }
    }

    pub fn failed(&self, reason: &str) {
        if let Ok(mut status) = self.0.lock() {
            status.phase = "failed".to_string();
            status.error = Some(reason.to_string());
        }
    }
}

/// Everything the commands share. A started server is handed over by the worker thread.
#[derive(Default)]
pub struct Inference {
    pub shared: Shared,
    pub draft_cancel: Arc<AtomicBool>,
    pub transcribe_cancel: Arc<AtomicBool>,
    group: Option<Arc<ProcessGroup>>,
    server: Arc<Mutex<Option<Server>>>,
}

impl Inference {
    /// Starts the server on a worker thread so the screen stays responsive.
    pub fn start(&mut self, settings: ServerSettings) {
        if self.server.lock().map(|server| server.is_some()).unwrap_or(false) {
            self.shared.failed("이미 실행 중입니다");
            return;
        }
        let group = match ProcessGroup::new() {
            Ok(group) => Arc::new(group),
            Err(error) => return self.shared.failed(&error),
        };
        self.group = Some(Arc::clone(&group));
        let shared = self.shared.clone();
        let slot = Arc::clone(&self.server);
        shared.phase("starting");
        std::thread::spawn(move || match Server::start(&group, &settings) {
            Ok(server) => {
                shared.ready(server.ready_seconds, server.port());
                if let Ok(mut place) = slot.lock() {
                    *place = Some(server);
                }
            }
            Err(error) => shared.failed(&error),
        });
    }

    /// Streams one draft on a worker thread; `cancel_draft` stops it.
    pub fn draft(&self, prompt: String, max_tokens: u32) {
        let Some((base, key)) = self.endpoint() else {
            return self.shared.failed("서버가 준비되지 않았습니다");
        };
        let shared = self.shared.clone();
        let cancel = Arc::clone(&self.draft_cancel);
        cancel.store(false, Ordering::Relaxed);
        shared.phase("drafting");
        std::thread::spawn(move || match stream_draft(&base, &key, &prompt, max_tokens, &cancel) {
            Ok((Stop::Finished, text)) => {
                shared.answer(&text);
                shared.phase("ready");
            }
            Ok((Stop::Cancelled, text)) => {
                shared.answer(&text);
                shared.phase("cancelled");
            }
            Err(error) => shared.failed(&error),
        });
    }

    /// Transcribes one chunk on a worker thread; `cancel_transcribe` stops it.
    pub fn transcribe(&self, settings: TranscribeSettings) {
        let Some(group) = self.group.as_ref().map(Arc::clone) else {
            return self.shared.failed("서버가 준비되지 않았습니다");
        };
        let shared = self.shared.clone();
        let cancel = Arc::clone(&self.transcribe_cancel);
        cancel.store(false, Ordering::Relaxed);
        shared.phase("transcribing");
        std::thread::spawn(move || match run(&group, &settings, &cancel) {
            Ok(Outcome::Text(text)) => {
                shared.answer(&text);
                shared.phase("ready");
            }
            Ok(Outcome::Cancelled) => shared.phase("cancelled"),
            Err(error) => shared.failed(&error),
        });
    }

    pub fn stop(&mut self) {
        if let Ok(mut server) = self.server.lock() {
            if let Some(server) = server.as_mut() {
                let _ = server.stop();
            }
            *server = None;
        }
        self.group = None;
        self.shared.phase("stopped");
    }

    fn endpoint(&self) -> Option<(String, String)> {
        let server = self.server.lock().ok()?;
        let server = server.as_ref()?;
        Some((server.base(), server.key().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_status_is_idle() {
        let status = Shared::default().read();
        assert_eq!(status.phase, "idle");
        assert_eq!(status.ready_seconds, 0.0);
        assert!(status.error.is_none());
    }

    #[test]
    fn the_phase_and_the_error_move_together() {
        let shared = Shared::default();
        shared.phase("starting");
        assert_eq!(shared.read().phase, "starting");
        shared.failed("서버가 뜨지 않았습니다");
        let status = shared.read();
        assert_eq!(status.phase, "failed");
        assert_eq!(status.error.as_deref(), Some("서버가 뜨지 않았습니다"));
        shared.phase("ready");
        assert!(shared.read().error.is_none(), "a new phase clears the last error");
    }

    #[test]
    fn the_answer_is_kept_for_the_screen() {
        let shared = Shared::default();
        shared.answer("한 문장 답입니다");
        assert_eq!(shared.read().last_answer, "한 문장 답입니다");
    }
}
