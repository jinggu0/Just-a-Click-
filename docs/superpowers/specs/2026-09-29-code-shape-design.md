# 코드 목록 모양 검사 설계

- 날짜: 2026-09-29
- 상태: 사용자 승인 설계. 구현·검증 전
- 대상: [결정 0010](../../decisions/0010-lecture-note-contract.md)의 코드 호출(`lecture-code-v1`)과 앱의 코드 정리
- 근거: [실제 강의 1차 검증](../../validation/2026-09-28-lecture-real-1.md) 3절, [v2 실측](../../validation/2026-09-26-lecture-contract-v2.md) 4절

## 1. 문제

실제 강의 30분의 코드 목록 30개 가운데 코드다운 것은 `seek_set`·`seek_current`·`seek_end`, 원문 그대로의 `mk-node`·`mk-pico` 정도였다. 나머지는 다음과 같았다.

- 알파벳 한 단어 17개: fd, buffer, read, write, open, close, create, NULL, sum, long, descriptor, Bluetooth, pointer, summation, fileoffset, filepointer, offset.
- 한글이 섞인 전사 오류 2개: "l식 함수", "lc 함수".
- 여러 단어 개념어: file descriptor, file table, create flag, exclusive flag, while loop.
- 구간 10개를 한꺼번에 인용한 항목 6개.

합성 강의에서도 "Banker's algorithm", "C", "wait", "signal", "kill"이 코드로 들어갔다.

## 2. 사용자 결정

| 질문 | 결정 |
| --- | --- |
| 코드 목록의 기준 | 코드 모양만 남긴다. 알파벳 한 단어, 한글이 섞인 항목, 넓게 인용한 항목은 앱이 지운다 |

## 3. 설계

코드 항목마다 아래 순서로 보고 처음 걸린 이유로 지운 뒤 기록한다. 새 값을 만들지 않는다.

| 순서 | 조건 | 기록 |
| --- | --- | --- |
| 1 | 영문자·숫자가 하나도 없음(기존) | `not_code_removed` |
| 2 | 한글 글자나 자모가 있음 | `hangul_in_code_removed` |
| 3 | 공백으로 나눈 조각이 하나이고 ASCII 영문자로만 이뤄짐 | `bare_word_removed` |
| 4 | 인용 구간이 4개 이상 | `wide_citation_removed` |

- 남는 예: `chmod 755 run.sh`, `gcc -o banker banker.c`, `ps aux`(공백 있는 명령), `seek_set`, `mk-node`, `read-only`, `O_CREAT`, `printf()`(기호·숫자·밑줄).
- 지워지는 예: `fd`, `NULL`, `read`, `Bluetooth`, `C`, `kill`, `lc 함수`.
- 한 단어 시스템 호출 이름(`read`, `open`)도 지워진다. 같은 내용은 개념·용어에 들어간다. 괄호를 붙인 `read()`는 남는다.
- 여러 단어 개념어(`file descriptor`)는 `ps aux` 같은 명령과 글자 모양으로 구별되지 않아 남는다.
- 구현: `lecture.rs`에 `code_problem(item: &Code) -> Option<&'static str>`를 두고 `Cleaner::code`가 쓴다.

## 4. 검증

| 확인 | 방법 | 통과 조건 |
| --- | --- | --- |
| 단위 테스트 | 위 남는 예·지워지는 예, 인용 4개 항목 | 모두 기대대로 |
| 저장된 합성 강의 출력 | 새 예제 `code_shape_check`로 측정 5개(25회분)의 초안 코드에 규칙 적용 | `chmod 755 run.sh`·`gcc -o banker banker.c`·`ps aux`가 초안에 있던 회차에서 하나도 지워지지 않음 |
| 저장된 실제 강의 출력 | 같은 예제로 `lecture-1`, `lecture-1-replay` 적용 | 남는 항목과 지워지는 항목을 보고. 지워진 항목 중 코드다운 것이 없음 |

## 5. 한계

- 여러 단어 개념어는 남는다.
- 강의가 한 단어 명령(`ls`, `pwd`)만 말하면 그 명령도 지워진다. 실제 강의 평가셋에서 빈도를 본다.
