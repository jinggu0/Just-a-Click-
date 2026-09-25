//! Times a recording through the same API as the app so validation runs are repeatable.
//!
//! Usage: cargo run --example record_check -- <microphone|system_sound> <seconds> <directory>
//!        [--pause-at <seconds> --pause-for <seconds>]
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use app_lib::audio::Source;
use app_lib::recorder::Recorder;

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: record_check <microphone|system_sound> <seconds> <directory> \
                    [--pause-at <seconds> --pause-for <seconds>]"
            .to_string());
    }
    let source = match arguments[0].as_str() {
        "microphone" => Source::Microphone,
        "system_sound" => Source::SystemSound,
        other => return Err(format!("unknown source: {other}")),
    };
    let seconds: f64 = arguments[1].parse().map_err(|_| "seconds must be a number")?;
    let directory = PathBuf::from(&arguments[2]);
    let pause_at = flag(&arguments, "--pause-at");
    let pause_for = flag(&arguments, "--pause-for");

    let mut recorder = Recorder::new();
    let started = Instant::now();
    let status = recorder.start(source, directory)?;
    println!("started: {}", serde_json::to_string(&status).unwrap_or_default());
    let mut paused = false;
    while started.elapsed().as_secs_f64() < seconds {
        if let (Some(at), Some(duration)) = (pause_at, pause_for) {
            if !paused && started.elapsed().as_secs_f64() >= at {
                println!("pausing at {:.1}s for {duration:.1}s", started.elapsed().as_secs_f64());
                recorder.pause()?;
                sleep(Duration::from_secs_f64(duration));
                recorder.resume()?;
                paused = true;
                println!("resumed at {:.1}s", started.elapsed().as_secs_f64());
            }
        }
        sleep(Duration::from_secs(5));
        let status = recorder.status();
        println!(
            "{:.0}s phase {:?} chunks {} recorded {:.1}s silence {:.1}s discontinuities {}",
            started.elapsed().as_secs_f64(),
            status.phase,
            status.chunks,
            status.recorded_seconds,
            status.silence_seconds,
            status.discontinuities
        );
    }
    let status = recorder.stop()?;
    println!("stopped: {}", serde_json::to_string(&status).unwrap_or_default());
    Ok(())
}

fn flag(arguments: &[String], name: &str) -> Option<f64> {
    let index = arguments.iter().position(|value| value == name)?;
    arguments.get(index + 1)?.parse().ok()
}
