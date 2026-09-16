# M0 로컬 LLM 실행 시험

## 준비

Windows에서 Python 3.11 이상으로 저장소 루트에서 실행한다.

```powershell
python scripts/prepare_llm.py
python -m unittest discover -s tests -v
python scripts/benchmark_llm.py --backend cpu --runs 3
python scripts/benchmark_llm.py --backend vulkan --runs 3
```

- 준비 도구는 `config/llm-model.json`과 `config/llama-runtime.json`에 고정된 파일을 받는다. 설치 관리자 제품 구현이 아닌 개발용 준비 스크립트다.
- 다운로드한 모델과 실행 파일은 `models/`, `runtimes/`, `downloads/`에 저장하며 Git에서 제외한다.
- 모델 다운로드는 최대 8개의 64MiB 구간 요청을 병행하고 완료된 구간은 재실행 시 재사용한다. 공급 서버가 정확한 Range 응답을 지원해야 한다.
- 파일 크기와 SHA-256이 맞아야 활성화한다. 해시 불일치 시 캐시 손상 여부를 조사한 후 해당 구간 캐시를 제거하고 다시 받는다.
- 각 실행은 고유한 `artifacts/smoke-.../` 디렉터리에 서버 로그·모델 원응답·측정 보고서를 저장한다. 결과 디렉터리는 커밋하지 않는다.
- 로컬 서버는 `127.0.0.1`에만 열고 임의 토큰으로 인증하며 시험 종료·실패 시 종료한다. 로컬 호출에서 프록시 설정을 사용하지 않는다.

## 측정 범위

`meeting-smoke.json`은 프로젝트에서 직접 작성한 합성 회의문이다. 개인정보나 실제 녹음은 포함하지 않는다. 취소된 제안, 담당자·미정 기한, 전사 시간 제외, 미검증 가속, 잡담을 올바르게 처리하는지 수동 검수한다.

CPU와 Vulkan을 같은 16,384 컨텍스트·8스레드·seed 42·non-thinking 설정으로 비교한다. 요청별 프롬프트 캐시를 비활성화한다. 프로세스 시작에서 준비 완료까지의 시간과 요청부터 응답 파일 저장까지의 시간을 구분한다. 첫 실행의 합계도 기록한다.

구조 검사는 JSON과 필수 필드 확인만 수행한다. 의미 정확성·출처 충실성은 `review_checklist`와 실제 응답을 사람이 확인해야 한다. `finish_reason=length`이면 완성된 요약으로 판단하지 않는다.

이 시험은 짧은 텍스트 추론의 동작 확인이다. 실제 음성 전사, 4시간 입력, 장문 분할·통합, 전체 제품 오프라인 통신 감사, 학습 및 출시 품질을 검증하지 않는다. 측정 도구 자체의 메모리 계측·전원 모드 통제도 현재 범위에 포함되지 않는다.
