# 강의 평가셋 실행 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 강의 평가셋의 정답 형식·채점 도구·녹음 절차·합격 기준을 만들고, 강의 1의 기준선을 채점한다.

**Architecture:** 정답은 `schemas/lecture-reference-v1.json`으로 검증하는 JSON이다. `scripts/lecture_eval.py`가 `lecture_pipeline` 결과와 정답을 비교해 강의별·전체 점수를 낸다. 강의 데이터는 `artifacts/eval-lectures/`에만 둔다.

**Tech Stack:** Python 3.11 표준 라이브러리(`json`, `unittest`), 기존 `lecture_pipeline` 예제.

## Global Constraints

- 설계는 [강의 평가셋 설계](../specs/2026-09-29-lecture-eval-set-design.md)를 따른다.
- 강의 음성·전사·정답은 커밋하지 않는다. 집계는 강의 내용 없이 수치만 `evaluation/results/`에 둔다.
- 새 의존성을 넣지 않는다(JSON 스키마 검증은 필요한 필드·열거값만 직접 확인한다).
- 문서는 한국어, 코드 식별자·주석은 영어. 작업마다 커밋하고 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`를 넣는다.

### Task 1: 정답 스키마와 채점 도구

**Files:** Create `schemas/lecture-reference-v1.json`, `scripts/lecture_eval.py`, `tests/test_lecture_eval.py`

- [ ] 테스트: 정답 검증(필수 필드·열거값·허락 누락 거부), 공지 짝짓기(종류와 구간 겹침), 없는 공지·상태 오류·날짜 오류, 코드 재현율·정밀도(공백·대소문자 무시), 용어 표기 비율, 핵심 사실 대리 재현율(5분 노트·정밀 정리 따로), 전사가 다르면 채점 거부, 합격 판정.
- [ ] 구현 후 `python -m unittest tests.test_lecture_eval -v` 통과, 커밋.

### Task 2: 녹음 절차와 평가셋 문서

**Files:** Modify `evaluation/README.md`

- [ ] "강의 평가셋" 절: 폴더 구조, 허락 확인, 녹음 체크리스트(AC·뚜껑·과목명·시작·종료), 정답 초안·검수 흐름, 채점 명령, 합격 기준 v1. 링크 검사 후 커밋.

### Task 3: 강의 1 정답 초안과 기준선

- [ ] 현재 코드로 강의 1 녹음을 재생 처리한다(`runs/2026-09-29-replay`).
- [ ] 그 전사로 정답 초안(`reference.json`, `review.status: draft`)을 쓴다.
- [ ] 초안 정답 기준으로 채점하고, 사용자에게 검수를 요청한다.
- [ ] 집계를 `evaluation/results/2026-09-29-lecture-eval.json`에 쓰고, 보고서·로드맵을 갱신해 커밋한다.
