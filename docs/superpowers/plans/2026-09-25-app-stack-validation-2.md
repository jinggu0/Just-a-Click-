# 앱 기술 검증 2차 실행 계획: 추론 프로세스·한국어 검색·자격 증명

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 결정 0007 3절의 ③④⑤를 기준 노트북에서 확인할 수 있는 최소 골격을 `app/`에 만들고, 사람 조작 없이 돌아가는 하네스로 판정한다.

**Architecture:** 1차와 같은 방식이다. Rust 코어에 책임이 하나씩인 모듈 여섯 개를 더하고, 화면에는 탭을 셋 더한다. 오래 걸리는 측정은 화면이 아니라 `examples/` 하네스에서 돌리고 결과를 JSON으로 남긴다. 추론 자식 프로세스는 Job Object에 묶어 앱이 어떻게 죽든 커널이 정리하게 한다.

**Tech Stack:** Rust 1.98.1(MSVC), Tauri 2.11.6, React 19·TypeScript 6, `rusqlite` 0.40(`bundled`, SQLite 3.53.2), `reqwest` 0.13(`blocking`,`json`), `windows` 0.62(`Win32_Security_Credentials`·`Win32_System_JobObjects`·`Win32_Foundation`).

## Global Constraints

- 설계는 [앱 기술 검증 2차 설계](../specs/2026-09-25-app-stack-validation-2-design.md)를 따른다. 성공 기준과 절차를 바꾸지 않는다. 단, ④의 판정은 이 계획 작성 중 확인한 사실에 따라 "질의 종류별 재현율 중심"으로 적용한다(아래 사전 확인 참고).
- 새로 넣는 서드파티 크레이트는 `rusqlite` 0.40(MIT) 하나다. `reqwest`·`windows`·`tokio`는 이미 의존성 트리에 있으므로 기능(feature)만 더한다. `keyring`은 쓰지 않는다.
- 추론 실행 설정은 [결정 0009](../../decisions/0009-concurrent-processing.md)를 따른다: 컨텍스트 8,192, `-np 1`, `--cache-ram 0`, `-ngl 99`, LLM 2스레드, STT 8스레드, whisper는 `-nt`.
- 토큰은 환경 변수 `LLAMA_API_KEY`로만 자식에 넘긴다. 명령줄·로그·상태에 토큰 값을 넣지 않는다.
- 서버는 `127.0.0.1`에만 연다. 포트는 0번 바인딩으로 빈 포트를 먼저 잡는다.
- 코퍼스·DB·자식 로그·측정 JSON은 `artifacts/` 아래에만 만들고 커밋하지 않는다. 사용자 음성과 모델 가중치도 커밋하지 않는다.
- 자격 증명 검증은 `dev.justaclick.test/notion` 항목만 건드리고 끝나면 지운다. 사용자의 다른 자격 증명은 읽지 않는다.
- 코드 주석과 식별자는 영어, 화면 문구와 문서는 한국어로 쓴다(1차와 같다).
- 모든 명령은 `C:\temp_git\Just-a-Click-`을 저장소 뿌리로 삼는다. Bash에서 cargo를 쓰려면 먼저 `export PATH="$PATH:/c/Users/jingg/.cargo/bin"`을 실행한다.
- 작업마다 커밋하고 메시지 끝에 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`를 넣는다. push는 하지 않는다.

## 계획 작성 중 확인한 사실 (스파이크, 2026-09-25)

계획의 코드는 아래를 실제로 실행해 확인한 API만 쓴다.

1. `rusqlite` 0.40 `bundled`은 **SQLite 3.53.2**를 담고 `tokenize='trigram'` 가상 테이블을 만들 수 있다.
2. Job Object에 자식을 넣고 `CloseHandle(job)`을 하면 자식이 함께 죽는다(`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
3. `CredWriteW` → `CredReadW` → `CredDeleteW` 왕복이 동작하고, 삭제 뒤 읽기는 `HRESULT(0x80070490)`(요소 없음)을 준다.
4. `reqwest`에 `blocking`·`json` 기능을 더해도 앱 크레이트가 그대로 빌드된다.
5. **400만 자(강의 100개분) 코퍼스에서 세 방식 모두 36ms 안에 답한다.** 그래서 속도는 변별력이 없고 판정은 질의 종류별 재현율로 한다. 같은 코퍼스에서 관찰한 차이는 다음과 같다.

| 질의 종류 | trigram | unicode61 | unicode61 접두 | LIKE |
| --- | --- | --- | --- | --- |
| 두 글자("있는") | **0건** | 20건 | 20건 | 20건 |
| 어간+조사("사람들") | 20건 | 19건 | 20건 | 20건 |
| 어중 부분 문자열("반적으") | 20건 | **0건** | 1건 | 20건 |
| 강의 고유 용어 | 1건 | **0건** | 1건 | 1건 |

   색인 시간·파일 크기는 trigram 0.7초·25.2MiB, unicode61 0.3초·13.3MiB, 색인 없음 9.8MiB였다. 이 표는 스파이크 값이며 Task 7의 측정으로 다시 만든다.

## 파일 구조

| 파일 | 책임 |
| --- | --- |
| `app/src-tauri/src/process.rs` | Job Object 생성, 자식 시작과 정지. OS 경계만 다룬다 |
| `app/src-tauri/src/llm.rs` | llama-server 시작·준비 대기·스트리밍 요청·취소 |
| `app/src-tauri/src/stt.rs` | whisper-cli 실행·취소·결과 읽기 |
| `app/src-tauri/src/store.rs` | SQLite 열기, 검증용 스키마, 조각 적재 |
| `app/src-tauri/src/search.rs` | 세 방식 질의, 정답 대조, 재현율·정확도 |
| `app/src-tauri/src/secrets.rs` | 자격 증명 관리자 저장·조회·삭제 |
| `app/src-tauri/examples/inference_check.rs` | ③ 절차를 순서대로 돌리고 JSON을 남긴다 |
| `app/src-tauri/examples/search_bench.rs` | 코퍼스 생성·색인·질의 측정을 돌리고 JSON을 남긴다 |
| `app/src/InferenceTab.tsx` `app/src/SearchTab.tsx` `app/src/SecretsTab.tsx` | 검증 탭 |
| `app/src/RecordingTab.tsx` | 1차 화면을 탭으로 옮긴 것 |
| `app/src/App.tsx` | 탭 전환만 맡는다 |

---

### Task 1: 자식 프로세스 묶음

**Files:**
- Create: `app/src-tauri/src/process.rs`
- Modify: `app/src-tauri/Cargo.toml`, `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 없음.
- Produces: `ProcessGroup::new() -> Result<ProcessGroup, String>`, `ProcessGroup::spawn(&self, &mut Command) -> Result<Child, String>`, `stop(&mut Child, Duration) -> Result<(), String>`, `wait_for_exit(&mut Child, Duration) -> Option<ExitStatus>`.

- [ ] **Step 1: 의존성 추가**

`app/src-tauri/Cargo.toml`의 `[dependencies]`에서 `windows` 줄을 아래로 바꾸고 두 줄을 더한다.

```toml
windows = { version = "0.62", features = ["Win32_System_Power", "Win32_Foundation", "Win32_Security_Credentials", "Win32_System_JobObjects"] }
reqwest = { version = "0.13", features = ["blocking", "json"] }
rusqlite = { version = "0.40", features = ["bundled"] }
```

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo fetch`
Expected: 오류 없이 끝나고 `Cargo.lock`에 `rusqlite`가 생긴다.

- [ ] **Step 2: 실패하는 테스트 작성**

`app/src-tauri/src/process.rs`를 만들고 아래 테스트만 먼저 넣는다.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 30 127.0.0.1 > nul"]);
        command
    }

    #[test]
    fn a_child_dies_when_the_group_is_dropped() {
        let group = ProcessGroup::new().expect("job object");
        let mut child = group.spawn(&mut sleeper()).expect("spawn");
        assert!(child.try_wait().expect("try_wait").is_none(), "child should still run");
        drop(group);
        assert!(
            wait_for_exit(&mut child, Duration::from_secs(5)).is_some(),
            "the job object should have killed the child"
        );
    }

    #[test]
    fn stop_ends_a_running_child() {
        let group = ProcessGroup::new().expect("job object");
        let mut child = group.spawn(&mut sleeper()).expect("spawn");
        stop(&mut child, Duration::from_secs(5)).expect("stop");
        assert!(child.try_wait().expect("try_wait").is_some(), "child should be gone");
    }
}
```

- [ ] **Step 3: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test process`
Expected: 컴파일 실패(`cannot find type ProcessGroup`).

- [ ] **Step 4: 구현 작성**

`app/src-tauri/src/process.rs`의 테스트 위에 아래를 넣는다.

```rust
//! Child processes that cannot outlive the app.
//!
//! Windows kills every process in a job object when the last handle to it closes, so the
//! inference runtimes go away even when the app is force-closed and cannot clean up.
use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

const POLL: Duration = Duration::from_millis(50);

/// A job object every inference child is assigned to.
pub struct ProcessGroup {
    job: HANDLE,
}

impl ProcessGroup {
    pub fn new() -> Result<Self, String> {
        let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }
            .map_err(|error| format!("job object failed: {error}"))?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| format!("job object limits failed: {error}"))?;
        Ok(Self { job })
    }

    /// Starts the command and puts it in the group before it can outlive the app.
    pub fn spawn(&self, command: &mut Command) -> Result<Child, String> {
        let child = command.spawn().map_err(|error| format!("start failed: {error}"))?;
        let handle = HANDLE(child.as_raw_handle() as *mut c_void);
        unsafe { AssignProcessToJobObject(self.job, handle) }
            .map_err(|error| format!("could not group the child: {error}"))?;
        Ok(child)
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.job) };
    }
}

/// Waits for the child, returning None when it is still running after the deadline.
pub fn wait_for_exit(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

/// Ends a child and waits for it. Windows has no graceful signal, so this terminates.
pub fn stop(child: &mut Child, grace: Duration) -> Result<(), String> {
    if child.try_wait().map_err(|error| error.to_string())?.is_some() {
        return Ok(());
    }
    child.kill().map_err(|error| format!("stop failed: {error}"))?;
    match wait_for_exit(child, grace) {
        Some(_) => Ok(()),
        None => Err(format!("the child was still running after {:.0}s", grace.as_secs_f64())),
    }
}
```

`app/src-tauri/src/lib.rs`의 모듈 목록에 `pub mod process;`를 알파벳 순서에 맞게 넣는다.

- [ ] **Step 5: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test process`
Expected: `2 passed`

- [ ] **Step 6: 커밋**

```bash
git add app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/process.rs app/src-tauri/src/lib.rs
git commit -m "feat: group inference children in a job object" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: llama-server 제어와 취소

**Files:**
- Create: `app/src-tauri/src/llm.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `process::{ProcessGroup, stop, wait_for_exit}`.
- Produces: `ServerSettings{executable,model,log,context_tokens,threads}`, `free_port() -> Result<u16,String>`, `server_command(&ServerSettings,u16,&str) -> Command`, `Server::start(&ProcessGroup,&ServerSettings) -> Result<Server,String>`, `Server::{base,key,alive,stop}`, `draft_body(&str,u32) -> String`, `consume_stream<R: BufRead>(R,&AtomicBool,&mut dyn FnMut(&str)) -> Result<Stop,String>`, `enum Stop{Finished,Cancelled}`, `stream_draft(&str,&str,&str,u32,&AtomicBool) -> Result<(Stop,String),String>`, `check_auth(&str,&str) -> Result<u16,String>`.

- [ ] **Step 1: 실패하는 테스트 작성**

`app/src-tauri/src/llm.rs`를 만들고 테스트를 먼저 넣는다.

```rust
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
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test llm`
Expected: 컴파일 실패(`cannot find function server_command`).

- [ ] **Step 3: 구현 작성**

```rust
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
```

`app/src-tauri/src/lib.rs`에 `pub mod llm;`을 더한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test llm`
Expected: `6 passed`

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/llm.rs app/src-tauri/src/lib.rs
git commit -m "feat: run llama-server with a loopback key and a cancellable stream" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: whisper-cli 실행과 취소

**Files:**
- Create: `app/src-tauri/src/stt.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `process::{ProcessGroup, stop, wait_for_exit}`.
- Produces: `TranscribeSettings{executable,model,chunk,output,threads}`, `transcribe_command(&TranscribeSettings) -> Command`, `run(&ProcessGroup,&TranscribeSettings,&AtomicBool) -> Result<Outcome,String>`, `enum Outcome{Text(String),Cancelled}`, `output_file(&TranscribeSettings) -> PathBuf`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn settings(command: &str, output: PathBuf) -> TranscribeSettings {
        TranscribeSettings {
            executable: PathBuf::from("cmd"),
            model: PathBuf::from(command),
            chunk: PathBuf::from("chunk-0001.wav"),
            output,
            threads: 8,
        }
    }

    #[test]
    fn the_command_asks_for_korean_without_timestamps() {
        let settings = TranscribeSettings {
            executable: PathBuf::from("whisper-cli.exe"),
            model: PathBuf::from("model.bin"),
            chunk: PathBuf::from("chunk-0001.wav"),
            output: PathBuf::from("out"),
            threads: 8,
        };
        let arguments: Vec<String> = transcribe_command(&settings)
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect();
        for expected in ["-l", "ko", "-nt", "-otxt", "-t", "8"] {
            assert!(arguments.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn the_output_file_gets_the_txt_suffix() {
        let settings = settings("unused", PathBuf::from("run/out"));
        assert_eq!(output_file(&settings), PathBuf::from("run/out.txt"));
    }

    #[test]
    fn a_cancelled_run_removes_the_partial_output() {
        let directory = std::env::temp_dir().join("jac-stt-cancel");
        let _ = std::fs::create_dir_all(&directory);
        let output = directory.join("out");
        std::fs::write(output.with_extension("txt"), "부분 결과").expect("write");
        let mut settings = settings("unused", output.clone());
        settings.executable = PathBuf::from("cmd");
        let group = crate::process::ProcessGroup::new().expect("group");
        let cancel = AtomicBool::new(true);
        let outcome = run_with(&group, &settings, &cancel, sleeper).expect("run");
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(!output.with_extension("txt").exists(), "partial output should be gone");
    }

    #[test]
    fn a_finished_run_returns_the_text() {
        let directory = std::env::temp_dir().join("jac-stt-done");
        let _ = std::fs::create_dir_all(&directory);
        let output = directory.join("out");
        let _ = std::fs::remove_file(output.with_extension("txt"));
        let settings = settings("unused", output.clone());
        let group = crate::process::ProcessGroup::new().expect("group");
        let cancel = AtomicBool::new(false);
        let outcome = run_with(&group, &settings, &cancel, writer).expect("run");
        assert_eq!(outcome, Outcome::Text("korean result".to_string()));
    }

    /// Stands in for whisper-cli: keeps running so the cancel path has something to stop.
    fn sleeper(_settings: &TranscribeSettings) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 30 127.0.0.1 > nul"]);
        command
    }

    /// Stands in for whisper-cli: writes the result file and exits. The text stays ASCII
    /// because `echo` writes in the console code page, not UTF-8, and `raw_arg` keeps
    /// Rust from escaping the quotes cmd.exe needs around the path.
    fn writer(settings: &TranscribeSettings) -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        command
            .arg("/c")
            .raw_arg(format!("echo korean result> \"{}\"", output_file(settings).display()));
        command
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test stt`
Expected: 컴파일 실패(`cannot find type TranscribeSettings`).

- [ ] **Step 3: 구현 작성**

```rust
//! One whisper-cli run over one chunk, with a cancel that leaves no partial file behind.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::process::{stop, ProcessGroup};

pub const STOP_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TranscribeSettings {
    pub executable: PathBuf,
    pub model: PathBuf,
    pub chunk: PathBuf,
    /// Without a suffix; whisper-cli appends `.txt`.
    pub output: PathBuf,
    pub threads: u32,
}

pub fn output_file(settings: &TranscribeSettings) -> PathBuf {
    settings.output.with_extension("txt")
}

/// Decision 0008: Korean, eight threads, no timestamps.
pub fn transcribe_command(settings: &TranscribeSettings) -> Command {
    let mut command = Command::new(&settings.executable);
    command
        .args(["-m".to_string(), settings.model.to_string_lossy().to_string()])
        .args(["-f".to_string(), settings.chunk.to_string_lossy().to_string()])
        .args(["-l".to_string(), "ko".to_string()])
        .args(["-t".to_string(), settings.threads.to_string()])
        .arg("-nt")
        .arg("-otxt")
        .args(["-of".to_string(), settings.output.to_string_lossy().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub fn run(
    group: &ProcessGroup,
    settings: &TranscribeSettings,
    cancel: &AtomicBool,
) -> Result<Outcome, String> {
    run_with(group, settings, cancel, transcribe_command)
}

/// The command builder is injected so tests can stand in for whisper-cli.
pub fn run_with(
    group: &ProcessGroup,
    settings: &TranscribeSettings,
    cancel: &AtomicBool,
    build: fn(&TranscribeSettings) -> Command,
) -> Result<Outcome, String> {
    let mut command = build(settings);
    let mut child = group.spawn(&mut command)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            stop(&mut child, STOP_GRACE)?;
            let _ = std::fs::remove_file(output_file(settings));
            return Ok(Outcome::Cancelled);
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => break,
            Some(status) => return Err(format!("whisper-cli exited {status}")),
            None => std::thread::sleep(POLL),
        }
    }
    let text = std::fs::read_to_string(output_file(settings))
        .map_err(|error| format!("result file missing: {error}"))?;
    Ok(Outcome::Text(text.trim().to_string()))
}
```

`app/src-tauri/src/lib.rs`에 `pub mod stt;`을 더한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test stt`
Expected: `4 passed`

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/stt.rs app/src-tauri/src/lib.rs
git commit -m "feat: transcribe one chunk with a cancellable whisper-cli run" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 추론 검증 하네스

**Files:**
- Create: `app/src-tauri/examples/inference_check.rs`

**Interfaces:**
- Consumes: `process::ProcessGroup`, `llm::{ServerSettings, Server, Stop, check_auth, stream_draft}`, `stt::{TranscribeSettings, Outcome, output_file, run}`.
- Produces: `artifacts/app-inference/<타임스탬프>.json`과 같은 내용의 표준 출력.

- [ ] **Step 1: 하네스 작성**

`app/src-tauri/examples/inference_check.rs`:

```rust
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

use app_lib::llm::{check_auth, stream_draft, Server, ServerSettings, Stop};
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
```

- [ ] **Step 2: 컴파일 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo build --release --example inference_check`
Expected: `Finished` (경고는 없어야 한다)

- [ ] **Step 3: 커밋**

```bash
git add app/src-tauri/examples/inference_check.rs
git commit -m "test: add the inference process validation harness" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: 검증용 저장소와 색인

**Files:**
- Create: `app/src-tauri/src/store.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 없음.
- Produces: `enum Method{Trigram,Unicode61,Like}`, `Method::{all,label,from_label}`, `struct Segment{lecture:i64, body:String}`, `open(&Path) -> Result<Connection,String>`, `create_schema(&Connection, Method) -> Result<(),String>`, `insert_segments(&Connection,&[Segment]) -> Result<(),String>`, `rebuild_index(&Connection, Method) -> Result<(),String>`, `SEGMENT_CHARACTERS`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Segment> {
        vec![
            Segment { lecture: 1, body: "확률변수의 기댓값은 분포에 따라 달라집니다".to_string() },
            Segment { lecture: 1, body: "정규분포에서 표준편차가 커지면 폭이 넓어집니다".to_string() },
            Segment { lecture: 2, body: "GPU 메모리가 부족하면 배치를 줄여야 합니다".to_string() },
        ]
    }

    fn loaded(method: Method) -> Connection {
        let connection = Connection::open_in_memory().expect("memory database");
        create_schema(&connection, method).expect("schema");
        insert_segments(&connection, &rows()).expect("insert");
        rebuild_index(&connection, method).expect("rebuild");
        connection
    }

    #[test]
    fn the_bundled_sqlite_is_new_enough_for_trigram() {
        let connection = Connection::open_in_memory().expect("memory database");
        let version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .expect("version");
        let numbers: Vec<u32> = version.split('.').filter_map(|part| part.parse().ok()).collect();
        assert!(numbers[0] > 3 || (numbers[0] == 3 && numbers[1] >= 34), "sqlite {version} has no trigram");
    }

    #[test]
    fn every_method_loads_the_same_rows() {
        for method in Method::all() {
            let connection = loaded(method);
            let count: i64 = connection
                .query_row("SELECT count(*) FROM segment", [], |row| row.get(0))
                .expect("count");
            assert_eq!(count, 3, "{} lost rows", method.label());
        }
    }

    #[test]
    fn the_label_round_trips() {
        for method in Method::all() {
            assert_eq!(Method::from_label(method.label()), Some(method));
        }
        assert_eq!(Method::from_label("none"), None);
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test store`
Expected: 컴파일 실패(`cannot find type Method`).

- [ ] **Step 3: 구현 작성**

```rust
//! The validation database: lecture segments and one index per method.
//!
//! This schema exists to compare search methods. The product schema is decided later
//! (roadmap item 10), so nothing here is a migration target.
use std::path::Path;

use rusqlite::Connection;

pub const SEGMENT_CHARACTERS: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Trigram,
    Unicode61,
    Like,
}

impl Method {
    pub fn all() -> [Method; 3] {
        [Method::Trigram, Method::Unicode61, Method::Like]
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::Trigram => "trigram",
            Method::Unicode61 => "unicode61",
            Method::Like => "like",
        }
    }

    pub fn from_label(label: &str) -> Option<Method> {
        Method::all().into_iter().find(|method| method.label() == label)
    }
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub lecture: i64,
    pub body: String,
}

pub fn open(path: &Path) -> Result<Connection, String> {
    Connection::open(path).map_err(|error| format!("database failed: {error}"))
}

/// One table of segments plus the index the method needs. `like` needs no index.
pub fn create_schema(connection: &Connection, method: Method) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS segment(
                 id INTEGER PRIMARY KEY,
                 lecture_id INTEGER NOT NULL,
                 body TEXT NOT NULL);",
        )
        .map_err(|error| format!("schema failed: {error}"))?;
    let index = match method {
        Method::Trigram => Some(
            "CREATE VIRTUAL TABLE IF NOT EXISTS segment_index USING fts5(
                 body, tokenize='trigram', content='segment', content_rowid='id');",
        ),
        Method::Unicode61 => Some(
            "CREATE VIRTUAL TABLE IF NOT EXISTS segment_index USING fts5(
                 body, tokenize='unicode61', content='segment', content_rowid='id');",
        ),
        Method::Like => None,
    };
    if let Some(statement) = index {
        connection
            .execute_batch(statement)
            .map_err(|error| format!("index failed: {error}"))?;
    }
    Ok(())
}

pub fn insert_segments(connection: &Connection, rows: &[Segment]) -> Result<(), String> {
    connection.execute_batch("BEGIN").map_err(|error| error.to_string())?;
    {
        let mut insert = connection
            .prepare("INSERT INTO segment(id, lecture_id, body) VALUES (?1, ?2, ?3)")
            .map_err(|error| error.to_string())?;
        for (index, row) in rows.iter().enumerate() {
            insert
                .execute((index as i64 + 1, row.lecture, &row.body))
                .map_err(|error| format!("insert failed: {error}"))?;
        }
    }
    connection.execute_batch("COMMIT").map_err(|error| error.to_string())
}

/// External content tables do not index anything until they are rebuilt.
pub fn rebuild_index(connection: &Connection, method: Method) -> Result<(), String> {
    if method == Method::Like {
        return Ok(());
    }
    connection
        .execute_batch("INSERT INTO segment_index(segment_index) VALUES('rebuild')")
        .map_err(|error| format!("rebuild failed: {error}"))
}
```

`app/src-tauri/src/lib.rs`에 `pub mod store;`를 더한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test store`
Expected: `3 passed`

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/store.rs app/src-tauri/src/lib.rs
git commit -m "feat: store lecture segments with one index per search method" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: 세 방식 질의와 정답 대조

**Files:**
- Create: `app/src-tauri/src/search.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `store::{Method, Segment}`.
- Produces: `search(&Connection, Method, &str, usize) -> Result<Vec<i64>,String>`, `search_prefix(&Connection,&str,usize) -> Result<Vec<i64>,String>`, `truth(&[Segment],&str) -> Vec<i64>`, `recall(&[i64],&[i64],usize) -> f64`, `precision(&[i64],&[i64]) -> f64`, `candidate_method(&str) -> Method`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_schema, insert_segments, rebuild_index};
    use rusqlite::Connection;

    fn rows() -> Vec<Segment> {
        vec![
            Segment { lecture: 1, body: "확률변수의 기댓값은 분포에 따라 달라집니다".to_string() },
            Segment { lecture: 1, body: "정규분포에서 표준편차가 커지면 폭이 넓어집니다".to_string() },
            Segment { lecture: 2, body: "GPU 메모리가 부족하면 배치를 줄여야 합니다".to_string() },
        ]
    }

    fn loaded(method: Method) -> Connection {
        let connection = Connection::open_in_memory().expect("memory database");
        create_schema(&connection, method).expect("schema");
        insert_segments(&connection, &rows()).expect("insert");
        rebuild_index(&connection, method).expect("rebuild");
        connection
    }

    #[test]
    fn trigram_finds_a_stem_that_carries_a_particle() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "확률변수", 20).expect("search");
        assert_eq!(found, vec![1]);
    }

    #[test]
    fn trigram_finds_a_substring_inside_a_word() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "률변수", 20).expect("search");
        assert_eq!(found, vec![1]);
    }

    #[test]
    fn unicode61_misses_the_stem_but_a_prefix_query_finds_it() {
        let connection = loaded(Method::Unicode61);
        assert!(search(&connection, Method::Unicode61, "확률변수", 20).expect("search").is_empty());
        assert_eq!(search_prefix(&connection, "확률변수", 20).expect("prefix"), vec![1]);
    }

    #[test]
    fn like_finds_both_shapes() {
        let connection = loaded(Method::Like);
        assert_eq!(search(&connection, Method::Like, "확률변수", 20).expect("search"), vec![1]);
        assert_eq!(search(&connection, Method::Like, "률변수", 20).expect("search"), vec![1]);
    }

    #[test]
    fn a_two_character_query_is_out_of_reach_for_trigram() {
        let found = search(&loaded(Method::Trigram), Method::Trigram, "분포", 20).expect("search");
        assert!(found.is_empty(), "trigram indexes three characters at a time");
        let others = search(&loaded(Method::Like), Method::Like, "분포", 20).expect("search");
        assert_eq!(others, vec![1, 2]);
    }

    #[test]
    fn the_candidate_rule_sends_short_queries_to_like() {
        assert_eq!(candidate_method("분포"), Method::Like);
        assert_eq!(candidate_method("확률변수"), Method::Trigram);
    }

    #[test]
    fn truth_and_scores_come_from_the_text_itself() {
        let truth = truth(&rows(), "분포");
        assert_eq!(truth, vec![1, 2]);
        assert_eq!(recall(&[1, 2], &truth, 20), 1.0);
        assert_eq!(recall(&[1], &truth, 20), 0.5);
        assert_eq!(precision(&[1, 3], &truth), 0.5);
        assert_eq!(precision(&[], &truth), 1.0);
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test search`
Expected: 컴파일 실패(`cannot find function search`).

- [ ] **Step 3: 구현 작성**

```rust
//! Queries for the three methods and scores against the text itself.
//!
//! The truth set is a plain substring scan over the corpus, so no engine decides what a
//! correct answer is.
use rusqlite::Connection;

use crate::store::{Method, Segment};

/// Trigram indexes three characters at a time, so shorter queries cannot be answered.
pub const TRIGRAM_MINIMUM: usize = 3;

fn quoted(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

pub fn search(
    connection: &Connection,
    method: Method,
    query: &str,
    limit: usize,
) -> Result<Vec<i64>, String> {
    match method {
        Method::Like => rows(
            connection,
            "SELECT id FROM segment WHERE body LIKE ?1 ORDER BY id LIMIT ?2",
            (format!("%{query}%"), limit as i64),
        ),
        _ => rows(
            connection,
            "SELECT rowid FROM segment_index WHERE segment_index MATCH ?1 ORDER BY rowid LIMIT ?2",
            (quoted(query), limit as i64),
        ),
    }
}

/// The unicode61 fallback for a stem that carries a particle: match the token's prefix.
pub fn search_prefix(connection: &Connection, query: &str, limit: usize) -> Result<Vec<i64>, String> {
    rows(
        connection,
        "SELECT rowid FROM segment_index WHERE segment_index MATCH ?1 ORDER BY rowid LIMIT ?2",
        (format!("{}*", quoted(query)), limit as i64),
    )
}

fn rows(
    connection: &Connection,
    statement: &str,
    parameters: (String, i64),
) -> Result<Vec<i64>, String> {
    let mut prepared = connection.prepare(statement).map_err(|error| error.to_string())?;
    let found = prepared
        .query_map(rusqlite::params![parameters.0, parameters.1], |row| row.get(0))
        .map_err(|error| format!("query failed: {error}"))?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(|error| format!("query failed: {error}"))?;
    Ok(found)
}

/// Segment ids that really contain the query, numbered the way the database numbers them.
pub fn truth(rows: &[Segment], query: &str) -> Vec<i64> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.body.contains(query))
        .map(|(index, _)| index as i64 + 1)
        .collect()
}

/// How much of what the query should find came back, given the result limit.
pub fn recall(found: &[i64], truth: &[i64], limit: usize) -> f64 {
    let reachable = truth.len().min(limit);
    if reachable == 0 {
        return 1.0;
    }
    let hits = found.iter().filter(|id| truth.contains(id)).count();
    hits as f64 / reachable as f64
}

/// How much of what came back belongs there.
pub fn precision(found: &[i64], truth: &[i64]) -> f64 {
    if found.is_empty() {
        return 1.0;
    }
    let hits = found.iter().filter(|id| truth.contains(id)).count();
    hits as f64 / found.len() as f64
}

/// The rule the report recommends if the measurement confirms it: trigram for three
/// characters and more, a scan for the short queries trigram cannot index.
pub fn candidate_method(query: &str) -> Method {
    if query.chars().count() >= TRIGRAM_MINIMUM {
        Method::Trigram
    } else {
        Method::Like
    }
}
```

`app/src-tauri/src/lib.rs`에 `pub mod search;`를 더한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test search`
Expected: `7 passed`. 하나라도 실패하면 그 결과가 곧 발견 사항이므로, 기대값을 고치지 말고 실제 동작을 기록한 뒤 Task 10 보고서에 반영한다.

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/search.rs app/src-tauri/src/lib.rs
git commit -m "feat: query the three search methods and score them against the text" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: 검색 측정 하네스

**Files:**
- Create: `app/src-tauri/examples/search_bench.rs`

**Interfaces:**
- Consumes: `store::{Method, Segment, create_schema, insert_segments, open, rebuild_index, SEGMENT_CHARACTERS}`, `search::{precision, recall, search, search_prefix, truth}`.
- Produces: `artifacts/app-search/<타임스탬프>/search-bench.json`과 방식별 `.db` 파일.

- [ ] **Step 1: 하네스 작성**

`app/src-tauri/examples/search_bench.rs`:

```rust
//! Builds a lecture-sized Korean corpus, indexes it three ways and measures each.
//!
//! Usage: cargo run --release --example search_bench -- <fleurs test.tsv> <out dir> [lectures]
//!
//! The corpus reuses public FLEURS sentences, so it is not real lecture speech. It is only
//! used to compare methods on the same text.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use app_lib::search::{precision, recall, search, search_prefix, truth};
use app_lib::store::{create_schema, insert_segments, open, rebuild_index, Method, Segment, SEGMENT_CHARACTERS};

const LECTURE_CHARACTERS: usize = 40_000;
const LIMIT: usize = 20;
const REPEATS: usize = 10;
const PARTICLES: [char; 8] = ['은', '는', '이', '가', '을', '를', '의', '에'];

/// A fixed sequence so a rerun builds the same corpus.
struct Picker(u64);

impl Picker {
    fn next(&mut self, limit: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize % limit
    }
}

fn sentences(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("fleurs tsv")
        .lines()
        .filter_map(|line| line.split('\t').nth(2).map(str::to_string))
        .filter(|text| text.chars().count() > 20)
        .collect()
}

fn corpus(pool: &[String], lectures: usize) -> (Vec<Segment>, Vec<String>) {
    let mut picker = Picker(20_260_925);
    let mut rows = Vec::new();
    let mut terms = Vec::new();
    for lecture in 0..lectures {
        let term = format!("제{lecture}장특강용어");
        let mut text = format!("{term}에 대한 강의입니다. ");
        while text.chars().count() < LECTURE_CHARACTERS {
            text.push_str(&pool[picker.next(pool.len())]);
            text.push(' ');
        }
        let characters: Vec<char> = text.chars().collect();
        for chunk in characters.chunks(SEGMENT_CHARACTERS) {
            rows.push(Segment { lecture: lecture as i64, body: chunk.iter().collect() });
        }
        terms.push(term);
    }
    (rows, terms)
}

fn hangul(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|character| ('가'..='힣').contains(&character))
}

/// Five query kinds taken from the corpus itself, four of each where possible.
fn queries(rows: &[Segment], terms: &[String]) -> Vec<(String, String)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        for word in row.body.split_whitespace() {
            let word: String = word
                .chars()
                .filter(|character| ('가'..='힣').contains(character) || character.is_ascii_alphanumeric())
                .collect();
            if !word.is_empty() {
                *counts.entry(word).or_default() += 1;
            }
        }
    }
    let mut words: Vec<(&String, &usize)> = counts.iter().collect();
    words.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    let mut picked: Vec<(String, String)> = Vec::new();
    let mut count = |picked: &Vec<(String, String)>, kind: &str| {
        picked.iter().filter(|(existing, _)| existing == kind).count()
    };
    for (word, _) in &words {
        let characters: Vec<char> = word.chars().collect();
        if count(&picked, "어간+조사") < 4
            && characters.len() >= 4
            && hangul(word)
            && PARTICLES.contains(&characters[characters.len() - 1])
        {
            picked.push(("어간+조사".into(), characters[..characters.len() - 1].iter().collect()));
        } else if count(&picked, "어중") < 4 && characters.len() >= 5 && hangul(word) {
            picked.push(("어중".into(), characters[1..4].iter().collect()));
        } else if count(&picked, "두 글자") < 4 && characters.len() == 2 && hangul(word) {
            picked.push(("두 글자".into(), word.to_string()));
        } else if count(&picked, "영문") < 2
            && characters.len() >= 3
            && word.chars().all(|character| character.is_ascii_alphanumeric())
        {
            picked.push(("영문".into(), word.to_string()));
        }
    }
    for term in terms.iter().take(2) {
        picked.push(("고유 용어".into(), term.clone()));
    }
    picked
}

fn milliseconds(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

fn main() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() < 2 {
        return Err("usage: search_bench <fleurs test.tsv> <out dir> [lectures]".into());
    }
    let pool = sentences(Path::new(&arguments[0]));
    let out_dir = PathBuf::from(&arguments[1]);
    let lectures: usize = arguments.get(2).and_then(|value| value.parse().ok()).unwrap_or(100);
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    let (rows, terms) = corpus(&pool, lectures);
    let characters: usize = rows.iter().map(|row| row.body.chars().count()).sum();
    println!("sentences {} segments {} characters {}", pool.len(), rows.len(), characters);

    let set = queries(&rows, &terms);
    let mut indexes = serde_json::Map::new();
    let mut results = Vec::new();
    for method in Method::all() {
        let path = out_dir.join(format!("corpus-{}.db", method.label()));
        let _ = std::fs::remove_file(&path);
        let connection = open(&path)?;
        create_schema(&connection, method)?;
        let started = Instant::now();
        insert_segments(&connection, &rows)?;
        rebuild_index(&connection, method)?;
        let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        drop(connection);
        let bytes = std::fs::metadata(&path).map(|data| data.len()).unwrap_or(0);
        indexes.insert(
            method.label().to_string(),
            serde_json::json!({"index_seconds": seconds, "file_mib": (bytes as f64 / 1_048_576.0 * 10.0).round() / 10.0}),
        );
        println!("{} index {seconds:.1}s file {:.1} MiB", method.label(), bytes as f64 / 1_048_576.0);

        let connection = open(&path)?;
        for (kind, query) in &set {
            let expected = truth(&rows, query);
            let mut names: Vec<(&str, Vec<i64>, Vec<f64>)> = Vec::new();
            let mut found = Vec::new();
            let mut times = Vec::new();
            for _ in 0..REPEATS {
                let started = Instant::now();
                found = search(&connection, method, query, LIMIT)?;
                times.push(milliseconds(started));
            }
            names.push((method.label(), found, times));
            if method == Method::Unicode61 {
                let mut prefix_found = Vec::new();
                let mut prefix_times = Vec::new();
                for _ in 0..REPEATS {
                    let started = Instant::now();
                    prefix_found = search_prefix(&connection, query, LIMIT)?;
                    prefix_times.push(milliseconds(started));
                }
                names.push(("unicode61-prefix", prefix_found, prefix_times));
            }
            for (label, found, mut times) in names {
                times.sort_by(|left, right| left.partial_cmp(right).unwrap());
                results.push(serde_json::json!({
                    "kind": kind,
                    "query": query,
                    "method": label,
                    "truth": expected.len(),
                    "found": found.len(),
                    "recall": (recall(&found, &expected, LIMIT) * 1000.0).round() / 1000.0,
                    "precision": (precision(&found, &expected) * 1000.0).round() / 1000.0,
                    "median_ms": times[times.len() / 2],
                    "max_ms": times[times.len() - 1],
                }));
            }
        }
    }

    let report = serde_json::json!({
        "lectures": lectures,
        "segments": rows.len(),
        "characters": characters,
        "limit": LIMIT,
        "repeats": REPEATS,
        "indexes": indexes,
        "queries": results,
    });
    let path = out_dir.join("search-bench.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    println!("written to {}", path.display());
    Ok(())
}
```

- [ ] **Step 2: 작은 규모로 동작 확인**

Run:

```bash
export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo run --release --example search_bench -- "C:\temp_git\Just-a-Click-\downloads\fleurs\ko_kr\test.tsv" "C:\temp_git\Just-a-Click-\artifacts\app-search\smoke" 3
```

Expected: `segments`와 방식별 색인 줄이 나오고 `search-bench.json`이 만들어진다.

- [ ] **Step 3: 커밋**

```bash
git add app/src-tauri/examples/search_bench.rs
git commit -m "test: add the Korean search comparison harness" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: 자격 증명 저장소

**Files:**
- Create: `app/src-tauri/src/secrets.rs`
- Modify: `app/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: 없음.
- Produces: `TARGET_PREFIX`, `target(&str) -> String`, `save(&str,&str) -> Result<(),String>`, `load(&str) -> Result<Option<String>,String>`, `delete(&str) -> Result<(),String>`.

- [ ] **Step 1: 실패하는 테스트 작성**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Only ever the test entry, never the real one.
    fn test_target() -> String {
        format!("{TARGET_PREFIX}.test/notion")
    }

    #[test]
    fn the_target_carries_the_product_prefix() {
        assert_eq!(target("notion"), "dev.justaclick/notion");
    }

    #[test]
    fn a_secret_survives_a_round_trip_and_disappears_after_delete() {
        let name = test_target();
        let _ = delete(&name);
        assert_eq!(load(&name).expect("load"), None);
        save(&name, "secret-token-value").expect("save");
        assert_eq!(load(&name).expect("load"), Some("secret-token-value".to_string()));
        delete(&name).expect("delete");
        assert_eq!(load(&name).expect("load"), None);
    }

    #[test]
    fn deleting_twice_is_not_an_error() {
        let name = format!("{TARGET_PREFIX}.test/missing");
        delete(&name).expect("first delete");
        delete(&name).expect("second delete");
    }

    /// Run on its own, then `a_saved_secret_is_there_in_a_new_process` in a second process.
    #[test]
    #[ignore = "persistence check across processes; see Task 11"]
    fn persistence_writes_the_secret() {
        save(&format!("{TARGET_PREFIX}.test/persist"), "persisted-value").expect("save");
    }

    #[test]
    #[ignore = "persistence check across processes; see Task 11"]
    fn a_saved_secret_is_there_in_a_new_process() {
        let name = format!("{TARGET_PREFIX}.test/persist");
        let found = load(&name).expect("load");
        delete(&name).expect("delete");
        assert_eq!(found, Some("persisted-value".to_string()));
    }

    #[test]
    fn a_korean_secret_survives_the_round_trip() {
        let name = format!("{TARGET_PREFIX}.test/korean");
        save(&name, "비밀-토큰").expect("save");
        assert_eq!(load(&name).expect("load"), Some("비밀-토큰".to_string()));
        delete(&name).expect("delete");
    }
}
```

- [ ] **Step 2: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test secrets`
Expected: 컴파일 실패(`cannot find function target`).

- [ ] **Step 3: 구현 작성**

```rust
//! Tokens live in the Windows credential manager, never in files, logs or the database.
use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

pub const TARGET_PREFIX: &str = "dev.justaclick";
/// HRESULT 0x80070490: the credential manager has no such entry.
const NOT_FOUND: i32 = -2_147_023_728;

pub fn target(name: &str) -> String {
    format!("{TARGET_PREFIX}/{name}")
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn save(target: &str, secret: &str) -> Result<(), String> {
    let name = wide(target);
    let user = wide("just-a-click");
    let mut blob = secret.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_ptr() as *mut u16),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_ptr() as *mut u16),
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }.map_err(|error| format!("could not save: {error}"))
}

pub fn load(target: &str) -> Result<Option<String>, String> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    match unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut found) } {
        Ok(()) => {}
        Err(error) if error.code().0 == NOT_FOUND => return Ok(None),
        Err(error) => return Err(format!("could not read: {error}")),
    }
    let secret = unsafe {
        let blob = std::slice::from_raw_parts((*found).CredentialBlob, (*found).CredentialBlobSize as usize);
        String::from_utf8(blob.to_vec())
    };
    unsafe { CredFree(found as *const c_void) };
    secret
        .map(Some)
        .map_err(|error| format!("the stored value is not text: {error}"))
}

/// Deleting an entry that is already gone is not an error: disconnecting twice is fine.
pub fn delete(target: &str) -> Result<(), String> {
    let name = wide(target);
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(error) if error.code().0 == NOT_FOUND => Ok(()),
        Err(error) => Err(format!("could not delete: {error}")),
    }
}
```

`app/src-tauri/src/lib.rs`에 `pub mod secrets;`를 더한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test secrets`
Expected: `4 passed`

Run: `powershell -NoProfile -Command "cmdkey /list | Select-String justaclick"`
Expected: 출력 없음(테스트가 만든 항목을 모두 지웠다)

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/secrets.rs app/src-tauri/src/lib.rs
git commit -m "feat: keep the Notion token in the Windows credential manager" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: 추론 코디네이터와 Tauri 명령

**Files:**
- Create: `app/src-tauri/src/inference.rs`
- Modify: `app/src-tauri/src/lib.rs`, `app/src-tauri/src/process.rs`

**Interfaces:**
- Consumes: `process::ProcessGroup`, `llm::{Server, ServerSettings, Stop, stream_draft}`, `stt::{Outcome, TranscribeSettings, run}`, `store`, `search`, `secrets`.
- Produces: Tauri 명령 `start_inference`, `inference_status`, `run_draft`, `cancel_draft`, `run_transcribe`, `cancel_transcribe`, `stop_inference`, `search_notes`, `save_notion_token`, `notion_token_saved`, `delete_notion_token`.

- [ ] **Step 1: `ProcessGroup`을 스레드 사이에서 쓸 수 있게 표시**

`app/src-tauri/src/process.rs`의 `impl Drop for ProcessGroup` 바로 위에 다음을 넣는다.

```rust
// A job object handle is process-wide and the Win32 calls on it are thread-safe, so the
// group can be shared with the worker threads that start the children.
unsafe impl Send for ProcessGroup {}
unsafe impl Sync for ProcessGroup {}
```

- [ ] **Step 2: 실패하는 테스트 작성**

`app/src-tauri/src/inference.rs`에 테스트를 먼저 넣는다.

```rust
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
```

- [ ] **Step 3: 테스트가 실패하는지 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test inference`
Expected: 컴파일 실패(`cannot find type Shared`).

- [ ] **Step 4: 구현 작성**

```rust
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
```

`app/src-tauri/src/lib.rs`에 `pub mod inference;`를 더하고, 명령과 상태 등록을 추가한다.

```rust
#[tauri::command]
fn start_inference(root: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    let root = std::path::PathBuf::from(root);
    let settings = llm::ServerSettings {
        executable: root.join("runtimes/b10994/vulkan/llama-server.exe"),
        model: root.join("models/Qwen3-8B-Q5_K_M.gguf"),
        log: root.join("artifacts/app-inference/screen-server.log"),
        context_tokens: 8192,
        threads: 2,
    };
    state.lock().map_err(|error| error.to_string())?.start(settings);
    Ok(())
}

#[tauri::command]
fn inference_status(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<inference::Status, String> {
    Ok(state.lock().map_err(|error| error.to_string())?.shared.read())
}

#[tauri::command]
fn run_draft(prompt: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state.lock().map_err(|error| error.to_string())?.draft(prompt, 300);
    Ok(())
}

#[tauri::command]
fn cancel_draft(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state
        .lock()
        .map_err(|error| error.to_string())?
        .draft_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn run_transcribe(root: String, chunk: String, state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    let root = std::path::PathBuf::from(root);
    let chunk = std::path::PathBuf::from(chunk);
    let settings = stt::TranscribeSettings {
        executable: root.join("runtimes/whisper-b5130/blas/Release/whisper-cli.exe"),
        model: root.join("models/whisper/ggml-large-v3-turbo.bin"),
        output: chunk.with_extension("screen"),
        chunk,
        threads: 8,
    };
    state.lock().map_err(|error| error.to_string())?.transcribe(settings);
    Ok(())
}

#[tauri::command]
fn cancel_transcribe(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state
        .lock()
        .map_err(|error| error.to_string())?
        .transcribe_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn stop_inference(state: tauri::State<'_, Mutex<inference::Inference>>) -> Result<(), String> {
    state.lock().map_err(|error| error.to_string())?.stop();
    Ok(())
}

#[tauri::command]
fn search_notes(database: String, query: String, method: String, limit: usize) -> Result<Vec<i64>, String> {
    let method = store::Method::from_label(&method).ok_or("알 수 없는 검색 방식입니다")?;
    let connection = store::open(std::path::Path::new(&database))?;
    search::search(&connection, method, &query, limit)
}

#[tauri::command]
fn save_notion_token(value: String) -> Result<(), String> {
    secrets::save(&secrets::target("notion"), &value)
}

#[tauri::command]
fn notion_token_saved() -> Result<bool, String> {
    Ok(secrets::load(&secrets::target("notion"))?.is_some())
}

#[tauri::command]
fn delete_notion_token() -> Result<(), String> {
    secrets::delete(&secrets::target("notion"))
}
```

`run()`의 `.manage(...)`에 `.manage(Mutex::new(inference::Inference::default()))`를 더하고, `tauri::generate_handler![...]`에 위 명령 열한 개를 더한다.

- [ ] **Step 5: 테스트와 빌드 확인**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test`
Expected: `51 passed`(1차 22개 + Task 1~9에서 더한 29개), 실패 0

- [ ] **Step 6: 커밋**

```bash
git add app/src-tauri/src/inference.rs app/src-tauri/src/process.rs app/src-tauri/src/lib.rs
git commit -m "feat: drive the inference runtimes from the screen" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: 검증 화면 탭

**Files:**
- Create: `app/src/RecordingTab.tsx`, `app/src/InferenceTab.tsx`, `app/src/SearchTab.tsx`, `app/src/SecretsTab.tsx`
- Modify: `app/src/App.tsx`

**Interfaces:**
- Consumes: Task 9의 명령과 1차의 녹음 명령.
- Produces: 탭 네 개가 있는 검증 화면.

- [ ] **Step 1: 녹음 화면을 탭으로 옮기기**

`app/src/App.tsx`의 현재 본문(상태 훅, `run` 도우미, 입력·제어·상태·기록 부분)을 그대로 `app/src/RecordingTab.tsx`로 옮기고 `export default function RecordingTab() { … }`으로 감싼다. 동작은 바꾸지 않는다.

- [ ] **Step 2: `App.tsx`를 탭 전환만 하도록 바꾸기**

```tsx
import { useState } from "react";
import RecordingTab from "./RecordingTab";
import InferenceTab from "./InferenceTab";
import SearchTab from "./SearchTab";
import SecretsTab from "./SecretsTab";

const TABS = [
  { id: "recording", label: "녹음", view: <RecordingTab /> },
  { id: "inference", label: "추론", view: <InferenceTab /> },
  { id: "search", label: "검색", view: <SearchTab /> },
  { id: "secrets", label: "자격 증명", view: <SecretsTab /> },
];

function App() {
  const [active, setActive] = useState("recording");
  return (
    <main style={{ padding: 16, fontFamily: "system-ui, sans-serif" }}>
      <nav style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        {TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActive(tab.id)}
            style={{ fontWeight: active === tab.id ? 700 : 400 }}
          >
            {tab.label}
          </button>
        ))}
      </nav>
      {TABS.find((tab) => tab.id === active)?.view}
    </main>
  );
}

export default App;
```

- [ ] **Step 3: 추론 탭**

`app/src/InferenceTab.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Status = {
  phase: string;
  ready_seconds: number;
  port: number;
  last_answer: string;
  error: string | null;
};

const ROOT = "C:\\temp_git\\Just-a-Click-";

export default function InferenceTab() {
  const [status, setStatus] = useState<Status | null>(null);
  const [chunk, setChunk] = useState("");
  const [log, setLog] = useState<string[]>([]);

  const note = (text: string) =>
    setLog((entries) => [`${new Date().toLocaleTimeString()} ${text}`, ...entries].slice(0, 20));

  const call = async (command: string, args: Record<string, unknown> = {}) => {
    try {
      await invoke(command, args);
      note(`${command} 보냄`);
    } catch (error) {
      note(`${command} 실패: ${error}`);
    }
  };

  useEffect(() => {
    const timer = setInterval(async () => {
      try {
        setStatus(await invoke<Status>("inference_status"));
      } catch (error) {
        note(`상태 조회 실패: ${error}`);
      }
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  return (
    <section>
      <h2>추론 프로세스</h2>
      <p>
        단계 {status?.phase ?? "-"} · 준비 {status?.ready_seconds?.toFixed(1) ?? "0.0"}초 · 포트{" "}
        {status?.port ?? 0}
      </p>
      {status?.error ? <p style={{ color: "#c00" }}>오류: {status.error}</p> : null}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button onClick={() => call("start_inference", { root: ROOT })}>서버 시작</button>
        <button onClick={() => call("run_draft", { prompt: "다음 강의 구간을 요약해 주세요." })}>
          초안 1건
        </button>
        <button onClick={() => call("cancel_draft")}>초안 취소</button>
        <button onClick={() => call("run_transcribe", { root: ROOT, chunk })}>조각 전사</button>
        <button onClick={() => call("cancel_transcribe")}>전사 취소</button>
        <button onClick={() => call("stop_inference")}>서버 정지</button>
      </div>
      <label style={{ display: "block", marginTop: 8 }}>
        전사할 조각 경로
        <input value={chunk} onChange={(event) => setChunk(event.currentTarget.value)} style={{ width: "100%" }} />
      </label>
      <h3>마지막 결과</h3>
      <p style={{ whiteSpace: "pre-wrap" }}>{status?.last_answer || "없음"}</p>
      <h3>기록</h3>
      <ul>
        {log.map((entry) => (
          <li key={entry}>{entry}</li>
        ))}
      </ul>
    </section>
  );
}
```

- [ ] **Step 4: 검색 탭**

`app/src/SearchTab.tsx`:

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const METHODS = ["trigram", "unicode61", "like"];

export default function SearchTab() {
  const [database, setDatabase] = useState("");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Record<string, number[]>>({});
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setError(null);
    const found: Record<string, number[]> = {};
    for (const method of METHODS) {
      try {
        found[method] = await invoke<number[]>("search_notes", {
          database: database.replace("<방식>", method),
          query,
          method,
          limit: 20,
        });
      } catch (caught) {
        setError(String(caught));
        found[method] = [];
      }
    }
    setResults(found);
  };

  return (
    <section>
      <h2>한국어 검색</h2>
      <label style={{ display: "block" }}>
        DB 경로(방식 자리에는 <code>&lt;방식&gt;</code>을 넣는다)
        <input
          value={database}
          placeholder="C:\녹음\검증\corpus-<방식>.db"
          onChange={(event) => setDatabase(event.currentTarget.value)}
          style={{ width: "100%" }}
        />
      </label>
      <label style={{ display: "block", marginTop: 8 }}>
        검색어
        <input value={query} onChange={(event) => setQuery(event.currentTarget.value)} style={{ width: "100%" }} />
      </label>
      <button onClick={run} disabled={database.length === 0 || query.length === 0} style={{ marginTop: 8 }}>
        세 방식으로 검색
      </button>
      {error ? <p style={{ color: "#c00" }}>오류: {error}</p> : null}
      <ul>
        {METHODS.map((method) => (
          <li key={method}>
            {method}: {results[method]?.length ?? 0}건 {results[method]?.slice(0, 5).join(", ")}
          </li>
        ))}
      </ul>
    </section>
  );
}
```

- [ ] **Step 5: 자격 증명 탭**

`app/src/SecretsTab.tsx`:

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function SecretsTab() {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setSaved(await invoke<boolean>("notion_token_saved"));
      setError(null);
    } catch (caught) {
      setError(String(caught));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const save = async () => {
    try {
      await invoke("save_notion_token", { value });
      setValue("");
      await refresh();
    } catch (caught) {
      setError(String(caught));
    }
  };

  const remove = async () => {
    try {
      await invoke("delete_notion_token");
      await refresh();
    } catch (caught) {
      setError(String(caught));
    }
  };

  return (
    <section>
      <h2>자격 증명</h2>
      <p>저장 위치: Windows 자격 증명 관리자 <code>dev.justaclick/notion</code></p>
      <p>현재 상태: {saved === null ? "확인 중" : saved ? "저장됨" : "없음"}</p>
      <label style={{ display: "block" }}>
        Notion 내부 연결 토큰
        <input
          type="password"
          value={value}
          onChange={(event) => setValue(event.currentTarget.value)}
          style={{ width: "100%" }}
        />
      </label>
      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        <button onClick={save} disabled={value.length === 0}>
          저장
        </button>
        <button onClick={remove}>삭제</button>
      </div>
      {error ? <p style={{ color: "#c00" }}>오류: {error}</p> : null}
      <p>화면과 기록에는 토큰 값을 표시하지 않는다.</p>
    </section>
  );
}
```

- [ ] **Step 6: 타입 검사와 실행 확인**

Run: `cd app && npm run typecheck`
Expected: 오류 없음

Run: `cd app && npm run tauri dev`
Expected: 창이 뜨고 탭 네 개가 보인다. 확인 뒤 창을 닫는다.

- [ ] **Step 7: 커밋**

```bash
git add app/src/App.tsx app/src/RecordingTab.tsx app/src/InferenceTab.tsx app/src/SearchTab.tsx app/src/SecretsTab.tsx
git commit -m "feat: add the inference, search and credential tabs" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: 검증 실행과 기록

**Files:**
- Create: `docs/validation/<날짜>-app-stack-2.md`
- Modify: `docs/decisions/0007-app-stack.md`, `docs/ROADMAP.md`, `README.md`

**Interfaces:**
- Consumes: Task 1~10의 모듈과 하네스.
- Produces: ③④⑤의 판정과 2차 검증 기록.

- [ ] **Step 1: 전원과 배경 상태 기록**

Run: `python -c "import json, scripts.bench_env as e; print(json.dumps({'power': e.power_status(), 'mode': e.power_mode(), 'competing': e.competing_processes([])}, ensure_ascii=False))"`
Expected: `ac_power`가 참이고 `ac_mode`가 `best_performance`다. 아니면 사용자에게 전원 모드 변경을 요청한다.

- [ ] **Step 2: ③ 측정**

Run:

```bash
export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo run --release --example inference_check -- "C:\temp_git\Just-a-Click-" "C:\temp_git\Just-a-Click-\artifacts\app-recordings\long-30m\chunk-0001.wav" "C:\temp_git\Just-a-Click-\artifacts\app-inference"
```

Expected: JSON에 `auth`가 `{no_key: 401, wrong_key: 401, right_key: 200}`, `lan_blocked: true`, `draft_cancel.stop: "Cancelled"`, `draft_after_cancel.stop: "Finished"`, `transcribe.characters > 0`, `transcribe_cancel.leftover_file: false`, `crash_recovery.alive_after_kill: false`가 담긴다. 다른 값이 나오면 그대로 기록한다.

- [ ] **Step 3: 고아 방지 확인**

Run(백그라운드): `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo run --release --example inference_check -- "C:\temp_git\Just-a-Click-" --hold 180`

서버가 뜬 뒤(`held server pid …` 출력) 하네스를 강제 종료하고 남은 프로세스를 센다.

```bash
powershell -NoProfile -Command "Stop-Process -Name inference_check -Force; Start-Sleep -Seconds 2; (Get-Process llama-server -ErrorAction SilentlyContinue | Measure-Object).Count"
```

Expected: `0`

- [ ] **Step 4: ④ 측정**

Run:

```bash
export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo run --release --example search_bench -- "C:\temp_git\Just-a-Click-\downloads\fleurs\ko_kr\test.tsv" "C:\temp_git\Just-a-Click-\artifacts\app-search\run" 100
```

Expected: `search-bench.json`에 질의 종류별 `recall`·`precision`·`median_ms`와 방식별 색인 시간·파일 크기가 담긴다.

- [ ] **Step 5: ⑤ 측정**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test secrets -- --nocapture`
Expected: `4 passed`

Run(두 번에 나눠 서로 다른 프로세스로 실행한다):

```bash
export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test secrets::tests::persistence_writes_the_secret -- --ignored --exact
export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test secrets::tests::a_saved_secret_is_there_in_a_new_process -- --ignored --exact
```

Expected: 두 번째 명령이 `1 passed`다(첫 프로세스가 저장한 값을 새 프로세스가 읽고 지웠다)

Run: `powershell -NoProfile -Command "cmdkey /list | Select-String justaclick"`
Expected: 출력 없음

Run: `grep -ri "secret-token-value" app artifacts --include="*.log" --include="*.json" --include="*.db" | head`
Expected: 결과 없음(토큰이 로그·측정 파일·DB에 남지 않았다)

- [ ] **Step 6: 보고서 작성**

`docs/validation/<날짜>-app-stack-2.md`를 다음 구조로 쓴다. 수치는 앞 단계의 JSON에서 옮기고, 통과하지 못한 항목은 실패로 적는다.

````markdown
# 앱 기술 검증 2차: 추론 프로세스·한국어 검색·자격 증명

- 날짜, 범위(결정 0007 ③④⑤), 기기·전원 조건, 결과 파일 위치, 설계·계획 링크
## 1. 요약
③④⑤의 통과 여부를 3~5문장으로.
## 2. 환경과 버전
Rust·Tauri·rusqlite(SQLite 버전)·reqwest·windows 크레이트와 라이선스, 런타임·모델 태그.
## 3. 추론 프로세스
준비 시간, 인증 세 가지 응답, LAN 접근 차단, 취소 시간과 이어진 요청, 전사와 전사 취소, 강제 종료 복구, 고아 방지, 메모리 3회.
## 4. 한국어 검색
코퍼스 규모, 방식별 색인 시간·파일 크기, 질의 종류별 재현율·정확도·지연 표, 권고 조합.
## 5. 자격 증명
왕복·지속·삭제 결과와 누출 확인 방법.
## 6. 발견한 제약
trigram의 세 글자 제한처럼 제품 설계에 영향을 주는 사실.
## 7. 판정과 다음 결정
③④⑤ 각각의 판정과 남은 미검증 항목.
## 8. 한계
기기 1대, Windows 11만, 공개 문장 기반 코퍼스, 실제 강의·실기기 16GB 미검증.
````

- [ ] **Step 7: 결정·로드맵·README 갱신**

`docs/decisions/0007-app-stack.md` 3절 끝에 2차 결과 문단을 더한다(1차 문단과 같은 형식). 상태 줄도 2차 결과에 맞게 고친다.

`docs/ROADMAP.md` 남은 작업 7에 2차 결과와 남은 미검증 항목을 더한다. ①~⑤가 모두 끝나고 미검증 항목이 로드맵의 다른 작업으로 넘어갔다면 `[x]`로 바꾼다.

`README.md` 문서 목록에 다음을 더한다.

```markdown
- [앱 기술 검증 2차: 추론 프로세스·한국어 검색·자격 증명](docs/validation/<날짜>-app-stack-2.md)
```

- [ ] **Step 8: 검증 후 커밋**

Run: `export PATH="$PATH:/c/Users/jingg/.cargo/bin" && cd app/src-tauri && cargo test`
Expected: 실패 0

Run: `python artifacts/check_docs.py`
Expected: `local paths: none`, `broken links: none`

Run: `git status --short`
Expected: 문서만 변경되어 있고 `artifacts/`, `app/target`, `app/node_modules`는 보이지 않는다

```bash
git add docs/validation/<날짜>-app-stack-2.md docs/decisions/0007-app-stack.md docs/ROADMAP.md README.md
git commit -m "test: validate the inference processes, Korean search and the credential store" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9: 사용자 보고**

작업·결과·다음 형식으로 보고한다. 결과에는 ③④⑤의 판정, 검색 방식 권고, 발견한 제약, 미검증으로 남긴 항목을 넣는다.
