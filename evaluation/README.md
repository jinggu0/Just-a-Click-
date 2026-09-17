# M0 로컬 LLM 실행 시험

## 준비

Windows에서 Python 3.11 이상으로 저장소 루트에서 실행한다.

```powershell
python scripts/prepare_llm.py
python -m unittest discover -s tests -v
python scripts/benchmark_llm.py --backend cpu --runs 3
python scripts/benchmark_llm.py --backend vulkan --runs 3
```

현재 도구는 기본으로 엄격한 회의 스키마를 사용한다. 과거의 약한 JSON object 검사는 비교용 `--legacy-json`으로만 실행한다. 최초 기준선의 정확한 실행 코드는 커밋 `6bda768`에 보존되어 있다.

```powershell
python scripts/benchmark_llm.py --backend vulkan --runs 1 --strict --flash-attn auto --prefill-tokens 4096
python scripts/benchmark_llm.py --backend vulkan --runs 1 --strict --flash-attn on --prefill-tokens 4096 8192
python scripts/benchmark_llm.py --backend vulkan --probe-only --flash-attn on --batch-size 512 --ubatch-size 128 --prefill-tokens 8192
```

- 준비 도구는 `config/llm-model.json`과 `config/llama-runtime.json`에 고정된 파일을 받는다. 설치 관리자 제품 구현이 아닌 개발용 준비 스크립트다.
- 다운로드한 모델과 실행 파일은 `models/`, `runtimes/`, `downloads/`에 저장하며 Git에서 제외한다.
- 모델 다운로드는 최대 8개의 64MiB 구간 요청을 병행하고 완료된 구간은 재실행 시 재사용한다. 공급 서버가 정확한 Range 응답을 지원해야 한다.
- 파일 크기와 SHA-256이 맞아야 활성화한다. 해시 불일치 시 캐시 손상 여부를 조사한 후 해당 구간 캐시를 제거하고 다시 받는다.
- 각 실행은 고유한 `artifacts/smoke-.../` 디렉터리에 서버 로그·모델 원응답·측정 보고서를 저장한다. 결과 디렉터리는 커밋하지 않는다.
- 로컬 서버는 `127.0.0.1`에만 열고 임의 토큰으로 인증하며 시험 종료·실패 시 종료한다. 로컬 호출에서 프록시 설정을 사용하지 않는다.

## llama-bench 처리량과 시간 목표 추정

```powershell
python scripts/run_llama_bench.py --stage all
python scripts/feasibility.py evaluation/results/<날짜>-llama-bench-all.json --server-report evaluation/results/<날짜>-constraint-schema.json --output evaluation/results/<날짜>-feasibility.json --markdown artifacts/<날짜>-feasibility.md
```

- `run_llama_bench.py`는 고정된 b10994의 `llama-bench`로 합성 토큰의 입력 처리(pp512)와 생성(tg128) 속도를 조건별 3회 측정한다. CPU 4·8스레드와 Vulkan 4·8스레드 × Flash Attention off·on을 깊이 0·2,048에서 비교한 뒤, 가장 빠른 생성 설정으로 깊이 0·2,048·8,192를 ubatch 128·512로 측정한다.
- ubatch 512의 8,192 깊이는 서버 시험에서 GPU device lost가 났던 조건이므로 실패를 허용하는 위험 시험으로 마지막에 실행한다. 실패해도 드라이버·TDR 설정을 바꾸지 않는다.
- AC 전원이 아니거나 다른 `llama-*` 프로세스가 실행 중이면 시작하지 않는다. 측정 중에는 이 프로세스만 절전을 막고 시스템 전원 설정은 바꾸지 않는다.
- 원출력·로그는 `artifacts/llama-bench-*/`, 요약은 `evaluation/results/<날짜>-llama-bench-<단계>.json`에 저장하며 기존 파일을 덮어쓰지 않는다. 요약에는 로컬 경로를 남기지 않는다.
- 오래 걸리는 측정은 `--stage screen`과 `--stage depth --threads <스레드> --flash-attn <on|off>`로 나눠 실행할 수 있다.
- `feasibility.py`는 측정 처리량과 결정 0006의 가정(발화 속도, 음절당 토큰, 구간 출력량 등)으로 녹음 종료 후 5분·다시 요약 1시간 목표를 추정한다. 결과는 추정이며 실제 앱 지연시간이나 요약 품질 통과가 아니다. 한국어 표는 인코딩 문제를 피하려고 `--markdown` 파일로만 쓴다.

## 측정 범위

`meeting-smoke.json`은 프로젝트에서 직접 작성한 합성 회의문이다. 개인정보나 실제 녹음은 포함하지 않는다. 취소된 제안, 담당자·미정 기한, 전사 시간 제외, 미검증 가속, 잡담을 올바르게 처리하는지 수동 검수한다.

CPU와 Vulkan을 같은 16,384 컨텍스트·8스레드·seed 42·non-thinking 설정으로 비교한다. 요청별 프롬프트 캐시를 비활성화한다. 프로세스 시작에서 준비 완료까지의 시간과 요청부터 응답 파일 저장까지의 시간을 구분한다. 첫 실행의 합계도 기록한다.

기본 구조 검사는 버전 고정 스키마, 모든 action의 owner·deadline, 출처 ID와 명시된 값의 원문 존재 여부를 확인한다. 원문 등장만으로 실제 담당 관계를 증명할 수는 없다. 의미 정확성·출처 충실성은 `review_checklist`와 실제 응답을 사람이 확인해야 한다. `finish_reason=length`이면 완성된 요약으로 판단하지 않는다.

검증 실패 응답은 원응답 파일만 남기고 `accepted-N.json`을 만들지 않는다. 계약을 통과해도 합성 사례 전용 회귀 검사에서 실패하면 `completed_with_validation_failures`로 기록한다. `completed`도 일반적인 요약 품질 합격을 의미하지 않는다.

평가 도구 v2는 요청 시간에 템플릿 적용·토큰 예산 검사·로컬 계약 검증·accepted 파일 저장을 포함한다. strict 모드의 프롬프트와 출력 구조가 기존 기준선과 달라 단순 시간 차이를 GPU 설정만의 효과로 해석하면 안 된다. 기본 생성 모드는 `--reasoning off`로 설정한다.

`--prefill-tokens`는 반복 합성 텍스트를 토큰화한 뒤 지정한 길이의 토큰 배열로 입력 처리량만 측정한다. 1토큰만 생성하고 잘림 여부와 실제 입력 처리 수를 확인한다. 긴 문서의 내용 이해나 전체 장문 요약을 평가하는 기능은 아니다.

`--probe-only`는 요약을 반복하지 않고 입력 처리 시험만 실행한다. GPU 오류가 발생하면 서버를 종료하고 실패를 보고서에 남긴다. 입력을 잘라 성공으로 처리하거나 Windows 드라이버·TDR 설정을 변경하지 않는다. 배치 인자는 실험용이며 작은 배치가 모든 장치 오류를 해결한다고 가정하지 않는다.

이 시험은 짧은 텍스트 추론의 동작 확인이다. 실제 음성 전사, 4시간 입력, 장문 분할·통합, 전체 제품 오프라인 통신 감사, 학습 및 출시 품질을 검증하지 않는다. 측정 도구 자체의 메모리 계측·전원 모드 통제도 현재 범위에 포함되지 않는다.
