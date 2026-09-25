//! Converts captured device audio to the 16 kHz mono PCM16 the transcriber expects.
//!
//! WASAPI loopback only accepts the device mix format, so the app downmixes to mono and
//! resamples itself with a windowed-sinc filter that also removes the frequencies above
//! the 8 kHz output Nyquist.
use std::f32::consts::PI;

use wasapi::{SampleType, WaveFormat};

use crate::wav::SAMPLE_RATE;

/// Taps on each side of the interpolation point. 32 keeps speech clean and the cost low.
pub const FILTER_HALF: usize = 32;

pub struct Converter {
    channels: usize,
    bytes_per_sample: usize,
    float_samples: bool,
    step: f64,
    cutoff: f32,
    history: Vec<f32>,
    position: f64,
}

impl Converter {
    pub fn new(format: &WaveFormat) -> Result<Self, String> {
        let channels = format.get_nchannels() as usize;
        let bits = format.get_bitspersample();
        let float_samples = match format.get_subformat() {
            Ok(SampleType::Float) => true,
            Ok(SampleType::Int) => false,
            Err(error) => return Err(format!("unknown sample format: {error}")),
        };
        if channels == 0 {
            return Err("device reports no channels".to_string());
        }
        if !matches!((float_samples, bits), (true, 32) | (false, 16) | (false, 32)) {
            return Err(format!(
                "unsupported device format: {bits} bit {}",
                if float_samples { "float" } else { "int" }
            ));
        }
        let source_rate = format.get_samplespersec();
        if source_rate < SAMPLE_RATE {
            return Err(format!("device rate {source_rate} Hz is below 16 kHz"));
        }
        Ok(Self {
            channels,
            bytes_per_sample: (bits / 8) as usize,
            float_samples,
            step: source_rate as f64 / SAMPLE_RATE as f64,
            cutoff: SAMPLE_RATE as f32 / source_rate as f32,
            history: vec![0.0; FILTER_HALF],
            position: FILTER_HALF as f64,
        })
    }

    /// Interleaved device bytes in, 16 kHz mono PCM16 samples out.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<i16>, String> {
        let frame_bytes = self.channels * self.bytes_per_sample;
        if frame_bytes == 0 || bytes.len() % frame_bytes != 0 {
            return Err("capture buffer is not a whole number of frames".to_string());
        }
        for frame in bytes.chunks_exact(frame_bytes) {
            let mut total = 0.0f32;
            for sample in frame.chunks_exact(self.bytes_per_sample) {
                total += self.decode(sample);
            }
            self.history.push(total / self.channels as f32);
        }
        Ok(self.resample())
    }

    fn decode(&self, sample: &[u8]) -> f32 {
        match (self.float_samples, sample.len()) {
            (true, 4) => f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]),
            (false, 2) => i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32_768.0,
            (false, 4) => {
                i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32
                    / 2_147_483_648.0
            }
            _ => 0.0,
        }
    }

    /// Windowed-sinc interpolation at the output rate; the kernel doubles as the anti-alias filter.
    fn resample(&mut self) -> Vec<i16> {
        let mut samples = Vec::new();
        let last_usable = self.history.len().saturating_sub(FILTER_HALF + 1);
        while (self.position as usize) < last_usable {
            let center = self.position.floor() as usize;
            let offset = (self.position - center as f64) as f32;
            let mut value = 0.0f32;
            for tap in 0..=(2 * FILTER_HALF) {
                let index = center + tap - FILTER_HALF;
                let distance = tap as f32 - FILTER_HALF as f32 - offset;
                value += self.history[index] * kernel(distance, self.cutoff);
            }
            samples.push((value.clamp(-1.0, 1.0) * 32_767.0).round() as i16);
            self.position += self.step;
        }
        let consumed = (self.position as usize).saturating_sub(FILTER_HALF);
        if consumed > 0 {
            self.history.drain(..consumed);
            self.position -= consumed as f64;
        }
        samples
    }
}

/// Sinc low-pass at `cutoff` (relative to the input rate) times a Blackman window.
fn kernel(distance: f32, cutoff: f32) -> f32 {
    let half = FILTER_HALF as f32;
    if distance.abs() > half {
        return 0.0;
    }
    let sinc = if distance.abs() < 1e-6 {
        cutoff
    } else {
        (PI * cutoff * distance).sin() / (PI * distance)
    };
    let phase = PI * (distance + half) / half;
    let window = 0.42 - 0.5 * (phase).cos() + 0.08 * (2.0 * phase).cos();
    sinc * window
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(rate: u32, channels: u16, bits: u16, float_samples: bool) -> WaveFormat {
        WaveFormat::new(
            bits as usize,
            bits as usize,
            if float_samples {
                &SampleType::Float
            } else {
                &SampleType::Int
            },
            rate as usize,
            channels as usize,
            None,
        )
    }

    fn stereo_float_bytes(frames: usize, left: impl Fn(usize) -> f32, right: impl Fn(usize) -> f32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(frames * 8);
        for frame in 0..frames {
            bytes.extend_from_slice(&left(frame).to_le_bytes());
            bytes.extend_from_slice(&right(frame).to_le_bytes());
        }
        bytes
    }

    fn peak(samples: &[i16]) -> f32 {
        samples.iter().map(|value| (*value as f32 / 32_767.0).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn rejects_formats_the_converter_cannot_handle() {
        assert!(Converter::new(&format(8_000, 1, 16, false)).is_err());
        assert!(Converter::new(&format(48_000, 2, 24, false)).is_err());
        assert!(Converter::new(&format(48_000, 2, 32, true)).is_ok());
    }

    #[test]
    fn stereo_is_downmixed_to_silence_when_channels_cancel() {
        let mut converter = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        let bytes = stereo_float_bytes(4_800, |_| 0.5, |_| -0.5);
        let samples = converter.push(&bytes).unwrap();
        assert!(samples.len() >= 1_500 && samples.len() <= 1_600, "got {}", samples.len());
        assert_eq!(peak(&samples), 0.0);
    }

    #[test]
    fn speech_range_tone_keeps_its_amplitude() {
        let mut converter = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        let tone = |frame: usize| 0.5 * (2.0 * PI * 1_000.0 * frame as f32 / 48_000.0).sin();
        let bytes = stereo_float_bytes(48_000, tone, tone);
        let samples = converter.push(&bytes).unwrap();
        assert!(samples.len() >= 15_900 && samples.len() <= 16_010, "got {}", samples.len());
        let measured = peak(&samples[100..]);
        assert!(measured > 0.45 && measured < 0.55, "peak {measured}");
    }

    #[test]
    fn tone_above_the_output_nyquist_is_filtered_out() {
        let mut converter = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        let tone = |frame: usize| 0.5 * (2.0 * PI * 12_000.0 * frame as f32 / 48_000.0).sin();
        let bytes = stereo_float_bytes(48_000, tone, tone);
        let samples = converter.push(&bytes).unwrap();
        let measured = peak(&samples[100..]);
        assert!(measured < 0.05, "aliased peak {measured}");
    }

    #[test]
    fn splitting_a_buffer_keeps_the_sample_count() {
        let mut whole = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        let mut halves = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        let tone = |frame: usize| 0.25 * (2.0 * PI * 440.0 * frame as f32 / 48_000.0).sin();
        let bytes = stereo_float_bytes(9_600, tone, tone);
        let one = whole.push(&bytes).unwrap().len();
        let split = halves.push(&bytes[..bytes.len() / 2]).unwrap().len()
            + halves.push(&bytes[bytes.len() / 2..]).unwrap().len();
        assert!((one as i64 - split as i64).abs() <= 1, "{one} vs {split}");
    }

    #[test]
    fn a_16khz_mono_device_passes_through_unchanged_in_length() {
        let mut converter = Converter::new(&format(16_000, 1, 16, false)).unwrap();
        let mut bytes = Vec::new();
        for frame in 0..16_000i32 {
            let value = ((frame % 100) as i16 - 50) * 100;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let samples = converter.push(&bytes).unwrap();
        assert!(samples.len() >= 15_900 && samples.len() <= 16_000, "got {}", samples.len());
    }

    #[test]
    fn rejects_partial_frames() {
        let mut converter = Converter::new(&format(48_000, 2, 32, true)).unwrap();
        assert!(converter.push(&[0u8; 7]).is_err());
    }
}
