# 강의 계약 v2 설계: 작업 나누기

- 날짜: 2026-09-26
- 상태: 사용자 승인 설계. 구현·검증 전
- 대상 작업: [로드맵](../../ROADMAP.md) M0 남은 작업 8, [결정 0010](../../decisions/0010-lecture-note-contract.md) 4절의 다음 결정 1(작업 나누기)
- 이전 결과: [강의 노트 출력 계약 실측](../../validation/2026-09-26-lecture-contract.md), [v1 계약 설계](2026-09-25-lecture-note-contract-design.md)

## 1. 목적과 범위

v1 실측에서 한 번의 호출로 여러 섹션을 채우게 하자 내용이 첫 섹션으로 쏠렸다. 초안 15건 모두 개념·예제·코드가 비었고, 코드는 노트까지 한 건도 나오지 않았다. 또 모호한 기한이 날짜로 들어갔고, 취소된 퀴즈가 날짜가 붙은 시험으로 분류됐으며, 쉬는 시간 발언에서 가짜 과제가 생겼다.

이번에는 **호출마다 채울 배열을 하나로 줄이고**, 앱이 결과를 조립한다. 그리고 v1과 같은 합성 강의·같은 조건으로 다시 측정해 비교한다.

범위 밖: 5분 노트 전체를 앱이 병합하는 파이프라인(작업 4), 실제 강의 녹음 평가, 모델 미세조정.

## 2. 사용자 결정 사항

| 항목 | 결정 |
| --- | --- |
| 분리 범위 | 초안과 노트 조립까지. 창마다 요점·공지·코드를 따로 호출하고, 노트의 공지·코드는 앱이 초안에서 합친다 |
| 모호한 기한 | 날짜 형태 검사를 더한다. 구체적인 날짜 요소가 없는 `date_text`는 앱이 `null`로 바꾼다 |
| 취소된 일정 | 공지에 `cancelled` 필드를 더한다 |
| 개념·예제 | 창 초안이 아니라 노트 본문에서 쓴다 |
| 판정 | 라틴 명령어 포착과 쉬는 시간 구간의 공지 미생성을 판정용 기대치에 더한다 |

## 3. 흐름

```
창(5분)마다, 녹음 중
  요점 호출  lecture-points-v1   → points 1~8
  공지 호출  lecture-notices-v1  → notices 0~5 (cancelled 포함)
  코드 호출  lecture-code-v1     → code 0~8
  앱 조립    lecture-draft-v2    = 창 범위 + 요점 + 공지 + 코드

녹음 후
  본문 호출  lecture-note-body-v1 → topic, concepts 1~20, examples 0~15, terms 0~20, review 0~10
             5분 노트: 초안의 요점 + 요점이 인용한 구간 / 정밀 정리: 전체 전사
  앱 조립    lecture-note-v2      = 본문 + 초안들의 공지·코드(중복 제거)
```

세 호출은 같은 창 전사를 입력으로 받는다. `-np 1`이라 순서대로 실행한다. 창 하나의 시간은 세 호출의 합이다.

## 4. 계약

### 4.1 모델 출력

| 계약 | 필드 |
| --- | --- |
| `lecture-points-v1` | `schema_version`, `points`: 요점 `{content, source_refs}` 1~8개 |
| `lecture-notices-v1` | `schema_version`, `notices`: `{kind, content, date_text, scope_text, cancelled, source_refs}` 0~5개. `kind`는 `exam`·`assignment`·`announcement`, `cancelled`는 참·거짓 |
| `lecture-code-v1` | `schema_version`, `code`: `{code, language, explanation, source_refs}` 0~8개. `language`는 `shell`·`c`·`python`·`other` |
| `lecture-note-body-v1` | `schema_version`, `topic {content, source_refs}`, `concepts {name, explanation, source_refs}` 1~20개, `examples` 0~15개, `terms {term_ko, definition, term_en, source_refs}` 0~20개, `review` 0~10개 |

공통 규칙은 v1과 같다. 출처는 입력 구간 ID만 1개 이상 중복 없이 쓰고, 생성 때 `enum`으로 제약한다. 스키마에 없는 필드·중복 키·잘린 응답은 거부한다. 앱이 채우는 값(창 범위, 코드의 `from_transcript`, 용어의 `term_en_source`)을 모델이 쓰면 거부한다.

### 4.2 앱의 조립과 정리

| 규칙 | 내용 | 기록 이름 |
| --- | --- | --- |
| 원문 대조 | 인용 구간에 그대로 없는 `date_text`·`scope_text`와 채움 문자열은 `null` | `unverified_cleared` |
| **날짜 형태** | `date_text`에 숫자+`월`·`일`·`시` 또는 요일(`월요일`~`일요일`, `월`·`화`·`수`·`목`·`금`·`토`·`일`+`요일`)이 없으면 `null`. "오늘"·"내일"처럼 노트를 나중에 볼 때 뜻이 바뀌는 말도 여기서 비워진다 | `undated_cleared` |
| 채움·반복 | 채움 항목, 앞 항목과 똑같은 요점·예제·복습·개념 설명, 정의가 용어를 되풀이한 용어를 지운다 | `filler_removed`, `repeat_removed`, `empty_definition_removed` |
| **공지 병합** | 여러 초안의 공지를 합칠 때 `kind`와 공백을 정리한 `content`가 같거나, `kind`·`date_text`가 같고 출처가 겹치면 앞의 것만 남긴다 | `notice_merged` |
| **코드 병합** | 공백을 정리한 `code`가 같으면 앞의 것만 남긴다 | `code_merged` |
| 원문 여부 | 코드가 인용 구간에 그대로 있으면 `from_transcript` 참, 영문 원어가 인용 구간에 있으면 `term_en_source: transcript` | (값 채움) |

정리는 지우거나 비우기만 하고 새 값을 만들지 않는다. 한 초안 안의 공지·코드 반복도 조립 때 같은 규칙으로 합친다.

## 5. 앱 구성

```
schemas/lecture-points-v1.json, lecture-notices-v1.json, lecture-code-v1.json, lecture-note-body-v1.json
app/src-tauri/src/lecture.rs          호출별 타입·검증·프롬프트, 날짜 형태 검사
app/src-tauri/src/lecture_merge.rs    초안·노트 조립과 공지·코드 병합
app/src-tauri/src/lecture_fixture.rs  기대치(추가·강화)
app/src-tauri/examples/lecture_contract_check.rs  호출 나누기에 맞춘 하네스
```

- v1 스키마 `lecture-draft-v1.json`, `lecture-note-v1.json`은 지운다. 기록은 git과 결정 0010에 남는다.
- `contract.rs`(전사 파싱, 생성용 스키마, 중복 키, 위반 수집)는 그대로 쓴다.
- 프롬프트 버전은 호출마다 따로 둔다(`lecture-points-prompt-v1` 등).

## 6. 합성 강의

전사는 바꾸지 않는다. 함정 목록에 두 가지를 더한다. 둘 다 v1 측정에서 드러난 실패라 사후에 추가했음을 fixture와 보고서에 적는다.

| 함정 | 구간 | 기대 동작 |
| --- | --- | --- |
| 쉬는 시간 발언("잠깐 쉬었다가 실습실 환경으로") | s24 | 어떤 공지도 인용하지 않는다 |
| 두 번째 라틴 명령어(`gcc -o banker banker.c`) | s26 | 코드 항목이 원문 그대로 있다(참고) |

## 7. 측정과 판정

- 조건: AC 전원·"최고 성능", 시작·끝 전원 기록, 측정 중 절전 방지. 서버 설정은 v1과 같다.
- 한 번의 실행: 창 3개 × 호출 3회 → 5분 노트 본문 1회 → 정밀 정리 본문 1회. **5회** 반복한다.
- 정리 후 위반이 남으면 위반 목록을 붙여 1회만 재시도하고, 두 번 실패하면 완료로 저장하지 않는다.

| 판정 | 기준 |
| --- | --- |
| 채택 | 정리·재시도 후 모든 호출이 채택되고, 판정용 기대치가 5회 모두 통과 |
| 판정용 기대치 | 잡담 제외, 시험 날짜·범위 원문 유지, 모호한 기한 `null`, 배점 미생성, 취소된 퀴즈를 인용한 공지는 모두 `cancelled: true`, 쉬는 시간 구간(s24)을 인용한 공지 없음, `chmod 755 run.sh`가 노트 코드에 원문 그대로 있음 |
| 참고 | 과제 포착, `gcc` 명령어 포착, 발음 명령어 복원 표시, 영문 원어 출처 표시, 섹션별 항목 수, 정리 종류별 횟수, 첫 시도 깨끗함·유효 비율 |
| 시간 | 창 하나(세 호출 합)의 최대가 150초 이내인지, 녹음 종료 후 추정(2 × 최대 창 시간 + 5분 노트 본문 시간)이 300초 이내인지 회차별로 본다 |

## 8. 자동 테스트

모델 없이 `cd app/src-tauri && cargo test`로 통과해야 한다.

- 네 스키마와 타입의 필드 일치, 모든 배열의 `maxItems`, 필수 섹션의 `minItems`.
- 호출별 검증: 잘림, 문법 오류, 중복 키, 추가·누락 필드, 앱이 정하는 필드를 모델이 쓴 경우, 출처 오류, 항목 수 초과, 필수 섹션 비움.
- 날짜 형태: "10월 21일"·"다음 주 수요일"·"오전 10시"·"21일"은 남고, "다음 주쯤"·"잠깐"·"내일"·"게시판에 공지"는 비워진다.
- `cancelled` 필드가 빠지면 거부한다.
- 조립: 초안 셋의 공지·코드 병합, 같은 공지·코드의 중복 제거와 기록, 창 범위 채움, 노트 본문과 합친 결과의 원문 여부 표시.
- 기대치: 새 함정 두 개와 강화된 취소 기대치의 통과·실패 사례.

## 9. 기록

- 보고서 `docs/validation/<날짜>-lecture-contract-v2.md`: v1과 같은 표로 나란히 비교한다.
- 결정 0010에 v2 결과와 채택 여부를 덧붙인다. 로드맵 남은 작업 8을 갱신하고, 채택되면 완료로 표시한다.
