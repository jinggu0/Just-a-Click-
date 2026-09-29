# 코드 목록 모양 검사 검증

- 날짜: 2026-09-29
- 범위: [코드 목록 모양 검사 설계](../superpowers/specs/2026-09-29-code-shape-design.md). 실제 강의의 코드 목록이 개념 단어로 찬 문제([실제 강의 1차](2026-09-28-lecture-real-1.md) 3절)를 다룬다
- 규칙: 코드 항목은 차례로 검사하고 처음 걸린 이유로 지운 뒤 기록한다.
  1. 영문자·숫자가 없으면 `not_code_removed`(기존)
  2. 한글이 섞이면 `hangul_in_code_removed`
  3. 공백 없는 ASCII 영문자 한 단어면 `bare_word_removed`
  4. 인용 구간이 4개 이상이면 `wide_citation_removed`
- 방법: 모델 출력 뒤의 결정적 정리이므로 저장된 출력의 초안 코드 전부에 다시 적용했다(예제 `code_shape_check`, 모델 호출 없음)
- 결과 파일: `artifacts/lecture-contract-v2/code-shape-check.json`(커밋하지 않음)

## 1. 요약

**반드시 남아야 할 명령은 모두 남았고, 실제 강의의 코드 목록은 개념 단어가 빠져 8개로 줄었다.** 합성 강의 26회분에서 `chmod 755 run.sh`·`gcc -o banker banker.c`·`ps aux`는 한 번도 지워지지 않았다. 실제 강의 초안의 코드 43개 가운데 35개가 지워졌다. 지워진 것 가운데 코드다운 항목은 없었다.

## 2. 저장된 출력 재검사

| 실행 | 초안 코드 | 남음 | 한 단어 | 한글 | 넓은 인용 | 필수 명령 손실 |
| --- | --- | --- | --- | --- | --- | --- |
| v2 측정(`run`) | 43 | 22 | 21 | 0 | 0 | 0 |
| 시간 재측정(`timing`) | 47 | 20 | 22 | 5 | 0 | 0 |
| 시간 재측정 2(`timing-2`) | 44 | 22 | 21 | 1 | 0 | 0 |
| 정밀 정리 측정 1(`precise-1`) | 45 | 20 | 22 | 3 | 0 | 0 |
| 정밀 정리 측정 2(`precise`) | 46 | 22 | 22 | 2 | 0 | 0 |
| 공지 통합 확인(`notice-trial`) | 8 | 4 | 4 | 0 | 0 | 0 |
| 실제 강의(`lecture-1`) | 43 | **8** | 30 | 2 | 3 | — |
| 실제 강의 재생(`lecture-1-replay`) | 42 | **7** | 30 | 2 | 3 | — |

- 합성 강의에서 남은 항목: `chmod 755 run.sh`, `gcc -o banker banker.c`, `ps aux`, `ps aux | grep banker`, `kill -9`, "Banker's algorithm".
- 실제 강의에서 남은 항목: `seek_set`, `seek_current`, `seek_end`, `mk-node`, `mk-pico`, `read-only`, "exclusive flag", "file table".
- 실제 강의에서 지워진 항목: fd, read, write, open, close, buffer, NULL, Bluetooth, pointer, offset 등 한 단어 30개, "lc 함수"·"l식 함수", 구간 10개를 인용한 while loop 등.

## 3. 판정

| 조건(설계 4절) | 결과 |
| --- | --- |
| 단위 테스트(남는 예 9개, 지워지는 예 8개, 넓은 인용) | **통과** |
| 합성 강의의 필수 명령이 지워지지 않음 | **통과** 0건 손실 |
| 실제 강의에서 지워진 항목 가운데 코드다운 것 없음 | **통과** |

## 4. 남은 문제

- **여러 단어 개념어:** "Banker's algorithm"(기호가 있음), "exclusive flag", "file table"(여러 단어)은 남는다. 명령(`ps aux`)과 글자 모양으로 구별되지 않는다.
- **전사 오류가 코드에 남음:** `mk-node`·`mk-pico`는 강의에서 말한 `mknod`·`mkfifo`를 whisper가 잘못 적은 것으로 보인다. 원문 그대로라 "원문 확인"으로 표시되지만 명령 이름이 틀렸다. 전사 용어 교정이 필요하다.
- **한 단어 명령:** 강의가 `ls`, `pwd` 같은 한 단어 명령만 말하면 그 명령도 지워진다. 실제 강의 평가셋에서 빈도를 본다.
- 코드를 말로만 설명한 강의(`open(...)`의 인자 등)는 코드 줄이 목록에 들어오지 않는다. 강의 자료(PDF·PPTX) 연동에서 다룬다.
