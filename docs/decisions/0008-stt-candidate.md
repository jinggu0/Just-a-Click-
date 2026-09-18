# 0008 — 로컬 STT 1차 후보와 실행 설정

- 날짜: 2026-09-18
- 상태: 초안. 공개 낭독 음성(FLEURS 한국어) 기준 1차 후보이며 강의 녹음 평가 전이다. 출시 품질 확정이 아니다
- 근거 측정: [로컬 STT 측정 보고서](../validation/2026-09-18-stt-bench.md), [측정 설계](../superpowers/specs/2026-09-17-stt-benchmark-design.md)
- 관련 결정: [0002](0002-runtime-candidates.md) STT 런타임 후보, [0006](0006-lecture-first-product-scope.md) 제품 범위와 시간 목표

## 1. 결정

**A 단계의 1차 STT 후보를 Whisper `large-v3-turbo`(ggml f16)로 두고, 실행은 whisper.cpp `b5130` Windows x64 OpenBLAS 빌드·8스레드·`-nt`로 한다.** 확정이 아니라 다음 검증까지 유지하는 기준값이다.

| 항목 | 값 |
| --- | --- |
| 모델 | `ggml-large-v3-turbo.bin`(f16, 1,624,555,275 bytes), 저장소 `ggerganov/whisper.cpp` revision `5359861c739e955e79d9a303bcbc70fb988958b1`, 라이선스 MIT |
| 대안 | `ggml-large-v3-turbo-q5_0.bin`(574,041,195 bytes). CER 차이 0.0018로 동률 폭(0.005) 안이며 설치 용량이 3분의 1 |
| 런타임 | whisper.cpp `b5130`(commit `927cfce3`, MIT) Windows x64 OpenBLAS 빌드 |
| 실행 설정 | 8스레드, 언어 `ko`, 빔 5·후보 5(빌드 기본값), 타임스탬프 없이(`-nt`) |
| 측정값(공개 낭독 음성) | CER 0.063, 한글 CER 0.038, 조각 RTF 중앙값 0.422, 95번째 백분위 0.642 |

## 2. 선택 근거

- 속도 기준(조각 RTF 중앙값 ≤ 0.5, 95번째 백분위 ≤ 0.8)을 통과한 모델은 small q5_1, medium q5_0, large-v3-turbo q5_0, large-v3-turbo f16 4종이었다. 이 중 CER이 가장 낮은 두 모델의 차이가 동률 폭 안이라 더 빠른 f16을 골랐다.
- CER이 가장 낮은 모델은 large-v3 q5_0(0.050)이었지만 RTF 중앙값 0.747로 속도 기준을 넘었다. turbo f16과의 CER 차이는 0.013이다.
- 빌드·스레드는 결과 문자열을 바꾸지 않고 속도만 바꿨다. OpenBLAS·8스레드가 CPU 빌드·8스레드보다 RTF 중앙값이 34% 낮았다.
- 타임스탬프 방식은 같은 모델에서 CER을 27~34% 높이고 조각 뒷부분 누락을 만들었다. 그래서 기본 실행은 `-nt`로 둔다. 구간 시각이 필요하면 조각 경계나 음성 구간 검출로 얻는다.

## 3. 라이선스와 배포

- whisper.cpp: MIT([저장소 라이선스](https://github.com/ggml-org/whisper.cpp/blob/master/LICENSE)).
- Whisper ggml 모델: MIT(배포 저장소 카드 기준). 원본 Whisper 코드·가중치도 MIT다.
- OpenBLAS: BSD-3-Clause([저장소 라이선스](https://github.com/OpenMathLib/OpenBLAS/blob/develop/LICENSE)). 배포본에 동봉된 `libopenblas.dll`이 함께 포함하는 런타임 구성 요소의 고지 조건은 재배포 전에 확인해야 한다(미확인).
- 실행 파일은 공식 릴리스 태그 `b5130`의 zip을 크기·SHA-256으로 검증해 설치한다. 설정은 `config/stt-runtime.json`, `config/stt-models.json`에 고정했다.

## 4. 재검토 조건

다음 중 하나라도 확인되면 이 초안을 다시 판단한다.

- 강의 녹음 평가에서 전공 용어·잡음·긴 녹음 성능이 부족할 때.
- 녹음 중 STT와 LLM 동시 실행 측정(로드맵 M0 남은 작업 3)에서 CPU·메모리 경합으로 시간 목표를 넘길 때. 이번 측정에서 whisper는 8코어 중 약 6개를 썼다.
- 배터리 사용이나 "최고의 전원 효율성" 모드에서 실시간 처리가 어려울 때. 이번 판정은 AC·"최고 성능" 조건이다.
- GPU(Vulkan)·NPU(OpenVINO) 경로를 도입해 더 큰 모델을 실시간으로 쓸 수 있을 때. 현재 공식 배포본에는 Windows용 Vulkan 빌드가 없다.

## 5. 반영 문서

- [ROADMAP](../ROADMAP.md) M0 남은 작업 2에 측정 결과와 이 초안을 연결했다.
- [0006](0006-lecture-first-product-scope.md) 5절의 STT·LLM 동시 실행 위험 항목에 STT 단독 측정 결과를 연결했다.
- 측정 도구와 사용법은 [evaluation/README](../../evaluation/README.md)에 있다.
