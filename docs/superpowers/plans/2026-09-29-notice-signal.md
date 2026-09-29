# 공지 운영 단어 검사 실행 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 인용 구간에 운영 단어가 없는 공지를 앱이 지우고, 운영 단어가 없는 창은 공지 호출을 건너뛴다.

**Architecture:** `lecture.rs`에 운영 단어 목록과 `has_notice_signal`을 두고 `Cleaner::notices`가 쓴다. 하네스·`lecture_pipeline`은 창 전사로 공지 호출 여부를 정한다. 새 예제 `notice_signal_check`가 저장된 출력에 규칙을 다시 적용한다.

**Tech Stack:** Rust 1.98.1, `serde_json`(기존).

## Global Constraints

- 설계는 [공지 운영 단어 검사 설계](../specs/2026-09-29-notice-signal-design.md)를 따른다.
- 새 의존성을 넣지 않는다. 스키마·프롬프트는 바꾸지 않는다.
- 결과는 `artifacts/` 아래에만 두고 커밋하지 않는다. 측정은 AC·"최고 성능"에서 하고 전원 상태를 기록한다.
- 코드 주석·식별자는 영어, 문서는 한국어. 작업마다 커밋하고 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`를 넣는다.

### Task 1: 운영 단어 검사

**Files:** Modify `app/src-tauri/src/lecture.rs`

- [ ] 테스트 `notices_need_an_operational_word_in_their_segments`: 흔한 공지 문장 12개는 `has_notice_signal`이 참, 예제·코드 설명 문장 6개는 거짓. 기존 테스트 전사(s2 중간고사, s4 과제·퀴즈)의 공지는 남고, 운영 단어 없는 구간(s1, s3)을 인용한 공지는 `no_notice_signal`로 지워진다.
- [ ] 구현: `NOTICE_SIGNALS`, `has_notice_signal(text: &str) -> bool`(공백 제거, 소문자), `Cleaner::notices` 첫 검사.
- [ ] `cargo test --lib` 통과 후 커밋.

### Task 2: 공지 호출 건너뛰기

**Files:** Modify `app/src-tauri/examples/lecture_contract_check.rs`, `app/src-tauri/examples/lecture_pipeline.rs`

- [ ] 창 전사(`lines(window)`)에 `has_notice_signal`이 거짓이면 공지 호출 없이 `Accepted { value: vec![], repairs: vec![] }`를 쓰고, 초안 기록에 `notices_skipped: true`를 남긴다. 하네스의 창 시간은 호출한 것만 더한다.
- [ ] 두 예제 빌드, `cargo test` 통과 후 커밋.

### Task 3: 저장된 출력 재검사

**Files:** Create `app/src-tauri/examples/notice_signal_check.rs`

- [ ] 인자: `<root> <out json> <run dir>...`. `lecture-contract.json`이면 픽스처 전 구간, `lecture-pipeline.json`이면 그 폴더의 `transcript.txt`를 구간으로 쓴다. 초안·노트의 모든 공지에 대해 인용 구간 텍스트로 `has_notice_signal`을 계산해 남김·지움을 기록하고, 창마다 건너뛸지(창 전사에 운영 단어 없음)도 기록한다.
- [ ] 빌드 후 여섯 폴더로 실행하고 설계 4절 조건을 확인한다. 커밋.

### Task 4: 통합 확인과 기록

- [ ] 합성 강의 하네스 1회(`artifacts/lecture-contract-v2/notice-trial/`).
- [ ] 보고서 `docs/validation/2026-09-29-notice-signal.md`, 결정 0010에 한 문단, 실제 강의 보고서 5절 2번에 결과 링크, README 목록. `python artifacts/check_docs.py` 후 커밋.
