//! Microphone and system-sound capture through WASAPI, delivered as 16 kHz mono PCM16.
//!
//! Loopback capture only accepts the device mix format, so both sources are captured in
//! that format and converted by [`crate::convert`].
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use wasapi::{initialize_mta, DeviceEnumerator, Direction, StreamMode};

use crate::convert::Converter;

pub const EVENT_TIMEOUT_MS: u32 = 1_000;
/// Silence is only filled in once the stream is this far behind the clock.
pub const SILENCE_THRESHOLD_SAMPLES: u64 = crate::wav::SAMPLE_RATE as u64 / 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Microphone,
    SystemSound,
}

impl Source {
    /// The device to open. System sound comes from the playback device, which WASAPI
    /// turns into loopback capture when a render device is initialised for capture.
    pub fn device_direction(self) -> Direction {
        match self {
            Source::Microphone => Direction::Capture,
            Source::SystemSound => Direction::Render,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub source: Source,
    pub device: Option<String>,
    pub format: Option<String>,
}

/// Flags the recorder uses to steer a running capture thread.
#[derive(Clone)]
pub struct Signals {
    pub stop: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub discontinuities: Arc<AtomicU32>,
    pub silence_samples: Arc<AtomicU64>,
}

impl Signals {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            discontinuities: Arc::new(AtomicU32::new(0)),
            silence_samples: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Default for Signals {
    fn default() -> Self {
        Self::new()
    }
}

fn describe_device(source: Source) -> Option<(String, String)> {
    initialize_mta().ok().ok()?;
    let enumerator = DeviceEnumerator::new().ok()?;
    let device = enumerator.get_default_device(&source.device_direction()).ok()?;
    let name = device.get_friendlyname().ok()?;
    let format = device.get_iaudioclient().ok()?.get_mixformat().ok()?;
    let kind = match format.get_subformat() {
        Ok(sample_type) => format!("{sample_type:?}"),
        Err(_) => "unknown".to_string(),
    };
    Some((
        name,
        format!(
            "{} Hz, {} ch, {} bit {}",
            format.get_samplespersec(),
            format.get_nchannels(),
            format.get_bitspersample(),
            kind
        ),
    ))
}

/// Default microphone and playback device, so the interface can show what it will record.
pub fn describe_sources() -> Vec<SourceInfo> {
    [Source::Microphone, Source::SystemSound]
        .into_iter()
        .map(|source| match describe_device(source) {
            Some((device, format)) => SourceInfo {
                source,
                device: Some(device),
                format: Some(format),
            },
            None => SourceInfo {
                source,
                device: None,
                format: None,
            },
        })
        .collect()
}

/// Samples to add so the recording keeps up with the clock. An idle playback device
/// delivers nothing at all, so silence has to be filled in to keep chunks aligned.
pub fn silence_to_insert(delivered: u64, elapsed_seconds: f64) -> u64 {
    let expected = (elapsed_seconds * crate::wav::SAMPLE_RATE as f64) as u64;
    let behind = expected.saturating_sub(delivered);
    if behind >= SILENCE_THRESHOLD_SAMPLES {
        behind
    } else {
        0
    }
}

/// Whole frames waiting in the queue; a partial frame stays for the next read.
pub fn drain_frames(queue: &mut VecDeque<u8>, frame_bytes: usize) -> Vec<u8> {
    let take = queue.len() / frame_bytes * frame_bytes;
    queue.drain(..take).collect()
}

/// Captures until the stop flag is set. While paused the device keeps running and the
/// samples are dropped, so the next chunk starts at the resume point.
pub fn capture(
    source: Source,
    samples: SyncSender<Vec<i16>>,
    signals: Signals,
) -> Result<(), String> {
    initialize_mta()
        .ok()
        .map_err(|error| format!("COM initialisation failed: {error}"))?;
    let enumerator = DeviceEnumerator::new().map_err(|error| error.to_string())?;
    let device = enumerator
        .get_default_device(&source.device_direction())
        .map_err(|error| format!("no default device: {error}"))?;
    let mut client = device
        .get_iaudioclient()
        .map_err(|error| format!("audio client failed: {error}"))?;
    let format = client
        .get_mixformat()
        .map_err(|error| format!("mix format unavailable: {error}"))?;
    let frame_bytes = format.get_blockalign() as usize;
    let mut converter = Converter::new(&format)?;
    let (_default_period, min_period) = client
        .get_device_period()
        .map_err(|error| error.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: false,
        buffer_duration_hns: min_period,
    };
    // Always initialise for capture: on a render device that is what enables loopback.
    client
        .initialize_client(&format, &Direction::Capture, &mode)
        .map_err(|error| format!("capture could not start: {error}"))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|error| error.to_string())?;
    let capture_client = client
        .get_audiocaptureclient()
        .map_err(|error| error.to_string())?;
    let mut queue: VecDeque<u8> = VecDeque::new();
    client.start_stream().map_err(|error| error.to_string())?;
    let result = capture_loop(
        &capture_client,
        &event,
        &mut queue,
        frame_bytes,
        &mut converter,
        &samples,
        &signals,
    );
    let _ = client.stop_stream();
    result
}

/// Reads every packet the device has queued. WASAPI hands out one packet per read, so a
/// single read per wake-up falls behind and the driver reports discontinuities.
fn drain_packets(
    capture_client: &wasapi::AudioCaptureClient,
    queue: &mut VecDeque<u8>,
    signals: &Signals,
) -> Result<(), String> {
    loop {
        let waiting = capture_client
            .get_next_packet_size()
            .map_err(|error| format!("packet size failed: {error}"))?;
        if waiting == Some(0) {
            return Ok(());
        }
        let info = capture_client
            .read_from_device_to_deque(queue)
            .map_err(|error| format!("capture read failed: {error}"))?;
        if info.flags.data_discontinuity {
            signals.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
        if waiting.is_none() {
            return Ok(());
        }
    }
}

fn capture_loop(
    capture_client: &wasapi::AudioCaptureClient,
    event: &wasapi::Handle,
    queue: &mut VecDeque<u8>,
    frame_bytes: usize,
    converter: &mut Converter,
    samples: &SyncSender<Vec<i16>>,
    signals: &Signals,
) -> Result<(), String> {
    let mut delivered = 0u64;
    let mut recording_since = Instant::now();
    while !signals.stop.load(Ordering::Relaxed) {
        drain_packets(capture_client, queue, signals)?;
        if signals.paused.load(Ordering::Relaxed) {
            queue.clear();
            delivered = 0;
            recording_since = Instant::now();
        } else {
            let bytes = drain_frames(queue, frame_bytes);
            let mut block = if bytes.is_empty() {
                Vec::new()
            } else {
                converter.push(&bytes)?
            };
            let silence = silence_to_insert(
                delivered + block.len() as u64,
                recording_since.elapsed().as_secs_f64(),
            );
            if silence > 0 {
                block.extend(std::iter::repeat_n(0i16, silence as usize));
                signals.silence_samples.fetch_add(silence, Ordering::Relaxed);
            }
            if !block.is_empty() {
                delivered += block.len() as u64;
                if samples.send(block).is_err() {
                    return Ok(());
                }
            }
        }
        // A timeout is normal: an idle playback device sends nothing until sound plays.
        let _ = event.wait_for_event(EVENT_TIMEOUT_MS);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::SAMPLE_RATE;

    #[test]
    fn sources_open_the_microphone_and_the_playback_device() {
        assert_eq!(Source::Microphone.device_direction(), Direction::Capture);
        assert_eq!(Source::SystemSound.device_direction(), Direction::Render);
    }

    #[test]
    fn silence_is_filled_only_when_the_stream_falls_behind() {
        assert_eq!(silence_to_insert(16_000, 1.0), 0);
        assert_eq!(silence_to_insert(16_000, 1.1), 0);
        assert_eq!(silence_to_insert(0, 1.0), 16_000);
        assert_eq!(silence_to_insert(8_000, 1.0), 8_000);
        assert_eq!(silence_to_insert(20_000, 1.0), 0);
    }

    #[test]
    fn only_whole_frames_leave_the_queue() {
        let mut queue: VecDeque<u8> = (0u8..10).collect();
        assert_eq!(drain_frames(&mut queue, 4), vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn describe_sources_lists_both_inputs() {
        let sources = describe_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].source, Source::Microphone);
        assert_eq!(sources[1].source, Source::SystemSound);
    }

    /// Captures for a few seconds and reports how many samples arrived.
    fn capture_briefly(source: Source, seconds: u64) -> (Result<(), String>, usize) {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
        let signals = Signals::new();
        let stop = Arc::clone(&signals.stop);
        let worker = std::thread::spawn(move || capture(source, sender, signals));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        let mut samples = 0usize;
        while std::time::Instant::now() < deadline {
            match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(block) => samples += block.len(),
                Err(_) => continue,
            }
        }
        stop.store(true, Ordering::Relaxed);
        (worker.join().expect("capture thread"), samples)
    }

    #[test]
    #[ignore = "needs an audio device; run with cargo test -- --ignored"]
    fn microphone_capture_delivers_samples() {
        let (result, samples) = capture_briefly(Source::Microphone, 4);
        assert!(result.is_ok(), "capture failed: {result:?}");
        assert!(
            samples >= SAMPLE_RATE as usize,
            "expected at least a second of samples, got {samples}"
        );
    }

    #[test]
    #[ignore = "needs an audio device; run with cargo test -- --ignored"]
    fn system_sound_capture_starts() {
        let (result, _samples) = capture_briefly(Source::SystemSound, 3);
        assert!(result.is_ok(), "loopback capture failed: {result:?}");
    }
}
