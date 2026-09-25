//! The llama-server child: start it, wait until it answers, stream a draft, cancel it.
//!
//! The key never appears on the command line; llama-server reads it from the environment,
//! exactly like `scripts/llm_server.py` does for the measurements.
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::process::{stop, ProcessGroup};

pub const READY_TIMEOUT: Duration = Duration::from_secs(300);
pub const STOP_GRACE: Duration = Duration::from_secs(5);
const HEALTH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Finished,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ServerSettings {
    pub executable: PathBuf,
    pub model: PathBuf,
    pub log: PathBuf,
    pub context_tokens: u32,
    pub threads: u32,
}

/// Decision 0009: context 8,192, one slot, no prompt cache, every layer on the GPU.
pub fn server_command(settings: &ServerSettings, port: u16, key: &str) -> Command {
    let mut command = Command::new(&settings.executable);
    command
        .args(["-m".into(), settings.model.to_string_lossy().to_string()])
        .args(["--host".to_string(), "127.0.0.1".to_string()])
        .args(["--port".to_string(), port.to_string()])
        .args(["-c".to_string(), settings.context_tokens.to_string()])
        .args(["-np".to_string(), "1".to_string()])
        .args(["-ngl".to_string(), "99".to_string()])
        .args(["-t".to_string(), settings.threads.to_string()])
        .args(["--flash-attn".to_string(), "off".to_string()])
        .args(["-b".to_string(), "2048".to_string()])
        .args(["-ub".to_string(), "512".to_string()])
        .args(["--cache-ram".to_string(), "0".to_string()])
        .env("LLAMA_API_KEY", key)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Asks the operating system for an unused loopback port.
pub fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener.local_addr().map(|address| address.port()).map_err(|error| error.to_string())
}

/// A running llama-server. Dropping the group kills it even if `stop` never runs.
pub struct Server {
    child: Child,
    port: u16,
    key: String,
    pub ready_seconds: f64,
}

impl Server {
    pub fn start(group: &ProcessGroup, settings: &ServerSettings) -> Result<Self, String> {
        let port = free_port()?;
        let key = random_key();
        let log = std::fs::File::create(&settings.log)
            .map_err(|error| format!("log file failed: {error}"))?;
        let errors = log.try_clone().map_err(|error| error.to_string())?;
        let mut command = server_command(settings, port, &key);
        command.stdout(Stdio::from(log)).stderr(Stdio::from(errors));
        let child = group.spawn(&mut command)?;
        let mut server = Self { child, port, key, ready_seconds: 0.0 };
        server.wait_ready()?;
        Ok(server)
    }

    fn wait_ready(&mut self) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("llama-server exited early: {status}"));
            }
            if check_auth(&self.base(), &self.key).map(|code| code == 200).unwrap_or(false) {
                self.ready_seconds = started.elapsed().as_secs_f64();
                return Ok(());
            }
            if started.elapsed() > READY_TIMEOUT {
                let _ = stop(&mut self.child, STOP_GRACE);
                return Err(format!("llama-server was not ready in {}s", READY_TIMEOUT.as_secs()));
            }
            std::thread::sleep(HEALTH_INTERVAL);
        }
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn stop(&mut self) -> Result<(), String> {
        stop(&mut self.child, STOP_GRACE)
    }
}

/// A key that never leaves the process except through the child's environment.
/// `RandomState` keys come from 128 bits of operating-system randomness, which is plenty
/// for a key that only guards a loopback port.
fn random_key() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..4)
        .map(|_| format!("{:016x}", RandomState::new().build_hasher().finish()))
        .collect()
}

/// The status code `/v1/models` answers with. In b10994 only `/health` is open without a
/// key; this endpoint returns 401 when the key is missing or wrong.
pub fn check_auth(base: &str, key: &str) -> Result<u16, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(format!("{base}/v1/models"));
    if !key.is_empty() {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    request
        .send()
        .map(|response| response.status().as_u16())
        .map_err(|error| format!("request failed: {error}"))
}

pub fn draft_body(prompt: &str, max_tokens: u32) -> String {
    let message = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
        "stream": true,
        "temperature": 0.2,
    });
    serde_json::to_string(&message).unwrap_or_default()
}

/// Reads server-sent events until the stream ends or the flag is set. Dropping the reader
/// closes the connection, which is what actually cancels the work on the server.
pub fn consume_stream<R: BufRead>(
    reader: R,
    cancel: &AtomicBool,
    on_token: &mut dyn FnMut(&str),
) -> Result<Stop, String> {
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Stop::Cancelled);
        }
        let line = line.map_err(|error| format!("stream failed: {error}"))?;
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        if payload.trim() == "[DONE]" {
            return Ok(Stop::Finished);
        }
        let parsed: serde_json::Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(token) = parsed["choices"][0]["delta"]["content"].as_str() {
            on_token(token);
        }
    }
    Ok(Stop::Finished)
}

/// Sends one draft request and streams the answer back.
pub fn stream_draft(
    base: &str,
    key: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<(Stop, String), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{base}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .body(draft_body(prompt, max_tokens))
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    if response.status().as_u16() != 200 {
        return Err(format!("server answered {}", response.status().as_u16()));
    }
    let mut text = String::new();
    let stop = consume_stream(BufReader::new(response), cancel, &mut |token| text.push_str(token))?;
    Ok((stop, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_command_keeps_the_key_out_of_the_arguments() {
        let settings = ServerSettings {
            executable: PathBuf::from("llama-server.exe"),
            model: PathBuf::from("model.gguf"),
            log: PathBuf::from("server.log"),
            context_tokens: 8192,
            threads: 2,
        };
        let command = server_command(&settings, 51234, "secret-key");
        let arguments: Vec<String> = command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect();
        assert!(arguments.contains(&"--cache-ram".to_string()));
        assert!(arguments.contains(&"0".to_string()));
        assert!(arguments.contains(&"127.0.0.1".to_string()));
        assert!(!arguments.iter().any(|value| value.contains("secret-key")));
        let environment: Vec<String> = command
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| format!("{}={}", name.to_string_lossy(), value.to_string_lossy())))
            .collect();
        assert!(environment.contains(&"LLAMA_API_KEY=secret-key".to_string()));
    }

    #[test]
    fn keys_are_long_and_do_not_repeat() {
        let first = random_key();
        let second = random_key();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn a_free_port_is_above_the_reserved_range() {
        let port = free_port().expect("port");
        assert!(port > 1024, "unexpected port {port}");
    }

    #[test]
    fn the_stream_stops_when_the_flag_is_set() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"가\"}}]}\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"나\"}}]}\n\
                    data: [DONE]\n";
        let cancel = AtomicBool::new(true);
        let mut seen = String::new();
        let stop = consume_stream(Cursor::new(body), &cancel, &mut |token| seen.push_str(token))
            .expect("stream");
        assert_eq!(stop, Stop::Cancelled);
        assert_eq!(seen, "");
    }

    #[test]
    fn the_stream_collects_tokens_until_done() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"가\"}}]}\n\
                    \n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"나\"}}]}\n\
                    data: [DONE]\n";
        let cancel = AtomicBool::new(false);
        let mut seen = String::new();
        let stop = consume_stream(Cursor::new(body), &cancel, &mut |token| seen.push_str(token))
            .expect("stream");
        assert_eq!(stop, Stop::Finished);
        assert_eq!(seen, "가나");
    }

    #[test]
    fn the_draft_body_asks_for_a_stream() {
        let body = draft_body("요약해 주세요", 300);
        assert!(body.contains("\"stream\":true"));
        assert!(body.contains("\"max_tokens\":300"));
        assert!(body.contains("요약해 주세요"));
    }
}
