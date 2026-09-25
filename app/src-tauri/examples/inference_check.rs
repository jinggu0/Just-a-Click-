//! Runs the third validation item end to end and writes the result as JSON.
//!
//! Usage: cargo run --release --example inference_check -- <repository root> <chunk.wav> <out dir>
//!        cargo run --release --example inference_check -- <repository root> --hold <seconds>
//!
//! `--hold` starts the server and waits, so the orphan check can force-kill this harness.
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_lib::llm::{check_auth, stream_draft, Server, ServerSettings};
use app_lib::process::ProcessGroup;
use app_lib::stt::{output_file, run, Outcome, TranscribeSettings};

fn server_settings(root: &Path, log: PathBuf) -> ServerSettings {
    ServerSettings {
        executable: root.join("runtimes/b10994/vulkan/llama-server.exe"),
        model: root.join("models/Qwen3-8B-Q5_K_M.gguf"),
        log,
        context_tokens: 8192,
        threads: 2,
    }
}

fn transcribe_settings(root: &Path, chunk: PathBuf, output: PathBuf) -> TranscribeSettings {
    TranscribeSettings {
        executable: root.join("runtimes/whisper-b5130/blas/Release/whisper-cli.exe"),
        model: root.join("models/whisper/ggml-large-v3-turbo.bin"),
        chunk,
        output,
        threads: 8,
    }
}

/// The address another machine on the network would use, without sending anything.
fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let address = socket.local_addr().ok()?.ip();
    if address.is_loopback() {
        None
    } else {
        Some(address)
    }
}

fn private_mib(process_id: u32) -> Option<f64> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {process_id} -ErrorAction SilentlyContinue).PrivateMemorySize64"),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .map(|bytes| (bytes / 1_048_576.0 * 10.0).round() / 10.0)
}

fn hold(root: &Path, seconds: u64) -> Result<(), String> {
    let group = ProcessGroup::new()?;
    let settings = server_settings(root, root.join("artifacts/app-inference/hold-server.log"));
    let server = Server::start(&group, &settings)?;
    println!("held server pid {} for {seconds}s", server.process_id());
    std::thread::sleep(Duration::from_secs(seconds));
    Ok(())
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 2 {
        return Err("usage: inference_check <root> <chunk.wav> <out dir> | <root> --hold <seconds>".into());
    }
    let root = PathBuf::from(&arguments[0]);
    if arguments[1] == "--hold" {
        let seconds: u64 = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(120);
        return hold(&root, seconds);
    }
    let chunk = PathBuf::from(&arguments[1]);
    let out_dir = PathBuf::from(&arguments[2]);
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;

    let group = ProcessGroup::new()?;
    let settings = server_settings(&root, out_dir.join("server.log"));
    let mut server = Server::start(&group, &settings)?;
    let base = server.base();
    let key = server.key().to_string();
    let process_id = server.process_id();
    println!("ready in {:.1}s on port {}", server.ready_seconds, server.port());

    let no_key = check_auth(&base, "").unwrap_or(0);
    let wrong_key = check_auth(&base, "wrong-key").unwrap_or(0);
    let right_key = check_auth(&base, &key).unwrap_or(0);
    let lan_blocked = match lan_address() {
        Some(address) => {
            let target = SocketAddr::new(address, server.port());
            TcpStream::connect_timeout(&target, Duration::from_secs(2)).is_err()
        }
        None => false,
    };
    let memory_start = private_mib(process_id);

    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1_500));
        flag.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let (stop, partial) = stream_draft(&base, &key, "다음 강의 구간을 요약해 주세요.", 300, &cancel)?;
    let cancel_seconds = started.elapsed().as_secs_f64() - 1.5;
    println!("cancelled after {cancel_seconds:.2}s with {} characters", partial.chars().count());

    let sampler_id = process_id;
    let sample = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        private_mib(sampler_id)
    });
    let quiet = AtomicBool::new(false);
    let (follow_up, answer) = stream_draft(&base, &key, "한 문장으로 답해 주세요.", 64, &quiet)?;
    let memory_during = sample.join().unwrap_or(None);

    let transcript = run(
        &group,
        &transcribe_settings(&root, chunk.clone(), out_dir.join("chunk")),
        &AtomicBool::new(false),
    )?;
    let transcript_text = match &transcript {
        Outcome::Text(text) => text.clone(),
        Outcome::Cancelled => String::new(),
    };

    let stt_cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stt_cancel);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1_000));
        flag.store(true, Ordering::Relaxed);
    });
    let cancel_settings = transcribe_settings(&root, chunk, out_dir.join("cancelled"));
    let stt_outcome = run(&group, &cancel_settings, &stt_cancel)?;
    let leftover = output_file(&cancel_settings).exists();

    let killed = Command::new("taskkill")
        .args(["/PID", &process_id.to_string(), "/F"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    std::thread::sleep(Duration::from_secs(1));
    let alive_after_kill = server.alive();
    let mut restarted = Server::start(&group, &server_settings(&root, out_dir.join("server-2.log")))?;
    let restart_seconds = restarted.ready_seconds;
    let memory_after_restart = private_mib(restarted.process_id());
    restarted.stop()?;
    std::thread::sleep(Duration::from_millis(500));
    let stopped = !restarted.alive();

    let report = serde_json::json!({
        "ready_seconds": server.ready_seconds,
        "auth": {"no_key": no_key, "wrong_key": wrong_key, "right_key": right_key},
        "lan_blocked": lan_blocked,
        "draft_cancel": {"stop": format!("{stop:?}"), "seconds": (cancel_seconds * 100.0).round() / 100.0,
                          "characters": partial.chars().count()},
        "draft_after_cancel": {"stop": format!("{follow_up:?}"), "characters": answer.chars().count()},
        "transcribe": {"characters": transcript_text.chars().count(),
                        "sample": transcript_text.chars().take(40).collect::<String>()},
        "transcribe_cancel": {"outcome": format!("{stt_outcome:?}"), "leftover_file": leftover},
        "crash_recovery": {"killed": killed, "alive_after_kill": alive_after_kill,
                            "restart_seconds": restart_seconds, "stopped_cleanly": stopped},
        "memory_mib": {"start": memory_start, "during_request": memory_during, "after_restart": memory_after_restart},
    });
    let path = out_dir.join("inference-check.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    println!("written to {}", path.display());
    Ok(())
}
