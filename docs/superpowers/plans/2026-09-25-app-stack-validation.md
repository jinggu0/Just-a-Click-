# 앱 기술 검증 1차 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tauri 2 앱 골격을 저장소에 만들고, 마이크와 시스템 소리를 16kHz 모노 PCM16 30초 조각으로 녹음하며 일시정지·재개와 절전 방지가 동작하는지 확인한다.

**Architecture:** WASAPI 캡처 스레드가 장치 믹스 형식으로 받은 프레임을 변환 모듈에서 16kHz 모노로 바꾸고, 채널로 넘겨받은 쓰기 스레드가 30초마다 WAV 조각을 닫는다. 상태는 뮤텍스로 공유하고 Tauri 명령이 읽는다. 오디오 장치가 필요한 검사는 `#[ignore]`로 분리해 기본 테스트는 장치 없이 돈다.

**Tech Stack:** Rust 1.98(MSVC), Tauri 2.11, React 19 + TypeScript + Vite, `wasapi` 0.24(MIT), `windows` 0.62(MIT OR Apache-2.0), 의존성 없는 WAV 기록과 윈도우드 싱크 리샘플러.

## Global Constraints

- 설계 문서: `docs/superpowers/specs/2026-09-23-app-stack-validation-design.md`. 1차 범위는 결정 0007 3절의 ①②이며 ③④⑤는 2차다.
- 앱은 저장소 `app/`에 두고 A 단계에서 계속 키운다. `target/`, `node_modules/`, `dist/`는 추적하지 않고 `package-lock.json`과 `Cargo.lock`은 추적한다.
- 조각 형식은 16,000Hz·모노·16비트 PCM이며 조각 길이는 30초(480,000샘플)다. 파일명은 `chunk-0001.wav`부터 4자리다.
- 녹음 조각은 `artifacts/app-recordings/`에만 저장하고 커밋하지 않는다. 사용자 음성은 저장소에 넣지 않는다.
- 절전 방지는 `SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED|ES_DISPLAY_REQUIRED)`로 요청하고 정지 시 `ES_CONTINUOUS`로 되돌린다. 전원 설정·드라이버는 바꾸지 않는다.
- 기본 테스트는 오디오 장치 없이 통과해야 한다. 장치가 필요한 검사는 `#[ignore]`를 붙이고 `cargo test -- --ignored`로 돌린다.
- 문서는 한국어, 코드·주석은 영어로 쓴다. 각 Task는 검증 후 해당 경로만 stage하여 main에 커밋하고, 메시지 끝에 `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`를 붙인다. push하지 않는다.

## 사전 확인 사실 (2026-09-25, 계획 작성 중 직접 확인)

- rustup 공식 배포본(`static.rust-lang.org`)의 SHA-256이 공개 해시와 일치했고, 설치 결과는 rustc 1.98.1·cargo 1.98.1(stable-x86_64-pc-windows-msvc)이다. 설치 위치는 `%USERPROFILE%\.cargo`이며 PATH에 추가된다.
- `npm create tauri-app@latest -- <이름> --manager npm --template react-ts --identifier <식별자> --tauri-version 2 --yes`로 골격이 만들어진다. 결과는 Tauri 2.11, React 19, Vite 8, TypeScript 6이다.
- 기본 장치 믹스 형식은 마이크·스피커 모두 48,000Hz·2채널·32비트 float였다. 그래서 다운믹스와 48→16kHz 리샘플링이 필요하다.
- WASAPI 루프백은 **재생 장치를 열고 방향을 `Direction::Capture`로 초기화**해야 한다. 재생 방향으로 초기화하면 `AUDCLNT_E_UNSUPPORTED_FORMAT`(0x88890003)이 난다.
- 루프백은 16kHz 모노 요청(자동 변환)도 거부했다. 믹스 형식으로 캡처하고 앱에서 변환해야 한다.
- 재생 중 소리가 없으면 루프백은 이벤트를 주지 않는다. 무음 구간을 0으로 채워야 녹음 시간축이 벽시계와 맞는다.
- 시제품에서 단위 테스트 22개와 장치 테스트 3개(마이크 캡처, 루프백 시작, 마이크 녹음의 조각 파일 생성)가 통과했고 `tsc --noEmit`도 통과했다. `npm run tauri dev`로 창이 떴고 WebView2 프로세스가 함께 떴다.
- 이 기기 조건: Node.js 22.15.0, npm 10.9.2, WebView2 런타임 153.0.4234.48, MSVC 14.43.34808, Windows SDK 10.0.22621.0.

## File Structure

| 경로 | 구분 | 책임 |
| --- | --- | --- |
| `app/package.json`, `app/vite.config.ts`, `app/tsconfig*.json`, `app/index.html` | 생성(골격) | 프런트엔드 빌드 설정 |
| `app/src/App.tsx`, `app/src/main.tsx`, `app/src/App.css` | 생성 | 검증 화면 |
| `app/src-tauri/Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/build.rs` | 생성(골격) | Rust 빌드·앱 설정 |
| `app/src-tauri/src/lib.rs` | 생성 | Tauri 명령과 상태 등록 |
| `app/src-tauri/src/audio.rs` | 생성 | 장치 조회, WASAPI 캡처 스레드, 무음 보정 |
| `app/src-tauri/src/convert.rs` | 생성 | 다운믹스, 48→16kHz 리샘플, PCM16 변환 |
| `app/src-tauri/src/chunker.rs` | 생성 | 30초 조각 경계와 파일 이름 |
| `app/src-tauri/src/wav.rs` | 생성 | PCM16 WAV 헤더·기록 |
| `app/src-tauri/src/power.rs` | 생성 | 절전·화면 꺼짐 방지 가드 |
| `docs/validation/<날짜>-app-stack.md` | 검증 후 생성 | 검증 보고서 |
| `docs/decisions/0007-app-stack.md` | 검증 후 수정 | 라이브러리 버전·라이선스와 판정 기록 |

`<날짜>`는 검증을 실행한 날짜다.

---

### Task 1: 툴체인과 앱 골격

**Files:**
- Create: `app/` 전체 골격(create-tauri-app 산출물)
- Modify: `app/src-tauri/tauri.conf.json`, `app/package.json`, `app/src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: 없음.
- Produces: `npm run tauri dev`·`npm run tauri build`로 실행 가능한 앱 골격, `cargo test`가 도는 Rust 크레이트, 의존성 `wasapi`·`windows`.

- [ ] **Step 1: Rust 툴체인 설치**

Run:

```bash
cd "$(cygpath -u "$TEMP")" && curl -sSL --retry 3 -o rustup-init.exe https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe && curl -sSL https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe.sha256 | cut -d' ' -f1 > rustup-init.sha256 && python -c "import hashlib,pathlib; print(hashlib.sha256(pathlib.Path('rustup-init.exe').read_bytes()).hexdigest() == pathlib.Path('rustup-init.sha256').read_text().strip())"
```

Expected: `True`(해시 일치). 일치하지 않으면 설치하지 않고 중단한다.

Run: `"$(cygpath -u "$TEMP")/rustup-init.exe" -y --default-toolchain stable --profile default`
Expected: `stable-x86_64-pc-windows-msvc installed - rustc 1.98.1` 형식의 줄과 `Rust is installed now.`

Run: `export PATH="$PATH:$HOME/.cargo/bin" && rustc --version && cargo --version`
Expected: `rustc 1.98.1`, `cargo 1.98.1`. 이후 모든 명령에서 이 PATH를 쓴다.

- [ ] **Step 2: 앱 골격 생성**

Run: `npm create tauri-app@latest -- app --manager npm --template react-ts --identifier dev.justaclick.app --tauri-version 2 --yes`
Expected: `app/` 폴더가 만들어지고 `app/src-tauri/Cargo.toml`, `app/src/App.tsx`, `app/src-tauri/tauri.conf.json`이 있다.

Run: `cd app && npm install`
Expected: `found 0 vulnerabilities`로 끝나고 `app/package-lock.json`이 만들어진다.

- [ ] **Step 3: 앱 이름·창 제목·의존성 설정**

`app/src-tauri/tauri.conf.json`에서 다음 두 곳을 바꾼다.

```json
  "productName": "Just a Click",
```

```json
      {
        "title": "딸깍! 녹음 검증",
        "width": 900,
        "height": 700
      }
```

`app/package.json`의 `scripts`에 타입 검사를 추가한다.

```json
    "typecheck": "tsc --noEmit",
```

`app/src-tauri/Cargo.toml`의 `serde_json = "1"` 다음 줄에 의존성을 추가한다.

```toml
wasapi = "0.24"
windows = { version = "0.62", features = ["Win32_System_Power"] }
```

- [ ] **Step 4: 빌드와 실행 확인**

Run: `cd app/src-tauri && cargo test`
Expected: 컴파일이 성공하고 `test result: ok. 0 passed`(골격에는 아직 테스트가 없다). 첫 빌드는 2~3분 걸린다.

Run: `cd app && npm run typecheck`
Expected: 출력 없이 종료 코드 0.

Run: `cd app && npm run tauri dev` (백그라운드로 실행하고 90초 뒤 확인)
Expected: 로그에 `Running \`target\debug\app.exe\``가 있고, `Get-Process app` 결과에 창 제목이 보인다. 확인 후 프로세스를 종료한다.

- [ ] **Step 5: 커밋**

```bash
git add app
git commit -m "feat: add the Tauri 2 app skeleton for validation" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

Run: `git status --short`
Expected: `app/target`, `app/node_modules`, `app/dist`가 보이지 않는다(골격의 `.gitignore`가 제외한다).

---

### Task 2: WAV 기록과 30초 조각

**Files:**
- Create: `app/src-tauri/src/wav.rs`, `app/src-tauri/src/chunker.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 없음.
- Produces:
  - `wav::SAMPLE_RATE`(16_000), `wav::CHANNELS`(1), `wav::BITS_PER_SAMPLE`(16), `wav::HEADER_BYTES`(44)
  - `wav::header(data_bytes: u32) -> [u8; 44]`, `wav::WavWriter::create(&Path)`, `write(&[i16])`, `samples() -> u32`, `finish() -> std::io::Result<u32>`
  - `chunker::CHUNK_SECONDS`(30), `chunker::CHUNK_SAMPLES`(480_000)
  - `chunker::Chunker::new(PathBuf)`, `chunk_path(&Path, usize) -> PathBuf`, `push(&[i16])`, `close()`, `chunks() -> &[PathBuf]`, `open_chunk() -> Option<&Path>`, `total_samples() -> u64`

- [ ] **Step 1: 모듈 등록**

`app/src-tauri/src/lib.rs` 첫 줄(`// Learn more about Tauri commands…`) 앞에 모듈 선언을 넣는다.

```rust
mod chunker;
mod wav;
```

- [ ] **Step 2: WAV 기록 작성(테스트 포함)**

`app/src-tauri/src/wav.rs`:

```rust
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
```

Run: `cd app/src-tauri && cargo test wav`
Expected: `test result: ok. 2 passed`

- [ ] **Step 3: 조각 나누기 작성(테스트 포함)**

`app/src-tauri/src/chunker.rs`:

```rust
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
```

Run: `cd app/src-tauri && cargo test`
Expected: `test result: ok. 5 passed`

- [ ] **Step 4: 커밋**

```bash
git add app/src-tauri/src/wav.rs app/src-tauri/src/chunker.rs app/src-tauri/src/lib.rs
git commit -m "feat: write 30-second PCM16 chunk files" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: 장치 형식 변환

**Files:**
- Create: `app/src-tauri/src/convert.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `wav::SAMPLE_RATE`.
- Produces: `convert::FILTER_HALF`(32), `convert::Converter::new(&WaveFormat) -> Result<Self, String>`, `Converter::push(&[u8]) -> Result<Vec<i16>, String>`.

- [ ] **Step 1: 모듈 등록**

`app/src-tauri/src/lib.rs`의 `mod chunker;` 다음 줄에 추가한다.

```rust
mod convert;
```

- [ ] **Step 2: 변환 모듈 작성(테스트 포함)**

`app/src-tauri/src/convert.rs`:

```rust
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
```

Run: `cd app/src-tauri && cargo test convert`
Expected: `test result: ok. 7 passed`. 1kHz 정현파의 진폭이 유지되고 12kHz 정현파는 5% 아래로 줄어드는지 확인하는 테스트가 포함된다.

Run: `cd app/src-tauri && cargo test`
Expected: `test result: ok. 12 passed`

- [ ] **Step 3: 커밋**

```bash
git add app/src-tauri/src/convert.rs app/src-tauri/src/lib.rs
git commit -m "feat: convert device audio to 16 kHz mono PCM16" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: 절전 방지

**Files:**
- Create: `app/src-tauri/src/power.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 없음.
- Produces: `power::RECORDING_STATE`(0x8000_0003), `power::KeepAwake::new()`, `power::KeepAwake::with(fn(u32) -> u32)`. 값이 살아 있는 동안 절전·화면 꺼짐을 막고 `Drop`에서 되돌린다.

- [ ] **Step 1: 모듈 등록**

`app/src-tauri/src/lib.rs`의 `mod convert;` 다음 줄에 추가한다.

```rust
mod power;
```

- [ ] **Step 2: 절전 가드 작성(테스트 포함)**

`app/src-tauri/src/power.rs`:

```rust
//! Keeps Windows awake while recording. System power settings stay unchanged.
pub const ES_CONTINUOUS: u32 = 0x8000_0000;
pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
pub const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;
pub const RECORDING_STATE: u32 = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED;

/// Holds the sleep request for as long as it lives; dropping it restores normal sleep.
pub struct KeepAwake {
    set_state: fn(u32) -> u32,
}

impl KeepAwake {
    pub fn new() -> Self {
        Self::with(set_thread_execution_state)
    }

    pub fn with(set_state: fn(u32) -> u32) -> Self {
        set_state(RECORDING_STATE);
        Self { set_state }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        (self.set_state)(ES_CONTINUOUS);
    }
}

#[cfg(windows)]
fn set_thread_execution_state(flags: u32) -> u32 {
    use windows::Win32::System::Power::{SetThreadExecutionState, EXECUTION_STATE};
    unsafe { SetThreadExecutionState(EXECUTION_STATE(flags)).0 }
}

#[cfg(not(windows))]
fn set_thread_execution_state(_flags: u32) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static CALLS: AtomicU32 = AtomicU32::new(0);
    static LAST: AtomicU32 = AtomicU32::new(0);

    fn record(flags: u32) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        LAST.store(flags, Ordering::SeqCst);
        0
    }

    #[test]
    fn requests_sleep_block_and_restores_on_drop() {
        CALLS.store(0, Ordering::SeqCst);
        {
            let _guard = KeepAwake::with(record);
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
            assert_eq!(LAST.load(Ordering::SeqCst), 0x8000_0003);
        }
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        assert_eq!(LAST.load(Ordering::SeqCst), ES_CONTINUOUS);
    }

    #[test]
    fn real_call_returns_previous_state() {
        let guard = KeepAwake::new();
        drop(guard);
    }
}
```

Run: `cd app/src-tauri && cargo test power`
Expected: `test result: ok. 2 passed`

- [ ] **Step 3: 커밋**

```bash
git add app/src-tauri/src/power.rs app/src-tauri/src/lib.rs
git commit -m "feat: block sleep and display-off while recording" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: WASAPI 캡처

**Files:**
- Create: `app/src-tauri/src/audio.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `convert::Converter`, `wav::SAMPLE_RATE`.
- Produces:
  - `audio::Source`(`Microphone`·`SystemSound`, serde `snake_case`), `Source::device_direction() -> Direction`
  - `audio::SourceInfo { source, device: Option<String>, format: Option<String> }`, `audio::describe_sources() -> Vec<SourceInfo>`
  - `audio::Signals { stop, paused, discontinuities, silence_samples }`와 `Signals::new()`
  - `audio::silence_to_insert(delivered: u64, elapsed_seconds: f64) -> u64`, `audio::drain_frames(&mut VecDeque<u8>, usize) -> Vec<u8>`
  - `audio::capture(Source, SyncSender<Vec<i16>>, Signals) -> Result<(), String>`

- [ ] **Step 1: 모듈 등록**

`app/src-tauri/src/lib.rs`의 첫 모듈 선언 앞에 추가해 알파벳 순서를 유지한다.

```rust
mod audio;
```

- [ ] **Step 2: 캡처 모듈 작성(테스트 포함)**

`app/src-tauri/src/audio.rs`:

```rust
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
        let info = capture_client
            .read_from_device_to_deque(queue)
            .map_err(|error| format!("capture read failed: {error}"))?;
        if info.flags.data_discontinuity {
            signals.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
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
```

Run: `cd app/src-tauri && cargo test`
Expected: `test result: ok. 18 passed; 0 failed; 2 ignored`

- [ ] **Step 3: 실제 장치로 확인**

Run: `cd app/src-tauri && cargo test -- --ignored`
Expected: `microphone_capture_delivers_samples`와 `system_sound_capture_starts`가 통과한다. 마이크가 없거나 사용 중이면 실패하므로, 실패 시 오류 문구를 기록하고 장치 상태를 확인한다.

- [ ] **Step 4: 커밋**

```bash
git add app/src-tauri/src/audio.rs app/src-tauri/src/lib.rs
git commit -m "feat: capture the microphone and system sound through WASAPI" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: 녹음기와 화면

**Files:**
- Create: `app/src-tauri/src/recorder.rs`
- Modify: `app/src-tauri/src/lib.rs`, `app/src/App.tsx`

**Interfaces:**
- Consumes: Task 2~5의 모듈.
- Produces:
  - `recorder::Phase`(`idle`·`recording`·`paused`·`stopped`·`failed`), `recorder::Status { phase, source, directory, recorded_seconds, chunks, last_chunk, discontinuities, silence_seconds, error }`
  - `recorder::Recorder::new()`, `status()`, `start(Source, PathBuf)`, `start_with(Source, PathBuf, CaptureFn)`, `pause()`, `resume()`, `stop()`
  - Tauri 명령 `list_audio_sources`, `start_recording{source, directory}`, `pause_recording`, `resume_recording`, `stop_recording`, `recording_status`

- [ ] **Step 1: 녹음기 작성(테스트 포함)**

`app/src-tauri/src/recorder.rs`:

```rust
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
```

- [ ] **Step 2: 명령 등록**

`app/src-tauri/src/lib.rs` 전체를 다음으로 바꾼다.

```rust
//! Tauri commands for the recording validation build.
mod audio;
mod chunker;
mod convert;
mod power;
mod recorder;
mod wav;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::State;

use crate::audio::{describe_sources, Source, SourceInfo};
use crate::recorder::{Recorder, Status};

type Shared<'a> = State<'a, Mutex<Recorder>>;

fn with_recorder<F>(recorder: Shared<'_>, action: F) -> Result<Status, String>
where
    F: FnOnce(&mut Recorder) -> Result<Status, String>,
{
    let mut guard = recorder
        .lock()
        .map_err(|_| "recorder state is poisoned".to_string())?;
    action(&mut guard)
}

#[tauri::command]
fn list_audio_sources() -> Vec<SourceInfo> {
    describe_sources()
}

#[tauri::command]
fn start_recording(
    source: Source,
    directory: String,
    recorder: Shared<'_>,
) -> Result<Status, String> {
    with_recorder(recorder, |inner| {
        inner.start(source, PathBuf::from(directory))
    })
}

#[tauri::command]
fn pause_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.pause())
}

#[tauri::command]
fn resume_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.resume())
}

#[tauri::command]
fn stop_recording(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| inner.stop())
}

#[tauri::command]
fn recording_status(recorder: Shared<'_>) -> Result<Status, String> {
    with_recorder(recorder, |inner| Ok(inner.status()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Mutex::new(Recorder::new()))
        .invoke_handler(tauri::generate_handler![
            list_audio_sources,
            start_recording,
            pause_recording,
            resume_recording,
            stop_recording,
            recording_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Run: `cd app/src-tauri && cargo test`
Expected: `test result: ok. 22 passed; 0 failed; 3 ignored`

Run: `cd app/src-tauri && cargo test -- --ignored`
Expected: 장치 테스트 3개가 통과한다(`a_real_microphone_recording_writes_a_chunk_file` 포함).

- [ ] **Step 3: 검증 화면 작성**

`app/src/App.tsx` 전체를 다음으로 바꾼다.

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Source = "microphone" | "system_sound";
type Phase = "idle" | "recording" | "paused" | "stopped" | "failed";

type SourceInfo = { source: Source; device: string | null; format: string | null };

type Status = {
  phase: Phase;
  source: Source | null;
  directory: string | null;
  recorded_seconds: number;
  chunks: number;
  last_chunk: string | null;
  discontinuities: number;
  silence_seconds: number;
  error: string | null;
};

const SOURCE_LABELS: Record<Source, string> = {
  microphone: "마이크",
  system_sound: "시스템 소리",
};

/** Validation screen for the recording checks in the app stack validation. */
function App() {
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [source, setSource] = useState<Source>("microphone");
  const [directory, setDirectory] = useState("");
  const [status, setStatus] = useState<Status | null>(null);
  const [log, setLog] = useState<string[]>([]);

  const note = (message: string) =>
    setLog((entries) => [`${new Date().toLocaleTimeString()} ${message}`, ...entries].slice(0, 20));

  const run = async (command: string, args: Record<string, unknown> = {}) => {
    try {
      const next = await invoke<Status>(command, args);
      setStatus(next);
      note(`${command}: ${next.phase}`);
    } catch (error) {
      note(`${command} 실패: ${String(error)}`);
    }
  };

  useEffect(() => {
    invoke<SourceInfo[]>("list_audio_sources")
      .then((found) => {
        setSources(found);
        note(`장치 조회: ${found.map((item) => `${SOURCE_LABELS[item.source]}=${item.device ?? "없음"}`).join(", ")}`);
      })
      .catch((error) => note(`장치 조회 실패: ${String(error)}`));
  }, []);

  useEffect(() => {
    const timer = setInterval(() => {
      invoke<Status>("recording_status")
        .then(setStatus)
        .catch(() => undefined);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  const recording = status?.phase === "recording" || status?.phase === "paused";

  return (
    <main className="container">
      <h1>녹음 기술 검증</h1>

      <section>
        <h2>입력</h2>
        {sources.map((item) => (
          <label key={item.source} style={{ display: "block" }}>
            <input
              type="radio"
              name="source"
              value={item.source}
              checked={source === item.source}
              disabled={recording}
              onChange={() => setSource(item.source)}
            />
            {SOURCE_LABELS[item.source]} — {item.device ?? "장치 없음"}
            {item.format ? ` (${item.format})` : ""}
          </label>
        ))}
        <label style={{ display: "block", marginTop: "0.5rem" }}>
          저장 폴더
          <input
            value={directory}
            placeholder="C:\temp_git\Just-a-Click-\artifacts\app-recordings\test"
            disabled={recording}
            onChange={(event) => setDirectory(event.currentTarget.value)}
            style={{ width: "100%" }}
          />
        </label>
      </section>

      <section>
        <h2>제어</h2>
        <button disabled={recording || directory.length === 0} onClick={() => run("start_recording", { source, directory })}>
          시작
        </button>
        <button disabled={status?.phase !== "recording"} onClick={() => run("pause_recording")}>
          일시정지
        </button>
        <button disabled={status?.phase !== "paused"} onClick={() => run("resume_recording")}>
          재개
        </button>
        <button disabled={!recording} onClick={() => run("stop_recording")}>
          정지
        </button>
      </section>

      <section>
        <h2>상태</h2>
        <p>
          단계 {status?.phase ?? "idle"} · 녹음 {status?.recorded_seconds?.toFixed(1) ?? "0.0"}초 · 조각{" "}
          {status?.chunks ?? 0}개 · 불연속 {status?.discontinuities ?? 0}회 · 무음 보정{" "}
          {status?.silence_seconds?.toFixed(1) ?? "0.0"}초
        </p>
        <p>마지막 조각: {status?.last_chunk ?? "없음"}</p>
        {status?.error ? <p style={{ color: "crimson" }}>오류: {status.error}</p> : null}
      </section>

      <section>
        <h2>기록</h2>
        <ul>
          {log.map((entry, index) => (
            <li key={index}>{entry}</li>
          ))}
        </ul>
      </section>
    </main>
  );
}

export default App;
```

Run: `cd app && npm run typecheck`
Expected: 출력 없이 종료 코드 0.

- [ ] **Step 4: 커밋**

```bash
git add app/src-tauri/src/recorder.rs app/src-tauri/src/lib.rs app/src/App.tsx
git commit -m "feat: add the recording screen and its Tauri commands" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: 사람 참여 검증과 기록

**Files:**
- Create: `docs/validation/<날짜>-app-stack.md`
- Modify: `docs/decisions/0007-app-stack.md`, `docs/ROADMAP.md`, `README.md`

**Interfaces:**
- Consumes: Task 1~6의 앱.
- Produces: 항목 ①②의 판정, 실패 시 대안, 로드맵 반영.

- [ ] **Step 1: 사용자 확인**

다음을 사용자에게 알리고 시작한다. 마이크 확인에는 말소리가, 시스템 소리 확인에는 재생 중인 소리가 필요하다. 30분 연속 녹음 동안에는 노트북을 써도 되지만 무거운 작업은 피한다. 녹음 파일은 `artifacts/app-recordings/`에만 저장하고 커밋하지 않는다.

Run: `cd app/src-tauri && cargo test && cargo test -- --ignored`
Expected: `22 passed`와 장치 테스트 `3 passed`

- [ ] **Step 2: 앱 실행과 마이크 60초**

Run: `cd app && npm run tauri dev` (백그라운드)

화면에서 마이크를 고르고 저장 폴더에 `<저장소>\artifacts\app-recordings\mic-60s`를 넣고 시작한다. 60초 동안 사용자가 말한다. 정지 후 조각을 확인한다.

Run:

```bash
python - <<'EOF'
import wave, pathlib
for path in sorted(pathlib.Path('artifacts/app-recordings/mic-60s').glob('*.wav')):
    with wave.open(str(path)) as audio:
        print(path.name, audio.getnchannels(), audio.getframerate(), audio.getsampwidth(),
              round(audio.getnframes() / audio.getframerate(), 2))
EOF
```

Expected: 조각 2개가 `1 16000 2`이고 길이가 각각 30초와 나머지다.

Run:

```bash
./runtimes/whisper-b5130/blas/Release/whisper-cli.exe -m models/whisper/ggml-large-v3-turbo.bin -f artifacts/app-recordings/mic-60s/chunk-0001.wav -l ko -t 8 -nt 2>/dev/null | head -5
```

Expected: 사용자가 말한 내용이 한국어로 나온다(품질 판정은 하지 않는다).

- [ ] **Step 3: 시스템 소리 60초**

저장 폴더를 `artifacts\app-recordings\system-60s`로 바꾸고 시스템 소리를 고른 뒤, 사용자가 영상이나 음원을 재생한 상태에서 60초 녹음한다.

Expected: Step 2와 같은 확인에서 조각 2개가 나오고 whisper 전사에 재생한 소리의 내용이 담긴다. 무음 보정 값도 함께 기록한다.

- [ ] **Step 4: 일시정지와 재개**

저장 폴더를 `artifacts\app-recordings\pause`로 바꾸고 30초 녹음 → 10초 일시정지 → 60초 녹음 후 정지한다.

Expected: 조각 3개가 생기고 합계 길이가 약 90초다. 일시정지 구간의 소리는 조각에 들어가지 않는다(정지 중 재생한 소리가 전사에 나오지 않는지 확인).

- [ ] **Step 5: 30분 연속**

저장 폴더를 `artifacts\app-recordings\long-30m`으로 바꾸고 마이크로 30분 연속 녹음한다. 녹음 중 다음을 확인한다.

Run: `powershell -NoProfile -Command "powercfg /requests"`
Expected: `SYSTEM` 아래에 앱 실행 파일이 보인다. 정지 후 다시 실행하면 사라진다.

Run: `powershell -NoProfile -Command "Get-Process app | Select-Object Name,Id,@{n='private_mib';e={[math]::Round($_.PrivateMemorySize64/1MB,1)}} | Format-Table -AutoSize | Out-String"`
Expected: 앱의 전용 메모리를 시작·중간·종료 시점에 기록한다.

Expected(종료 후): 조각 60개가 생기고 마지막 조각만 짧다. 화면의 불연속 횟수와 무음 보정 값을 기록한다.

- [ ] **Step 6: 릴리스 빌드**

Run: `cd app && npm run tauri build`
Expected: `Finished \`release\` profile`과 번들 생성 줄이 나오고, `app/src-tauri/target/release/bundle/` 아래에 설치 파일이 만들어진다. 경로와 파일 크기를 기록한다.

- [ ] **Step 7: 보고서 작성**

`docs/validation/<날짜>-app-stack.md`를 다음 구조로 쓴다. 수치는 앞 단계에서 기록한 값을 옮기고 판정에는 "추정"이 아니라 실제 확인 결과를 적는다.

````markdown
# 앱 기술 검증 1차: Tauri 2 빌드와 녹음

- 날짜, 범위(결정 0007 ①②), 기기·툴체인 버전, 결과 파일 링크
## 1. 요약
빌드·녹음·일시정지·절전 방지의 통과 여부를 3~5문장으로.
## 2. 환경과 버전
Rust·Tauri·Node·WebView2·MSVC·Windows SDK 버전, 라이브러리 버전과 라이선스.
## 3. 빌드와 실행
개발 실행과 릴리스 빌드 결과, 설치 파일 경로·크기, WebView2 동작.
## 4. 녹음
마이크·시스템 소리 60초 결과(조각 수·형식·길이·전사 확인), 일시정지·재개, 30분 연속(조각 수, 불연속, 무음 보정, 메모리).
## 5. 절전 방지
`powercfg /requests` 결과와 정지 후 해제 확인.
## 6. 발견한 제약
루프백은 재생 장치를 캡처 방향으로 초기화해야 하고 믹스 형식만 받는다. 무음 구간은 앱이 채워야 한다. 자동 변환은 루프백에서 거부됐다.
## 7. 판정과 다음 결정
①②의 통과 여부, 2차 범위(③④⑤), Windows 10·4시간 녹음 미검증.
## 8. 한계
기기 1대, Windows 11만, 장치 분리 시험 미실시, 실제 강의 환경 미검증.
````

- [ ] **Step 8: 결정 기록과 문서 갱신**

`docs/decisions/0007-app-stack.md` 3절 끝에 다음을 덧붙인다(값은 보고서에서 옮긴다).

```markdown
2026-09-25 1차 검증: ①②를 확인했다. Rust 1.98.1·Tauri 2.11·WebView2 153으로 개발 실행과 릴리스 빌드가 됐고, 마이크와 시스템 소리(WASAPI 루프백)를 16kHz 모노 PCM16 30초 조각으로 녹음해 whisper-cli 전사까지 확인했다. 절전·화면 꺼짐 방지도 동작했다. 쓰는 라이브러리는 `wasapi` 0.24(MIT), `windows` 0.62(MIT OR Apache-2.0)이며 WAV 기록과 리샘플링은 의존성 없이 구현했다. 자세한 수치와 제약은 [검증 보고서](../validation/<날짜>-app-stack.md)에 있다. ③④⑤는 2차 검증에서 확인한다.
```

`docs/ROADMAP.md` M0 남은 작업 7 끝에 다음을 붙인다.

```markdown
[1차 검증 설계](superpowers/specs/2026-09-23-app-stack-validation-design.md), [실행 계획](superpowers/plans/2026-09-25-app-stack-validation.md). 결과: Tauri 2 빌드·WebView2와 마이크·시스템 소리 녹음·일시정지·절전 방지를 확인했다([보고서](validation/<날짜>-app-stack.md)). 추론 프로세스 관리, SQLite 한국어 검색, 자격 증명 저장은 2차에서 확인한다.
```

`README.md` 문서 목록에 다음을 추가한다.

```markdown
- [앱 기술 검증 1차: Tauri 2 빌드와 녹음](docs/validation/<날짜>-app-stack.md)
```

- [ ] **Step 9: 검증 후 커밋**

Run: `cd app/src-tauri && cargo test`
Expected: `22 passed`

Run: `python artifacts/check_docs.py`
Expected: `local paths: none`, `broken links: none`

Run: `git status --short`
Expected: 문서와 결정 기록만 변경되어 있고 `artifacts/`, `app/target`, `app/node_modules`는 보이지 않는다.

```bash
git add docs/validation/<날짜>-app-stack.md docs/decisions/0007-app-stack.md docs/ROADMAP.md README.md
git commit -m "test: validate the Tauri app build and recording path" -m "Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 10: 사용자 보고**

작업·결과·다음 형식으로 보고한다. 결과에는 빌드·녹음·일시정지·절전 방지의 통과 여부, 30분 연속 결과, 발견한 제약을 넣는다. 다음으로 2차 검증(③④⑤)을 제안한다.
