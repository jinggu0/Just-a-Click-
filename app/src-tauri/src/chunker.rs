//! Splits the capture stream into fixed-length chunk files the transcriber can consume.
use std::path::{Path, PathBuf};

use crate::wav::{WavWriter, SAMPLE_RATE};

pub const CHUNK_SECONDS: u32 = 30;
pub const CHUNK_SAMPLES: u32 = SAMPLE_RATE * CHUNK_SECONDS;

pub struct Chunker {
    directory: PathBuf,
    writer: Option<WavWriter>,
    written: Vec<PathBuf>,
    open_path: Option<PathBuf>,
    samples: u64,
}

impl Chunker {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            writer: None,
            written: Vec::new(),
            open_path: None,
            samples: 0,
        }
    }

    pub fn chunk_path(directory: &Path, index: usize) -> PathBuf {
        directory.join(format!("chunk-{:04}.wav", index))
    }

    /// Appends samples, closing a chunk file whenever it reaches the chunk length.
    pub fn push(&mut self, samples: &[i16]) -> std::io::Result<()> {
        let mut rest = samples;
        while !rest.is_empty() {
            if self.writer.is_none() {
                std::fs::create_dir_all(&self.directory)?;
                let path = Self::chunk_path(&self.directory, self.written.len() + 1);
                self.writer = Some(WavWriter::create(&path)?);
                self.open_path = Some(path);
            }
            let writer = self.writer.as_mut().expect("writer is open");
            let space = (CHUNK_SAMPLES - writer.samples()) as usize;
            let take = space.min(rest.len());
            writer.write(&rest[..take])?;
            self.samples += take as u64;
            rest = &rest[take..];
            if writer.samples() >= CHUNK_SAMPLES {
                self.close()?;
            }
        }
        Ok(())
    }

    /// Closes the open chunk, if any, so the file on disk is complete.
    pub fn close(&mut self) -> std::io::Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.finish()?;
            if let Some(path) = self.open_path.take() {
                self.written.push(path);
            }
        }
        Ok(())
    }

    pub fn chunks(&self) -> &[PathBuf] {
        &self.written
    }

    pub fn open_chunk(&self) -> Option<&Path> {
        self.open_path.as_deref()
    }

    pub fn total_samples(&self) -> u64 {
        self.samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("jac-chunker-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn one_full_chunk_closes_and_names_the_file() {
        let directory = temp_dir("full");
        let mut chunker = Chunker::new(directory.clone());
        chunker.push(&vec![7i16; CHUNK_SAMPLES as usize]).unwrap();
        assert_eq!(chunker.chunks().len(), 1);
        assert_eq!(chunker.chunks()[0], directory.join("chunk-0001.wav"));
        assert!(chunker.open_chunk().is_none());
        let bytes = std::fs::read(&chunker.chunks()[0]).unwrap();
        assert_eq!(bytes.len(), 44 + CHUNK_SAMPLES as usize * 2);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn extra_samples_start_the_next_chunk() {
        let directory = temp_dir("extra");
        let mut chunker = Chunker::new(directory.clone());
        chunker.push(&vec![1i16; CHUNK_SAMPLES as usize + 5]).unwrap();
        assert_eq!(chunker.chunks().len(), 1);
        assert_eq!(chunker.open_chunk(), Some(directory.join("chunk-0002.wav").as_path()));
        chunker.close().unwrap();
        assert_eq!(chunker.chunks().len(), 2);
        let second = std::fs::read(&chunker.chunks()[1]).unwrap();
        assert_eq!(second.len(), 44 + 10);
        assert_eq!(chunker.total_samples(), CHUNK_SAMPLES as u64 + 5);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn samples_split_across_pushes_fill_one_chunk() {
        let directory = temp_dir("split");
        let mut chunker = Chunker::new(directory.clone());
        for _ in 0..3 {
            chunker.push(&vec![2i16; (CHUNK_SAMPLES / 3) as usize]).unwrap();
        }
        assert_eq!(chunker.chunks().len(), 1);
        assert_eq!(chunker.total_samples(), CHUNK_SAMPLES as u64);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
