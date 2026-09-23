# 앱 기술 검증 1차 설계: Tauri 2 빌드와 녹음

- 날짜: 2026-09-23
- 상태: 사용자 승인 설계. 구현·검증 전
- 대상 작업: [로드맵](../../ROADMAP.md) M0 남은 작업 7 중 [결정 0007](../../decisions/0007-app-stack.md) 3절의 ①(Rust·MSVC 빌드와 Tauri 2·WebView2)과 ②(마이크·시스템 소리 녹음, 일시정지, 구간 파일 저장, 절전 방지)
- 2차 범위(별도 설계): ③ 추론 프로세스 관리, ④ SQLite FTS5 한국어 검색, ⑤ Windows 자격 증명 관리자

## 1. 목적과 범위

기준 노트북에서 Tauri 2 앱이 다음을 할 수 있는지 확인한다.

1. Windows에서 개발 실행과 릴리스 빌드가 되고 WebView2에서 화면이 뜬다.
2. 마이크와 시스템 소리(WASAPI 루프백)를 녹음해 STT가 바로 쓸 수 있는 30초 조각 WAV를 만든다.
3. 일시정지·재개가 조각을 깨지 않는다.
4. 녹음 중 절전 진입을 막는다.

범위 밖: STT·LLM 연동, 데이터베이스, 노트 생성, 자격 증명 저장, Windows 10 확인, 4시간 연속 녹음, 화면 디자인.

## 2. 사용자 결정 사항

| 항목 | 결정 |
| --- | --- |
| 툴체인 설치 | rustup(MSVC 툴체인), Tauri CLI, React·TypeScript·Vite 의존성까지 설치한다 |
| 산출물 | 검증용 앱을 저장소 `app/`에 골격으로 남기고 A 단계에서 계속 키운다 |
| 녹음 검증 깊이 | 마이크 60초, 시스템 소리 60초, 일시정지·재개, 30분 연속 1회. 조각은 whisper-cli로 전사해 확인한다 |
| 범위 분할 | 1차는 결정 0007의 ①②, 2차는 ③④⑤ |

## 3. 확인한 사전 조건 (2026-09-23)

- Node.js 22.15.0, npm 10.9.2가 설치돼 있다.
- WebView2 런타임 153.0.4234.48이 설치돼 있다(`HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients`).
- Visual Studio 2022 Community의 MSVC 14.43.34808과 Windows SDK 10.0.22621.0이 있다. Rust MSVC 타깃의 링커 조건을 만족한다.
- Rust 툴체인은 없다. rustup은 사용자 폴더에 설치되며 관리자 권한이 필요하지 않다.
- 크레이트 조사(crates.io API): `tauri` 2.11.6(Apache-2.0 OR MIT, 2026-09-21 갱신), `wasapi` 0.24.0(MIT, 2026-08-12), `windows` 0.62.2(MIT OR Apache-2.0). `wasapi`는 README에 루프백 캡처, 이벤트 기반 버퍼링, 장치 알림을 지원한다고 적고 있다.
- `wasapi` 0.24의 사용 방식: `DeviceEnumerator::new()` → `get_default_device(&Direction::Capture)`(마이크) 또는 `&Direction::Render`(시스템 소리 루프백) → `get_iaudioclient()` → `initialize_client(&WaveFormat::new(16, 16, &SampleType::Int, 16000, 1, None), &direction, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns })` → `set_get_eventhandle()` → `get_audiocaptureclient()`. `autoconvert: true`면 오디오 엔진이 16kHz·모노·PCM16으로 변환해 주므로 앱에서 재샘플링하지 않는다.
- `cpal` 0.18.2는 README·변경 기록에서 루프백을 macOS에만 명시한다. 그래서 마이크·시스템 소리를 `wasapi` 하나로 처리한다.

## 4. 성공 기준

- **빌드:** `npm run tauri dev`로 창이 뜨고 검증 화면이 보인다. `npm run tauri build`가 성공하고 설치 파일이 만들어진다. WebView2 버전을 기록한다. Windows 10은 기기가 없어 미검증으로 남긴다.
- **조각 파일:** 마이크·시스템 소리 각각 60초 녹음에서 30초 조각 WAV가 2개 만들어지고, 형식이 16kHz·모노·16비트 PCM이며 길이가 30초(±0.2초)다. whisper-cli(large-v3-turbo f16, OpenBLAS)로 전사해 사람이 읽을 수 있는 한국어가 나온다. 품질 판정은 하지 않는다.
- **일시정지:** 30초 녹음 → 10초 정지 → 60초 녹음에서 조각 3개가 나오고, 정지 구간의 소리가 조각에 들어가지 않는다.
- **30분 연속:** 조각 60개가 빠짐없이 생기고 마지막 조각만 30초 미만일 수 있다. WASAPI 불연속(`AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`) 횟수와 앱 프로세스 메모리를 기록한다. 불연속 0건이면 통과, 있으면 횟수와 시각을 기록해 판단한다.
- **절전 방지:** 녹음 중 `powercfg /requests`에 앱의 SYSTEM 요청이 나타나고 정지하면 사라진다.
- 항목이 실패하면 원인과 대안(다른 크레이트, C# .NET 재검토)을 결정 0007에 기록한다.

## 5. 앱 골격과 모듈

```
app/
  package.json, vite.config.ts, tsconfig.json, index.html
  src/            React·TypeScript 검증 화면
  src-tauri/
    Cargo.toml, tauri.conf.json, build.rs
    src/lib.rs    Tauri 명령과 상태
    src/audio.rs  장치 목록, 캡처 스레드
    src/chunker.rs 프레임 누적과 조각 경계
    src/wav.rs    PCM16 WAV 기록
    src/power.rs  절전 방지 가드
```

- `target/`, `node_modules/`, `dist/`는 추적하지 않는다.
- **Tauri 명령:** `list_audio_sources()`, `start_recording(source, directory)`, `pause_recording()`, `resume_recording()`, `stop_recording()`, `recording_status()`. 상태에는 소스, 상태값(`idle`·`recording`·`paused`·`stopped`·`failed`), 경과 초, 조각 수, 마지막 조각 경로, 불연속 횟수, 오류 메시지를 담는다.
- **캡처 구조:** 명령이 캡처 스레드를 띄우고, 스레드는 이벤트 대기 → 버퍼 읽기 → 조각 버퍼에 누적 → 30초가 차면 WAV로 쓰기를 반복한다. 일시정지·재개·정지는 원자 플래그로 전달한다. 상태는 `Arc<Mutex<RecorderState>>`로 공유한다.
- **조각 규칙:** 16,000샘플/초 × 30초 = 480,000샘플마다 새 파일. 파일명은 `chunk-0001.wav`부터 4자리 번호. 마지막 조각은 짧을 수 있다. 일시정지 동안 들어온 프레임은 버린다.
- **절전 방지:** 녹음 시작 시 `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)`를 호출하고 정지 시 `ES_CONTINUOUS`로 되돌린다. 화면 유지는 STT 측정에서 대기 모드 중단을 겪어 함께 요청한다. 전원 설정은 바꾸지 않는다.
- **화면:** 소스 선택(마이크·시스템 소리), 시작·일시정지·재개·정지 버튼, 상태 표시(경과 시간·조각 수·불연속·오류), 최근 로그 목록. A 단계의 녹음 화면으로 확장한다.

## 6. 검증 절차

사람이 참여해야 하는 절차다. 마이크 확인에는 말소리가, 시스템 소리 확인에는 재생 중인 소리가 필요하다.

1. 툴체인 설치와 `npm install` 뒤 `npm run tauri dev`로 창을 띄운다.
2. 마이크 60초 녹음(사용자가 말한다) → 조각 2개 → whisper-cli 전사.
3. 시스템 소리 60초 녹음(사용자가 영상·음원을 재생한다) → 조각 2개 → whisper-cli 전사.
4. 30초 녹음 → 10초 일시정지 → 60초 녹음 → 조각 3개와 경계 확인.
5. 30분 연속 녹음. 조각 60개, 불연속 횟수, 앱 메모리, `powercfg /requests`를 기록한다. 이 시간에는 노트북을 써도 되지만 무거운 작업은 피한다.
6. `npm run tauri build`로 릴리스 빌드와 설치 파일 생성을 확인한다.

## 7. 자동 테스트

오디오 장치가 없어도 통과해야 한다. `cd app/src-tauri && cargo test`로 실행한다.

- 조각 경계: 누적 프레임이 480,000에 닿을 때마다 새 조각이 열리고, 남은 프레임이 다음 조각으로 넘어간다.
- WAV 기록: 헤더 44바이트, 채널 1, 샘플레이트 16,000, 비트 16, 데이터 길이가 샘플 수 × 2다.
- 일시정지: 일시정지 상태에서 들어온 프레임은 조각에 쌓이지 않는다.
- 불연속: 불연속 플래그를 받은 횟수가 상태에 누적된다.
- 절전 가드: 생성 시 요청 1회, 해제 시 복원 1회를 호출한다(호출 함수를 주입해 확인).
- 장치 목록: `list_audio_sources`가 오류 없이 결과를 돌려준다(장치가 없으면 빈 목록).
- 프런트엔드는 `npm run typecheck`(`tsc --noEmit`)만 확인한다.

## 8. 기록과 오류 처리

- 녹음 조각은 `artifacts/app-recordings/<타임스탬프>/`에 저장하고 커밋하지 않는다. 사용자 음성은 저장소에 넣지 않는다.
- 검증 결과는 `docs/validation/<날짜>-app-stack.md`에 쓰고, 라이브러리 버전·라이선스와 판정은 결정 0007에 덧붙인다. 실패한 항목은 대안과 함께 남긴다.
- 장치 분리·기본 장치 변경은 `wasapi` 알림으로 받아 상태를 `failed`로 바꾸고 녹음을 안전하게 정지한다. 이미 쓴 조각은 보존한다. 수동 시험은 선택 항목으로 둔다.
- 장치 초기화 실패, 권한 거부, 디스크 쓰기 실패는 화면에 원인을 표시하고 상태에 남긴다.
- 캡처 스레드가 죽으면 상태가 `failed`가 되고 다시 시작할 수 있다.
- 툴체인 설치는 사용자 폴더에만 하고 시스템 설정을 바꾸지 않는다. 설치한 버전은 검증 보고서에 기록한다.

## 9. 후속 작업

- 2차 검증: 추론 프로세스 관리, SQLite FTS5 한국어 검색, 자격 증명 관리자 저장.
- Windows 10 기기 확보 후 빌드·녹음 확인.
- 4시간 연속 녹음과 디스크 사용량 측정(로드맵 남은 작업 5와 함께).
- 실제 강의 녹음으로 수직 흐름 검증(로드맵 남은 작업 4).
