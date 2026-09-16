# 0002 — 실행 구조와 모델 검증 후보

- 날짜: 2026-09-16
- 상태: 초기 후보 조사 기록. LLM은 이후 [0004](0004-llm-selection.md)에서 Qwen3-8B Q5_K_M으로 확정했으며 아래 후보 비교는 당시 기록이다. UI·STT와 실제 추론 검증은 미완료
- 전제: Windows 10·11 exe, 별도 Install Manager, 최대 4시간 녹음

## 실행 구조 제안

앱 UI → 작업 관리자 → 로컬 STT·LLM 프로세스 → 구조화된 결과 저장으로 구성한다. 녹음은 작은 구간으로 디스크에 저장하고, 추론 프로세스는 UI와 분리한다. STT와 LLM은 우선 순차 실행하여 메모리 경합을 줄인다. 이 구조의 성능 효과는 실제 측정한다.

| 영역 | 우선 검증 후보 | 선택 이유와 미확정 사항 |
| --- | --- | --- |
| 데스크톱 셸 | Tauri 2 + TypeScript UI | Windows WebView2와 외부 실행 파일 연동이 문서화되어 있음. Rust 도구 설치 및 Windows 10 검증 필요 |
| 대안 셸 | Windows 네이티브 .NET UI | 웹 UI가 필요하지 않다면 비교할 대안. 이번 조사에서는 SDK·패키징·호환성 미검증 |
| STT 런타임 | whisper.cpp | 로컬 CLI와 CPU 기준 실행, Vulkan·OpenVINO 가속 경로 비교 가능 |
| LLM 런타임 | llama.cpp | 로컬 실행과 양자화 모델 활용, CPU·Vulkan·SYCL 경로 비교 가능 |
| 저장 | 로컬 파일 + SQLite | 원본 미디어와 작업 메타데이터 분리. 라이브러리·마이그레이션 도구 선택은 앱 스택 확정 후 진행 |
| 설치 관리 | 별도 Install Manager | 설치 중 다운로드로 확정. 앱·런타임·모델 버전과 체크섬 관리 |

Tauri의 Windows 개발에는 MSVC 빌드 도구·Rust·WebView2 준비가 필요하며 외부 바이너리를 sidecar로 연결할 수 있다. 앱 셸과 추론 프로세스를 나누는 후보로 검토한다. [Tauri 사전 요구사항](https://tauri.app/start/prerequisites/), [외부 바이너리 연동](https://v2.tauri.app/develop/sidecar/).

whisper.cpp는 CPU 실행 외에 Vulkan과 OpenVINO 경로를 문서화한다. Windows에서의 SYCL 경로는 문서상 진행 중이므로 STT의 첫 가속 후보로 고정하지 않는다. [whisper.cpp](https://github.com/ggml-org/whisper.cpp/blob/master/README.md), [SYCL 상태](https://github.com/ggml-org/whisper.cpp/blob/master/README_sycl.md).

llama.cpp는 CPU 및 Vulkan·SYCL 백엔드를 제공한다. 해당 기능의 존재가 현재 Arc 140V에서의 안정성이나 성능을 보장하지는 않는다. NPU는 초기 필수 의존성으로 두지 않는다. [llama.cpp](https://github.com/ggml-org/llama.cpp).

## 모델 비교 범위

- STT: 다국어 Whisper small을 초기 실행 기준으로, medium과 large-v3-turbo를 품질 비교 후보로 검토한다. 영어 전용 `.en` 모델은 한국어 평가에 사용하지 않는다. Whisper 코드와 가중치는 MIT 라이선스로 공개되어 있다. [Whisper 공식 저장소](https://github.com/openai/whisper).
- LLM: Qwen3-4B와 Qwen3-8B의 GGUF 양자화 실행을 비교한다. 공식 원본 모델 카드는 Apache-2.0 라이선스를 명시한다. 초기에는 사고 모드를 끈 요약을 비교하되 실제 런타임의 템플릿 적용을 확인한다. [Qwen3-4B](https://huggingface.co/Qwen/Qwen3-4B), [Qwen3-8B](https://huggingface.co/Qwen/Qwen3-8B).
- 위 모델은 평가 기준을 만들기 위한 후보이며 최신·최고 모델이라는 주장이 아니다. 한국어 요약 적합성은 직접 평가한다.
- 다운로드 전에 정확한 모델 revision, 양자화 파일, 공급자, 체크섬, 라이선스·고지 파일을 기록한다. 원본 라이선스 확인만으로 모든 변환 배포물의 출처 확인을 대신하지 않는다.
- 모델 크기만으로 실행 메모리를 추정하지 않는다. 컨텍스트·KV 캐시·작업 버퍼 및 통합 GPU의 메모리 사용을 함께 측정한다.

## 현재 개발 환경

직접 조회 결과 Python 3.11.9, Node.js v22.15.0, CMake 4.3.2 및 Git 실행 파일이 확인되었다. Rust·Cargo는 현재 PATH에서 발견되지 않았다. MSVC·WebView2·Vulkan 런타임의 실제 빌드·실행 가능 여부는 아직 검증하지 않았다. 발견된 도구 버전은 프로젝트 고정 버전이 아니다.

## 확정하지 않은 사항

- UI 스택과 라이브러리 버전, Windows 최소 빌드, GPU 가속 백엔드.
- 허용 설치 용량. 공급 방식은 설치 중 다운로드로 확정했다.
- 4시간 녹음의 요약 목표는 전사 완료 후 요약 실행부터 결과 저장까지 최대 120초·품질 우선으로 확정되었다. 실측 전 달성을 보장하지 않는다.
- Notion 인증과 무료 검색의 실행 가능성. 별도 M0 조사 항목으로 유지한다.

실제 검증 절차는 [M0 기술 검증 계획](../M0-VALIDATION.md)을 따른다.
