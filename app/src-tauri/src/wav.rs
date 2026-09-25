//! Minimal 16-bit PCM WAV writer so the recorder controls the exact chunk format.
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;
pub const BITS_PER_SAMPLE: u16 = 16;
pub const HEADER_BYTES: usize = 44;

/// RIFF header for mono 16-bit PCM; sizes are rewritten when the chunk is closed.
pub fn header(data_bytes: u32) -> [u8; HEADER_BYTES] {
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
    let byte_rate = SAMPLE_RATE * block_align as u32;
    let mut header = [0u8; HEADER_BYTES];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

pub struct WavWriter {
    file: BufWriter<File>,
    samples: u32,
}

impl WavWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(&header(0))?;
        Ok(Self { file, samples: 0 })
    }

    pub fn write(&mut self, samples: &[i16]) -> std::io::Result<()> {
        for sample in samples {
            self.file.write_all(&sample.to_le_bytes())?;
        }
        self.samples += samples.len() as u32;
        Ok(())
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Writes the real sizes into the header; the length is only known at the end.
    pub fn finish(mut self) -> std::io::Result<u32> {
        let samples = self.samples;
        self.file.flush()?;
        let mut file = self
            .file
            .into_inner()
            .map_err(|error| error.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(samples * 2))?;
        file.flush()?;
        Ok(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_describes_mono_16khz_pcm() {
        let header = header(480_000 * 2);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([header[20], header[21]]), 1);
        assert_eq!(u16::from_le_bytes([header[22], header[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([header[24], header[25], header[26], header[27]]),
            16_000
        );
        assert_eq!(u16::from_le_bytes([header[34], header[35]]), 16);
        assert_eq!(
            u32::from_le_bytes([header[40], header[41], header[42], header[43]]),
            960_000
        );
    }

    #[test]
    fn finished_file_has_header_and_samples() {
        let directory = std::env::temp_dir().join(format!("jac-wav-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("chunk.wav");
        let mut writer = WavWriter::create(&path).unwrap();
        writer.write(&[1, -1, 32_767, -32_768]).unwrap();
        assert_eq!(writer.samples(), 4);
        assert_eq!(writer.finish().unwrap(), 4);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), HEADER_BYTES + 8);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            8
        );
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
