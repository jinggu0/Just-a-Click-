# 정밀 정리 창별 호출 실행 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 정밀 정리를 "창마다 한 번 호출 → 앱이 이름으로 병합 → 주제·복습만 쓰는 합성 호출"로 바꾸고, 합성 강의 5회로 기존 한 번 읽기 방식(10건)과 비교해 채택 여부를 정한다.

**Architecture:** 모델 출력 스키마 두 개(`lecture-precise-window-v1`, `lecture-precise-synthesis-v1`)를 더한다. `lecture.rs`는 두 호출의 타입·검증·프롬프트를, `lecture_merge.rs`는 창별 결과 병합(`merge_precise`), 합성 입력(`synthesis_input`, `cited_by`)과 노트 조립(`assemble_precise_note`)을 맡는다. 하네스는 기존 `note_body_from_transcript` 단계를 새 흐름으로 바꾼다. 5분 노트 경로는 그대로 둔다.

**Tech Stack:** Rust 1.98.1, `serde`·`serde_json`(기존), llama.cpp `b10994` Vulkan `llama-server`, Qwen3-8B Q5_K_M.

## Global Constraints

- 설계는 [정밀 정리 설계](../specs/2026-09-28-precise-note-by-window-design.md)를 따른다. 공통 규칙과 역할 분담은 [v2 설계](../specs/2026-09-26-lecture-contract-split-design.md)와 같다.
- 새 의존성을 넣지 않는다.
- 모델 출력 스키마는 `schemas/`의 파일이 원본이고 앱은 `include_str!`로 담는다.
- 서버 설정은 v2 측정과 같다: 컨텍스트 8,192, `-np 1`, `--cache-ram 0`, `-ngl 99`, 2스레드, `--jinja --reasoning off`, 온도 0.2.
- 측정은 AC 전원·Windows 전원 모드 "최고 성능"·배터리 완충(충전 없음)에서 하고, 시작과 끝의 전원 상태와 배경 CPU 부하를 기록한다. 전원 설정은 사용자가 바꾸고, 사용자 앱(OneDrive 등)을 멈추기 전에는 사용자에게 묻는다.
- 원응답·측정 결과·서버 로그는 `artifacts/lecture-contract-v2/` 아래에만 두고 커밋하지 않는다.
- 코드 주석과 식별자는 영어, 프롬프트·문서는 한국어로 쓴다.
- 모든 명령은 저장소 뿌리 `C:\temp_git\Just-a-Click-`에서 시작한다. Bash에서 cargo를 쓰려면 먼저 `export PATH="$PATH:/c/Users/jingg/.cargo/bin"`을 실행한다.
- 작업마다 커밋하고 메시지 끝에 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`를 넣는다. push하지 않는다.

## 파일 구조

| 파일 | 책임 |
| --- | --- |
| `schemas/lecture-precise-window-v1.json` | 창 호출 출력: 개념 0~8, 예제 0~6, 용어 0~10 |
| `schemas/lecture-precise-synthesis-v1.json` | 합성 호출 출력: 주제 1, 복습 0~10 |
| `app/src-tauri/src/contract.rs` | 두 스키마 상수와 `LECTURE_SCHEMAS`(6개) |
| `app/src-tauri/src/lecture.rs` | `PreciseWindow`·`PreciseSynthesis`, 검증 두 개, 프롬프트 두 개, `english_source` |
| `app/src-tauri/src/lecture_merge.rs` | `PreciseBody`, `name_key`, `merge_precise`, `synthesis_input`, `cited_by`, `assemble_precise_note` |
| `app/src-tauri/examples/lecture_contract_check.rs` | 정밀 정리 단계 교체와 요약 지표 |
| `docs/validation/<날짜>-precise-note-by-window.md` | 결과 보고서 |

---

### Task 1: 두 스키마

**Files:**
- Create: `schemas/lecture-precise-window-v1.json`
- Create: `schemas/lecture-precise-synthesis-v1.json`
- Modify: `app/src-tauri/src/contract.rs:11-17`
- Test: `app/src-tauri/src/lecture.rs`(테스트 모듈의 `schema_files_and_types_name_the_same_fields`)

**Interfaces:**
- Produces: `contract::PRECISE_WINDOW_SCHEMA: &str`, `contract::PRECISE_SYNTHESIS_SCHEMA: &str`, `contract::LECTURE_SCHEMAS: [&str; 6]`(순서: 요점, 공지, 코드, 노트 본문, 창, 합성). 테스트 도우미 `precise_window() -> Value`, `synthesis() -> Value`(lecture.rs 테스트 모듈).

- [ ] **Step 1: 실패하는 테스트 작성**

`app/src-tauri/src/lecture.rs` 테스트 모듈의 `body()` 아래에 도우미를 더한다.

```rust
    fn precise_window() -> Value {
        json!({
            "schema_version": "lecture-precise-window-v1",
            "concepts": [{"name": "교착 상태", "explanation": "서로의 자원을 기다리며 멈춘 상태", "source_refs": ["s1"]}],
            "examples": [{"content": "두 프로세스가 서로의 자원을 기다리는 예", "source_refs": ["s1"]}],
            "terms": [{"term_ko": "교착 상태", "definition": "서로 기다리며 멈춘 상태", "term_en": "Deadlock", "source_refs": ["s1"]}]
        })
    }

    fn synthesis() -> Value {
        json!({
            "schema_version": "lecture-precise-synthesis-v1",
            "topic": {"content": "교착 상태", "source_refs": ["s1"]},
            "review": [{"content": "교착 상태의 조건을 복습한다", "source_refs": ["s1"]}]
        })
    }
```

`schema_files_and_types_name_the_same_fields`의 구조 분해와 끝부분을 바꾼다.

```rust
        let [points_schema, notices_schema, code_schema, body_schema, window_schema, synthesis_schema] = &schemas[..] else {
            panic!("six schemas expected");
        };
```

함수 끝(`assert_eq!(body_schema["properties"]["concepts"]["minItems"], json!(1));` 다음)에 더한다.

```rust
        assert_eq!(required(window_schema, "/required"), keys(&precise_window()));
        assert_eq!(required(synthesis_schema, "/required"), keys(&synthesis()));
        assert_eq!(required(window_schema, "/$defs/concept/required"), keys(&precise_window()["concepts"][0]));
        assert_eq!(required(window_schema, "/$defs/item/required"), keys(&precise_window()["examples"][0]));
        assert_eq!(required(window_schema, "/$defs/term/required"), keys(&precise_window()["terms"][0]));
        assert_eq!(required(synthesis_schema, "/$defs/item/required"), keys(&synthesis()["topic"]));
        assert_eq!(window_schema["properties"]["concepts"]["maxItems"], json!(8));
        assert_eq!(window_schema["properties"]["examples"]["maxItems"], json!(6));
        assert_eq!(window_schema["properties"]["terms"]["maxItems"], json!(10));
        assert!(window_schema["properties"]["concepts"].get("minItems").is_none(), "a window may have no concepts");
        assert_eq!(synthesis_schema["properties"]["review"]["maxItems"], json!(10));
```

- [ ] **Step 2: 실패 확인**

Run: `cd app/src-tauri && cargo test --lib schema_files_and_types_name_the_same_fields`
Expected: 컴파일은 되지만 `panicked at ... six schemas expected`로 FAIL.

- [ ] **Step 3: 스키마 작성**

`schemas/lecture-precise-window-v1.json`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema_version",
    "concepts",
    "examples",
    "terms"
  ],
  "properties": {
    "schema_version": {
      "type": "string",
      "const": "lecture-precise-window-v1"
    },
    "concepts": {
      "type": "array",
      "maxItems": 8,
      "items": {
        "$ref": "#/$defs/concept"
      }
    },
    "examples": {
      "type": "array",
      "maxItems": 6,
      "items": {
        "$ref": "#/$defs/item"
      }
    },
    "terms": {
      "type": "array",
      "maxItems": 10,
      "items": {
        "$ref": "#/$defs/term"
      }
    }
  },
  "$defs": {
    "segment": {
      "type": "string"
    },
    "refs": {
      "type": "array",
      "minItems": 1,
      "items": {
        "$ref": "#/$defs/segment"
      }
    },
    "text": {
      "type": "string",
      "minLength": 1
    },
    "optional_text": {
      "type": [
        "string",
        "null"
      ],
      "minLength": 1
    },
    "item": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "content",
        "source_refs"
      ],
      "properties": {
        "content": {
          "$ref": "#/$defs/text"
        },
        "source_refs": {
          "$ref": "#/$defs/refs"
        }
      }
    },
    "concept": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "name",
        "explanation",
        "source_refs"
      ],
      "properties": {
        "name": {
          "$ref": "#/$defs/text"
        },
        "explanation": {
          "$ref": "#/$defs/text"
        },
        "source_refs": {
          "$ref": "#/$defs/refs"
        }
      }
    },
    "term": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "term_ko",
        "definition",
        "term_en",
        "source_refs"
      ],
      "properties": {
        "term_ko": {
          "$ref": "#/$defs/text"
        },
        "definition": {
          "$ref": "#/$defs/text"
        },
        "term_en": {
          "$ref": "#/$defs/optional_text"
        },
        "source_refs": {
          "$ref": "#/$defs/refs"
        }
      }
    }
  }
}
```

`schemas/lecture-precise-synthesis-v1.json`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema_version",
    "topic",
    "review"
  ],
  "properties": {
    "schema_version": {
      "type": "string",
      "const": "lecture-precise-synthesis-v1"
    },
    "topic": {
      "$ref": "#/$defs/item"
    },
    "review": {
      "type": "array",
      "maxItems": 10,
      "items": {
        "$ref": "#/$defs/item"
      }
    }
  },
  "$defs": {
    "segment": {
      "type": "string"
    },
    "refs": {
      "type": "array",
      "minItems": 1,
      "items": {
        "$ref": "#/$defs/segment"
      }
    },
    "text": {
      "type": "string",
      "minLength": 1
    },
    "item": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "content",
        "source_refs"
      ],
      "properties": {
        "content": {
          "$ref": "#/$defs/text"
        },
        "source_refs": {
          "$ref": "#/$defs/refs"
        }
      }
    }
  }
}
```

`app/src-tauri/src/contract.rs`의 `NOTE_BODY_SCHEMA` 아래에 상수를 더하고 목록을 늘린다.

```rust
pub const PRECISE_WINDOW_SCHEMA: &str = include_str!("../../../schemas/lecture-precise-window-v1.json");
pub const PRECISE_SYNTHESIS_SCHEMA: &str = include_str!("../../../schemas/lecture-precise-synthesis-v1.json");
```

```rust
pub const LECTURE_SCHEMAS: [&str; 6] = [
    POINTS_SCHEMA,
    NOTICES_SCHEMA,
    CODE_SCHEMA,
    NOTE_BODY_SCHEMA,
    PRECISE_WINDOW_SCHEMA,
    PRECISE_SYNTHESIS_SCHEMA,
];
```

- [ ] **Step 4: 통과 확인**

Run: `cd app/src-tauri && cargo test --lib`
Expected: 모두 PASS. `the_generation_schema_allows_only_the_given_segments`가 새 두 스키마에도 `enum`을 넣는지 함께 확인한다.

- [ ] **Step 5: 커밋**

```bash
git add schemas/lecture-precise-window-v1.json schemas/lecture-precise-synthesis-v1.json app/src-tauri/src/contract.rs app/src-tauri/src/lecture.rs
git commit -m "feat: add the precise-note window and synthesis schemas"
```

---

### Task 2: 창·합성 호출의 검증과 프롬프트

**Files:**
- Modify: `app/src-tauri/src/lecture.rs`

**Interfaces:**
- Consumes: Task 1의 `PRECISE_WINDOW_SCHEMA`, `PRECISE_SYNTHESIS_SCHEMA`.
- Produces:
  - `pub struct PreciseWindow { pub schema_version: String, pub concepts: Vec<Concept>, pub examples: Vec<Item>, pub terms: Vec<Term> }`
  - `pub struct PreciseSynthesis { pub schema_version: String, pub topic: Item, pub review: Vec<Item> }`
  - `pub fn validate_precise_window(content: &str, finish_reason: &str, window: &[Segment]) -> Result<Accepted<PreciseWindow>, Vec<Violation>>`
  - `pub fn validate_precise_synthesis(content: &str, finish_reason: &str, cited: &[Segment]) -> Result<Accepted<PreciseSynthesis>, Vec<Violation>>`
  - `pub fn precise_window_prompt() -> String`, `pub fn precise_synthesis_prompt() -> String`
  - `pub fn english_source(segments: &[Segment], term: &Term) -> Option<TermSource>`
  - 상수 `PRECISE_WINDOW_VERSION`, `PRECISE_SYNTHESIS_VERSION`, `PRECISE_WINDOW_PROMPT_VERSION`, `PRECISE_SYNTHESIS_PROMPT_VERSION`

- [ ] **Step 1: 실패하는 테스트 작성**

테스트 모듈 끝에 더한다.

```rust
    #[test]
    fn a_precise_window_cites_only_its_own_segments_and_may_be_empty() {
        let all = transcript();
        let first = &all[..2];
        let accepted = validate_precise_window(&precise_window().to_string(), "stop", first).expect("window");
        assert!(accepted.repairs.is_empty());
        assert_eq!(accepted.value.terms[0].term_en_source, Some(TermSource::Transcript));
        let later = &all[2..];
        assert_eq!(
            rules(validate_precise_window(&precise_window().to_string(), "stop", later)),
            vec!["unknown_source", "unknown_source", "unknown_source"]
        );
        let mut empty = precise_window();
        empty["concepts"] = json!([]);
        empty["examples"] = json!([]);
        empty["terms"] = json!([]);
        let accepted = validate_precise_window(&empty.to_string(), "stop", later).expect("a window without concepts");
        assert!(accepted.value.concepts.is_empty());
        let mut sourced = precise_window();
        sourced["terms"][0]["term_en_source"] = json!("model");
        assert_eq!(rules(validate_precise_window(&sourced.to_string(), "stop", first)), vec!["structure"]);
        let mut long = precise_window();
        long["concepts"] = json!((0..9)
            .map(|index| json!({"name": format!("개념 {index}"), "explanation": format!("설명 {index}"),
                                "source_refs": ["s1"]}))
            .collect::<Vec<_>>());
        assert_eq!(rules(validate_precise_window(&long.to_string(), "stop", first)), vec!["too_many_items"]);
        let mut twice = precise_window();
        twice["concepts"] = json!([
            {"name": "교착 상태", "explanation": "서로 기다리며 멈춘 상태", "source_refs": ["s1"]},
            {"name": "데드락", "explanation": "서로 기다리며 멈춘 상태", "source_refs": ["s1"]}
        ]);
        let accepted = validate_precise_window(&twice.to_string(), "stop", first).expect("window");
        assert_eq!(kinds(&accepted.repairs), vec!["repeat_removed"]);
    }

    #[test]
    fn the_synthesis_needs_a_topic_and_cites_only_the_merged_segments() {
        let all = transcript();
        let cited = vec![all[0].clone(), all[2].clone()];
        let accepted = validate_precise_synthesis(&synthesis().to_string(), "stop", &cited).expect("synthesis");
        assert!(accepted.repairs.is_empty());
        let mut chatter = synthesis();
        chatter["topic"]["source_refs"] = json!(["s1", "s4"]);
        assert_eq!(rules(validate_precise_synthesis(&chatter.to_string(), "stop", &cited)), vec!["unknown_source"]);
        let mut no_topic = synthesis();
        no_topic.as_object_mut().unwrap().remove("topic");
        assert_eq!(rules(validate_precise_synthesis(&no_topic.to_string(), "stop", &cited)), vec!["structure"]);
        let mut no_review = synthesis();
        no_review["review"] = json!([]);
        assert!(validate_precise_synthesis(&no_review.to_string(), "stop", &cited).is_ok());
        let ids = vec!["s1".to_string(), "s3".to_string()];
        let schema = crate::contract::generation_schema(crate::contract::PRECISE_SYNTHESIS_SCHEMA, &ids).expect("schema");
        assert_eq!(schema["$defs"]["segment"]["enum"], json!(["s1", "s3"]));
    }

    #[test]
    fn the_precise_prompts_ask_for_their_own_lists_only() {
        for prompt in [precise_window_prompt(), precise_synthesis_prompt()] {
            for rule in ["source_refs", "빈 배열", "잡담", "데이터", "한 줄"] {
                assert!(prompt.contains(rule), "prompt lacks {rule}");
            }
            for decided_by_the_app in ["from_transcript", "term_en_source"] {
                assert!(!prompt.contains(decided_by_the_app), "prompt asks for {decided_by_the_app}");
            }
        }
        assert!(precise_window_prompt().contains("공지와 코드는 따로"));
        assert!(precise_window_prompt().contains("복습 항목"));
        assert!(!precise_window_prompt().contains("topic"));
        assert!(precise_synthesis_prompt().contains("topic"));
        assert!(!precise_synthesis_prompt().contains("concepts에는"));
    }
```

- [ ] **Step 2: 실패 확인**

Run: `cd app/src-tauri && cargo test --lib lecture::tests`
Expected: `cannot find function validate_precise_window` 등으로 컴파일 FAIL.

- [ ] **Step 3: 구현**

`use crate::contract::{...}`에 `PRECISE_SYNTHESIS_SCHEMA, PRECISE_WINDOW_SCHEMA`를 더한다. 상수를 더한다.

```rust
pub const PRECISE_WINDOW_VERSION: &str = "lecture-precise-window-v1";
pub const PRECISE_SYNTHESIS_VERSION: &str = "lecture-precise-synthesis-v1";
```

```rust
pub const PRECISE_WINDOW_PROMPT_VERSION: &str = "lecture-precise-window-prompt-v1";
pub const PRECISE_SYNTHESIS_PROMPT_VERSION: &str = "lecture-precise-synthesis-prompt-v1";
```

`note_body_prompt` 아래에 프롬프트 두 개를 더한다.

```rust
pub fn precise_window_prompt() -> String {
    format!(
        "너는 한국어 대학 강의의 전사 한 구간을 자세히 정리한다. 출력 형식은 lecture-precise-window-v1이다. 공지와 코드는 따로 모으므로 쓰지 않는다.\n{COMMON_RULES}\n\
         concepts에는 이 구간에서 설명한 개념과 그 설명을 쓴다. examples에는 이 구간의 비유·예시와 예제 풀이를 쓴다.\n\
         terms는 이 구간의 주요 용어다. definition은 용어를 되풀이하지 말고 뜻을 설명한다. 영문 원어를 알면 term_en에 쓰고 모르면 null로 둔다.\n\
         복습 항목, 다음 시간 예고, 수업 진행 안내는 개념으로 쓰지 않는다."
    )
}

pub fn precise_synthesis_prompt() -> String {
    format!(
        "너는 한국어 대학 강의 한 회차의 개념 목록을 읽고 주제와 복습 항목을 쓴다. 출력 형식은 lecture-precise-synthesis-v1이다.\n{COMMON_RULES}\n\
         topic은 이번 강의의 주제 한 문장이다. source_refs에는 주제를 가장 잘 보여 주는 구간만 쓴다.\n\
         review는 복습할 항목이다. 개념 설명을 되풀이하지 말고 무엇을 복습할지 적는다."
    )
}
```

`NoteBody` 아래에 타입 두 개를 더한다.

```rust
/// One window of the precise note, read from that window's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreciseWindow {
    pub schema_version: String,
    pub concepts: Vec<Concept>,
    pub examples: Vec<Item>,
    pub terms: Vec<Term>,
}

/// The precise note's topic and review, written from the merged concepts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreciseSynthesis {
    pub schema_version: String,
    pub topic: Item,
    pub review: Vec<Item>,
}
```

`cited` 함수 아래에 영문 원어 출처 판정을 꺼낸다.

```rust
fn english_in(texts: &HashMap<&str, &str>, term: &Term) -> Option<TermSource> {
    let source = cited(texts, &term.source_refs).to_lowercase();
    term.term_en.as_ref().map(|english| {
        if source.contains(&english.to_lowercase()) {
            TermSource::Transcript
        } else {
            TermSource::Model
        }
    })
}

/// Whether a term's English form appears in the segments it cites. Merging recomputes it
/// after a term's sources grow.
pub fn english_source(segments: &[Segment], term: &Term) -> Option<TermSource> {
    let texts = segments
        .iter()
        .map(|segment| (segment.id.as_str(), segment.text.as_str()))
        .collect();
    english_in(&texts, term)
}
```

`Cleaner::terms`의 출처 계산 부분을 바꾼다.

```rust
            term.term_en_source = english_in(&self.texts, &term);
            kept.push(term);
```

`Cleaner`에 개념 정리를 더한다(기존 `validate_note_body`의 반복문을 옮긴 것).

```rust
    fn concepts(&mut self, concepts: Vec<Concept>) -> Vec<Concept> {
        let mut kept = Vec::new();
        for (index, concept) in concepts.into_iter().enumerate() {
            if self.keep(&format!("$.concepts[{index}]"), &concept.explanation) {
                kept.push(concept);
            }
        }
        kept
    }
```

`check_entry` 아래에 세 목록의 공통 검사를 더한다.

```rust
fn check_lists(checker: &mut Checker, concepts: &[Concept], examples: &[Item], terms: &[Term]) {
    for (index, concept) in concepts.iter().enumerate() {
        checker.text(&format!("$.concepts[{index}].name"), &concept.name);
        check_entry(checker, format!("$.concepts[{index}]"), &concept.explanation, &concept.source_refs);
    }
    for (index, item) in examples.iter().enumerate() {
        check_entry(checker, format!("$.examples[{index}]"), &item.content, &item.source_refs);
    }
    for (index, term) in terms.iter().enumerate() {
        checker.text(&format!("$.terms[{index}].term_ko"), &term.term_ko);
        check_entry(checker, format!("$.terms[{index}]"), &term.definition, &term.source_refs);
    }
}
```

`validate_note_body`를 두 도우미로 바꾼다(동작은 같다).

```rust
pub fn validate_note_body(content: &str, finish_reason: &str, segments: &[Segment]) -> Result<Accepted<NoteBody>, Vec<Violation>> {
    let mut body: NoteBody = parse(content, finish_reason)?;
    let mut cleaner = Cleaner::new(segments);
    body.concepts = cleaner.concepts(std::mem::take(&mut body.concepts));
    body.examples = cleaner.items("$.examples", std::mem::take(&mut body.examples));
    body.review = cleaner.items("$.review", std::mem::take(&mut body.review));
    body.terms = cleaner.terms(std::mem::take(&mut body.terms));

    let mut checker = Checker::new(segments);
    version(&mut checker, &body.schema_version, NOTE_BODY_VERSION);
    required_section(&mut checker, "$.concepts", body.concepts.len());
    bounded(
        &mut checker,
        NOTE_BODY_SCHEMA,
        &[
            ("concepts", body.concepts.len()),
            ("examples", body.examples.len()),
            ("terms", body.terms.len()),
            ("review", body.review.len()),
        ],
    );
    check_entry(&mut checker, "$.topic".into(), &body.topic.content, &body.topic.source_refs);
    check_lists(&mut checker, &body.concepts, &body.examples, &body.terms);
    for (index, item) in body.review.iter().enumerate() {
        check_entry(&mut checker, format!("$.review[{index}]"), &item.content, &item.source_refs);
    }
    accept(body, checker, cleaner.repairs)
}
```

검증 두 개를 더한다.

```rust
/// One window of the precise note. Only the window's segments may be cited. A window without
/// concepts (a break, chatter) is accepted.
pub fn validate_precise_window(content: &str, finish_reason: &str, window: &[Segment]) -> Result<Accepted<PreciseWindow>, Vec<Violation>> {
    let mut answer: PreciseWindow = parse(content, finish_reason)?;
    let mut cleaner = Cleaner::new(window);
    answer.concepts = cleaner.concepts(std::mem::take(&mut answer.concepts));
    answer.examples = cleaner.items("$.examples", std::mem::take(&mut answer.examples));
    answer.terms = cleaner.terms(std::mem::take(&mut answer.terms));
    let mut checker = Checker::new(window);
    version(&mut checker, &answer.schema_version, PRECISE_WINDOW_VERSION);
    bounded(
        &mut checker,
        PRECISE_WINDOW_SCHEMA,
        &[
            ("concepts", answer.concepts.len()),
            ("examples", answer.examples.len()),
            ("terms", answer.terms.len()),
        ],
    );
    check_lists(&mut checker, &answer.concepts, &answer.examples, &answer.terms);
    accept(answer, checker, cleaner.repairs)
}

/// The precise note's topic and review. `cited` holds the segments the merged note cites, so
/// the topic cannot reach segments every window left out, such as chatter.
pub fn validate_precise_synthesis(content: &str, finish_reason: &str, cited: &[Segment]) -> Result<Accepted<PreciseSynthesis>, Vec<Violation>> {
    let mut answer: PreciseSynthesis = parse(content, finish_reason)?;
    let mut cleaner = Cleaner::new(cited);
    answer.review = cleaner.items("$.review", std::mem::take(&mut answer.review));
    let mut checker = Checker::new(cited);
    version(&mut checker, &answer.schema_version, PRECISE_SYNTHESIS_VERSION);
    bounded(&mut checker, PRECISE_SYNTHESIS_SCHEMA, &[("review", answer.review.len())]);
    check_entry(&mut checker, "$.topic".into(), &answer.topic.content, &answer.topic.source_refs);
    for (index, item) in answer.review.iter().enumerate() {
        check_entry(&mut checker, format!("$.review[{index}]"), &item.content, &item.source_refs);
    }
    accept(answer, checker, cleaner.repairs)
}
```

- [ ] **Step 4: 통과 확인**

Run: `cd app/src-tauri && cargo test --lib`
Expected: 새 테스트 3개를 포함해 모두 PASS. 기존 `filler_repeats_and_circular_definitions_are_dropped`와 `the_app_labels_code_and_english_terms_from_the_cited_text`가 통과하면 `validate_note_body`의 동작은 그대로다.

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/lecture.rs
git commit -m "feat: validate precise-note window and synthesis answers"
```

---

### Task 3: 창별 결과 병합과 정밀 노트 조립

**Files:**
- Modify: `app/src-tauri/src/lecture_merge.rs`

**Interfaces:**
- Consumes: Task 2의 `PreciseWindow`, `PreciseSynthesis`, `english_source`, 기존 `collapse`, `Accepted`, `Repair`.
- Produces:
  - `pub struct PreciseBody { pub concepts: Vec<Concept>, pub examples: Vec<Item>, pub terms: Vec<Term> }`(`Default`, `Serialize`)
  - `pub fn name_key(name: &str) -> String`
  - `pub fn merge_precise(windows: Vec<Accepted<PreciseWindow>>, segments: &[Segment]) -> Accepted<PreciseBody>` — 수선 경로는 `window{n}:`(1부터)로 시작한다
  - `pub fn synthesis_input(body: &PreciseBody) -> String`
  - `pub fn cited_by(body: &PreciseBody, all: &[Segment]) -> Vec<Segment>`
  - `pub fn assemble_precise_note(body: Accepted<PreciseBody>, synthesis: Accepted<PreciseSynthesis>, drafts: &[Draft]) -> Accepted<Note>`

- [ ] **Step 1: 실패하는 테스트 작성**

테스트 모듈의 `use`에 `use crate::lecture::{PreciseSynthesis, PreciseWindow, TermSource};`를 더하고, 도우미와 테스트를 더한다.

```rust
    fn refs(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn concept(name: &str, explanation: &str, ids: &[&str]) -> Concept {
        Concept { name: name.into(), explanation: explanation.into(), source_refs: refs(ids) }
    }

    fn item(content: &str, ids: &[&str]) -> Item {
        Item { content: content.into(), source_refs: refs(ids) }
    }

    fn term(korean: &str, definition: &str, english: Option<&str>, ids: &[&str]) -> Term {
        Term {
            term_ko: korean.into(),
            definition: definition.into(),
            term_en: english.map(str::to_string),
            term_en_source: None,
            source_refs: refs(ids),
        }
    }

    fn precise(concepts: Vec<Concept>, examples: Vec<Item>, terms: Vec<Term>) -> Accepted<PreciseWindow> {
        accepted(PreciseWindow { schema_version: "lecture-precise-window-v1".into(), concepts, examples, terms })
    }

    fn lecture() -> Vec<Segment> {
        parse_transcript(
            "[s1 00:00] 교착 상태, 영어로 deadlock\n[s2 00:20] 은행원 알고리즘\n\
             [s13 05:00] 교착 상태를 다시 봅니다\n[s14 05:20] 안전 상태",
        )
        .expect("segments")
    }

    #[test]
    fn names_match_once_spacing_trailing_punctuation_and_case_are_ignored() {
        assert_eq!(name_key(" 교착  상태. "), name_key("교착 상태"));
        assert_eq!(name_key("Banker's Algorithm"), name_key("banker's algorithm"));
        assert_ne!(name_key("안전 상태"), name_key("안전한 상태"));
    }

    #[test]
    fn concepts_with_the_same_name_are_merged_keeping_both_explanations() {
        let first = precise(
            vec![concept("교착 상태", "서로 기다리며 멈춘 상태", &["s1"]), concept("은행원 알고리즘", "안전 상태를 지키는 방법", &["s2"])],
            vec![],
            vec![],
        );
        let second = precise(
            vec![
                concept("교착 상태.", "서로 기다리며  멈춘 상태", &["s13"]),
                concept("교착 상태", "네 조건이 모두 성립할 때 생긴다", &["s13"]),
                concept("안전 상태", "모두 끝낼 수 있는 순서가 있는 상태", &["s14"]),
            ],
            vec![],
            vec![],
        );
        let merged = merge_precise(vec![first, second], &lecture());
        let concepts = &merged.value.concepts;
        assert_eq!(concepts.len(), 3);
        assert_eq!(concepts[0].explanation, "서로 기다리며 멈춘 상태 네 조건이 모두 성립할 때 생긴다");
        assert_eq!(concepts[0].source_refs, refs(&["s1", "s13"]));
        assert_eq!(kinds(&merged.repairs), vec!["concept_merged", "concept_merged"]);
        assert_eq!(merged.repairs[0].path, "window2:$.concepts[0]");
        assert_eq!(merged.repairs[1].path, "window2:$.concepts[1]");
    }

    #[test]
    fn terms_merge_like_concepts_and_the_english_source_is_recomputed() {
        let merged = merge_precise(
            vec![
                precise(vec![], vec![], vec![term("교착 상태", "서로 기다리며 멈춘 상태", None, &["s13"])]),
                precise(vec![], vec![], vec![term("교착 상태", "자원을 서로 기다리는 상태", Some("deadlock"), &["s1"])]),
                precise(vec![], vec![], vec![term("교착 상태", "서로 기다리며 멈춘 상태", Some("Impasse"), &["s13"])]),
            ],
            &lecture(),
        );
        let terms = &merged.value.terms;
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].definition, "서로 기다리며 멈춘 상태 자원을 서로 기다리는 상태");
        assert_eq!(terms[0].term_en.as_deref(), Some("deadlock"));
        assert_eq!(terms[0].term_en_source, Some(TermSource::Transcript));
        assert_eq!(terms[0].source_refs, refs(&["s13", "s1"]));
        assert_eq!(kinds(&merged.repairs), vec!["term_merged", "term_merged"]);
    }

    #[test]
    fn identical_examples_across_windows_are_merged() {
        let merged = merge_precise(
            vec![
                precise(vec![], vec![item("프린터와 스캐너를 서로 기다리는 예", &["s1"])], vec![]),
                precise(
                    vec![],
                    vec![item("프린터와  스캐너를 서로 기다리는 예", &["s13"]), item("철학자 예", &["s14"])],
                    vec![],
                ),
            ],
            &lecture(),
        );
        assert_eq!(merged.value.examples.len(), 2);
        assert_eq!(merged.value.examples[0].source_refs, refs(&["s1", "s13"]));
        assert_eq!(kinds(&merged.repairs), vec!["example_merged"]);
    }

    #[test]
    fn window_repairs_keep_their_window_and_the_synthesis_reads_the_merged_concepts() {
        let mut first = precise(vec![concept("교착 상태", "멈춘 상태", &["s1"])], vec![item("예", &["s2"])], vec![]);
        first.repairs.push(Repair { path: "$.examples[1]".into(), kind: "repeat_removed", detail: String::new() });
        let merged = merge_precise(vec![first, precise(vec![], vec![], vec![])], &lecture());
        assert_eq!(merged.repairs[0].path, "window1:$.examples[1]");
        assert_eq!(synthesis_input(&merged.value), "- 교착 상태 [s1]: 멈춘 상태");
        let cited: Vec<String> = cited_by(&merged.value, &lecture()).into_iter().map(|segment| segment.id).collect();
        assert_eq!(cited, refs(&["s1", "s2"]));
    }

    #[test]
    fn a_precise_note_takes_topic_and_review_from_the_synthesis_and_notices_from_the_drafts() {
        let body = merge_precise(vec![precise(vec![concept("교착 상태", "멈춘 상태", &["s1"])], vec![], vec![])], &lecture());
        let synthesis = accepted(PreciseSynthesis {
            schema_version: "lecture-precise-synthesis-v1".into(),
            topic: item("교착 상태", &["s1"]),
            review: vec![item("네 조건을 복습한다", &["s1"])],
        });
        let drafts = [
            Draft {
                window: Window { first: "s1".into(), last: "s12".into() },
                points: vec![],
                notices: vec![notice(NoticeKind::Exam, "중간고사", Some("10월 21일"), &["s9"])],
                code: vec![],
            },
            Draft {
                window: Window { first: "s13".into(), last: "s24".into() },
                points: vec![],
                notices: vec![
                    notice(NoticeKind::Exam, "중간고사", Some("10월 21일"), &["s9"]),
                    notice(NoticeKind::Assignment, "은행원 알고리즘 구현", None, &["s22"]),
                ],
                code: vec![code("chmod 755 run.sh", true)],
            },
        ];
        let note = assemble_precise_note(body, synthesis, &drafts);
        assert_eq!(note.value.topic.content, "교착 상태");
        assert_eq!(note.value.review.len(), 1);
        assert_eq!(note.value.concepts.len(), 1);
        assert_eq!(note.value.notices.len(), 2);
        assert_eq!(note.value.code.len(), 1);
        assert_eq!(kinds(&note.repairs), vec!["notice_merged"]);
        assert_eq!(note.repairs[0].path, "note:$.notices[1]");
    }
```

- [ ] **Step 2: 실패 확인**

Run: `cd app/src-tauri && cargo test --lib lecture_merge`
Expected: `cannot find function merge_precise` 등으로 컴파일 FAIL.

- [ ] **Step 3: 구현**

`use` 줄을 바꾼다.

```rust
use std::collections::BTreeSet;

use serde::Serialize;

use crate::contract::Segment;
use crate::lecture::{
    collapse, english_source, Accepted, Code, Concept, Item, Notice, NoteBody, PreciseSynthesis, PreciseWindow, Repair,
    Term,
};
```

모듈 설명 끝에 한 문단을 더한다.

```rust
//!
//! The precise note is read one window at a time; its lists are merged here by name, and a
//! last call writes only the topic and the review over the merged concepts.
```

`Note` 아래에 병합 결과 타입을 더한다.

```rust
/// The precise note before its topic and review: every window's lists in order, with equal
/// names merged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PreciseBody {
    pub concepts: Vec<Concept>,
    pub examples: Vec<Item>,
    pub terms: Vec<Term>,
}
```

`assemble_note`를 공지·코드 모으기와 나눈다(동작과 수선 순서는 같다).

```rust
/// Every draft's notices and code with duplicates across windows dropped.
fn gathered(drafts: &[Draft]) -> (Vec<Notice>, Vec<Code>, Vec<Repair>) {
    let notices: Vec<Notice> = drafts.iter().flat_map(|draft| draft.notices.clone()).collect();
    let code: Vec<Code> = drafts.iter().flat_map(|draft| draft.code.clone()).collect();
    let (notices, notice_repairs) = merge_notices(notices, "$.notices");
    let (code, code_repairs) = merge_code(code, "$.code");
    let repairs = tagged("note", notice_repairs).chain(tagged("note", code_repairs)).collect();
    (notices, code, repairs)
}

/// The note: the body as written, then every draft's notices and code with duplicates
/// across windows dropped.
pub fn assemble_note(body: Accepted<NoteBody>, drafts: &[Draft]) -> Accepted<Note> {
    let (notices, code, note_repairs) = gathered(drafts);
    let repairs = tagged("body", body.repairs).chain(note_repairs).collect();
    let body = body.value;
    Accepted {
        value: Note {
            topic: body.topic,
            concepts: body.concepts,
            examples: body.examples,
            terms: body.terms,
            notices,
            code,
            review: body.review,
        },
        repairs,
    }
}
```

병합 함수들을 더한다.

```rust
const TRAILING_PUNCTUATION: [char; 7] = ['.', ',', '!', '?', ':', ';', '。'];

/// Names compare equal once spacing is collapsed, trailing punctuation dropped and Latin
/// letters lowercased. Different wording ("안전 상태", "안전한 상태") stays different.
pub fn name_key(name: &str) -> String {
    collapse(name)
        .trim_end_matches(&TRAILING_PUNCTUATION[..])
        .trim_end()
        .to_lowercase()
}

fn union(into: &mut Vec<String>, refs: &[String]) {
    for id in refs {
        if !into.contains(id) {
            into.push(id.clone());
        }
    }
}

/// Adds `text` after `into` unless `into` already says it, spacing aside. Both sentences
/// passed their own call's checks; nothing new is written.
fn append(into: &mut String, text: &str) -> bool {
    if collapse(into).contains(&collapse(text)) {
        return false;
    }
    into.push(' ');
    into.push_str(text.trim());
    true
}

fn merged(stage: &str, list: &str, position: usize, kind: &'static str, first: usize, appended: bool) -> Repair {
    let detail = if appended {
        format!("merged into kept item {first}, text appended")
    } else {
        format!("merged into kept item {first}")
    };
    Repair { path: format!("{stage}:$.{list}[{position}]"), kind, detail }
}

/// Joins the windows of the precise note in order. Concepts and terms with the same name and
/// identical examples become one item with the sources of all.
pub fn merge_precise(windows: Vec<Accepted<PreciseWindow>>, segments: &[Segment]) -> Accepted<PreciseBody> {
    let mut body = PreciseBody::default();
    let mut repairs = Vec::new();
    for (index, window) in windows.into_iter().enumerate() {
        let stage = format!("window{}", index + 1);
        repairs.extend(tagged(&stage, window.repairs));
        let value = window.value;
        for (position, concept) in value.concepts.into_iter().enumerate() {
            match body.concepts.iter().position(|kept| name_key(&kept.name) == name_key(&concept.name)) {
                Some(first) => {
                    let kept = &mut body.concepts[first];
                    let appended = append(&mut kept.explanation, &concept.explanation);
                    union(&mut kept.source_refs, &concept.source_refs);
                    repairs.push(merged(&stage, "concepts", position, "concept_merged", first, appended));
                }
                None => body.concepts.push(concept),
            }
        }
        for (position, example) in value.examples.into_iter().enumerate() {
            match body.examples.iter().position(|kept| collapse(&kept.content) == collapse(&example.content)) {
                Some(first) => {
                    union(&mut body.examples[first].source_refs, &example.source_refs);
                    repairs.push(merged(&stage, "examples", position, "example_merged", first, false));
                }
                None => body.examples.push(example),
            }
        }
        for (position, term) in value.terms.into_iter().enumerate() {
            match body.terms.iter().position(|kept| name_key(&kept.term_ko) == name_key(&term.term_ko)) {
                Some(first) => {
                    let kept = &mut body.terms[first];
                    let appended = append(&mut kept.definition, &term.definition);
                    if kept.term_en.is_none() {
                        kept.term_en = term.term_en;
                    }
                    union(&mut kept.source_refs, &term.source_refs);
                    repairs.push(merged(&stage, "terms", position, "term_merged", first, appended));
                }
                None => body.terms.push(term),
            }
        }
    }
    for term in &mut body.terms {
        term.term_en_source = english_source(segments, term);
    }
    Accepted { value: body, repairs }
}

/// What the synthesis call reads: one merged concept per line with its sources.
pub fn synthesis_input(body: &PreciseBody) -> String {
    body.concepts
        .iter()
        .map(|concept| format!("- {} [{}]: {}", concept.name, concept.source_refs.join(", "), concept.explanation))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The segments the merged body cites, in transcript order: all the synthesis may quote.
pub fn cited_by(body: &PreciseBody, all: &[Segment]) -> Vec<Segment> {
    let cited: BTreeSet<&String> = body
        .concepts
        .iter()
        .flat_map(|concept| &concept.source_refs)
        .chain(body.examples.iter().flat_map(|item| &item.source_refs))
        .chain(body.terms.iter().flat_map(|term| &term.source_refs))
        .collect();
    all.iter().filter(|segment| cited.contains(&segment.id)).cloned().collect()
}

/// The precise note: merged windows, the synthesis's topic and review, and the drafts'
/// notices and code.
pub fn assemble_precise_note(
    body: Accepted<PreciseBody>,
    synthesis: Accepted<PreciseSynthesis>,
    drafts: &[Draft],
) -> Accepted<Note> {
    let (notices, code, note_repairs) = gathered(drafts);
    let repairs = body
        .repairs
        .into_iter()
        .chain(tagged("synthesis", synthesis.repairs))
        .chain(note_repairs)
        .collect();
    Accepted {
        value: Note {
            topic: synthesis.value.topic,
            concepts: body.value.concepts,
            examples: body.value.examples,
            terms: body.value.terms,
            notices,
            code,
            review: synthesis.value.review,
        },
        repairs,
    }
}
```

- [ ] **Step 4: 통과 확인**

Run: `cd app/src-tauri && cargo test --lib`
Expected: 새 테스트 6개와 기존 `a_note_gathers_notices_and_code_from_every_draft`를 포함해 모두 PASS.

- [ ] **Step 5: 커밋**

```bash
git add app/src-tauri/src/lecture_merge.rs
git commit -m "feat: merge precise-note windows by name and assemble the precise note"
```

---

### Task 4: 하네스

**Files:**
- Modify: `app/src-tauri/examples/lecture_contract_check.rs`

**Interfaces:**
- Consumes: Task 1~3의 스키마 상수, 검증, 프롬프트, `merge_precise`, `synthesis_input`, `cited_by`, `assemble_precise_note`, `name_key`.
- Produces: `lecture-contract.json`의 단계 `precise_window`(창마다), `precise_synthesis`(회차마다), 노트 종류 `note_precise`, 요약 `summary.precise`(`notes`, `cover_every_window`, `duplicate_names`, `concept_counts`), `per_run[].precise_seconds`.

- [ ] **Step 1: 머리 설명과 가져오기 바꾸기**

파일 머리 설명의 둘째 문단을 바꾼다.

```rust
//! One run asks three times per five-minute window (points, notices, code) and assembles a
//! draft, then writes two notes. The five-minute note's body is written from the drafts'
//! points. The precise note is read one window at a time from the transcript, merged by the
//! app and finished by one call for topic and review. Notices and code in both notes come
//! from the drafts. A refused answer is retried once with its violations attached, and the
//! retry counts towards the time.
```

가져오기를 바꾼다.

```rust
use app_lib::contract::{
    generation_schema, Segment, Violation, CODE_SCHEMA, NOTE_BODY_SCHEMA, NOTICES_SCHEMA, POINTS_SCHEMA,
    PRECISE_SYNTHESIS_SCHEMA, PRECISE_WINDOW_SCHEMA,
};
use app_lib::lecture::{
    code_prompt, note_body_prompt, notices_prompt, points_prompt, precise_synthesis_prompt, precise_window_prompt,
    validate_code, validate_note_body, validate_notices, validate_points, validate_precise_synthesis,
    validate_precise_window, Accepted, PreciseWindow, Repair, CODE_PROMPT_VERSION, NOTE_BODY_PROMPT_VERSION,
    NOTICES_PROMPT_VERSION, POINTS_PROMPT_VERSION, PRECISE_SYNTHESIS_PROMPT_VERSION, PRECISE_WINDOW_PROMPT_VERSION,
    PRECISE_WINDOW_VERSION,
};
use app_lib::lecture_fixture::{check_draft, check_note, Expectation, Fixture};
use app_lib::lecture_merge::{
    assemble_draft, assemble_note, assemble_precise_note, cited_by, merge_precise, name_key, synthesis_input, Draft,
    Note,
};
```

- [ ] **Step 2: 도우미 더하기**

`empty` 아래에 더한다.

```rust
/// A precise window refused twice adds nothing, but keeps its place in the numbering.
fn empty_window() -> Accepted<PreciseWindow> {
    Accepted {
        value: PreciseWindow {
            schema_version: PRECISE_WINDOW_VERSION.into(),
            concepts: Vec::new(),
            examples: Vec::new(),
            terms: Vec::new(),
        },
        repairs: Vec::new(),
    }
}

/// Whether each precise note reaches every window, how many names repeat after merging, and
/// how many concepts each note has.
fn precise_summary(notes: &[Assembled<Note>], windows: &[Vec<Segment>]) -> serde_json::Value {
    let precise: Vec<&Assembled<Note>> = notes.iter().filter(|note| note.kind == "note_precise").collect();
    let covers = |note: &Note, window: &[Segment]| {
        note.concepts
            .iter()
            .flat_map(|concept| &concept.source_refs)
            .any(|id| window.iter().any(|segment| &segment.id == id))
    };
    let cover_every_window = precise
        .iter()
        .filter(|note| windows.iter().all(|window| covers(&note.value, window)))
        .count();
    let duplicate_names: usize = precise
        .iter()
        .map(|note| {
            let mut seen = BTreeSet::new();
            note.value
                .concepts
                .iter()
                .map(|concept| format!("concept:{}", name_key(&concept.name)))
                .chain(note.value.terms.iter().map(|term| format!("term:{}", name_key(&term.term_ko))))
                .filter(|key| !seen.insert(key.clone()))
                .count()
        })
        .sum();
    let concept_counts: Vec<usize> = precise.iter().map(|note| note.value.concepts.len()).collect();
    serde_json::json!({
        "notes": precise.len(),
        "cover_every_window": cover_every_window,
        "duplicate_names": duplicate_names,
        "concept_counts": concept_counts,
    })
}
```

- [ ] **Step 3: 요약 바꾸기**

`summarize`의 서명과 단계 목록을 바꾼다.

```rust
fn summarize(
    calls: &[Call],
    drafts: &[Assembled<Draft>],
    notes: &[Assembled<Note>],
    runs: usize,
    windows: &[Vec<Segment>],
) -> serde_json::Value {
    let mut stages = serde_json::Map::new();
    for stage in ["points", "notices", "code", "note_body_from_drafts", "precise_window", "precise_synthesis"] {
```

`per_run`의 `note` 계산 아래에 정밀 정리 시간을 더하고 JSON에 넣는다.

```rust
            let precise = notes
                .iter()
                .filter(|note| note.run == run && note.kind == "note_precise")
                .map(|note| note.seconds)
                .fold(0.0, f64::max);
            serde_json::json!({
                "run": run,
                "max_window_seconds": round(window, 1),
                "note_body_from_drafts_seconds": round(note, 1),
                "after_recording_estimate_seconds": round(2.0 * window + note, 1),
                "precise_seconds": round(precise, 1),
            })
```

반환 JSON에 `"precise": precise_summary(notes, windows),`를 `"per_run": per_run,` 앞에 더한다.

- [ ] **Step 4: 프롬프트 목록과 정밀 정리 흐름 바꾸기**

`prompts` 배열에 두 개를 더한다(배열 길이 6).

```rust
    let prompts = [
        (POINTS_PROMPT_VERSION, points_prompt()),
        (NOTICES_PROMPT_VERSION, notices_prompt()),
        (CODE_PROMPT_VERSION, code_prompt()),
        (NOTE_BODY_PROMPT_VERSION, note_body_prompt()),
        (PRECISE_WINDOW_PROMPT_VERSION, precise_window_prompt()),
        (PRECISE_SYNTHESIS_PROMPT_VERSION, precise_synthesis_prompt()),
    ];
```

`let inputs = [...]`부터 그 `for` 반복문 끝까지를 5분 노트 한 개와 정밀 정리로 바꾼다.

```rust
        let cited = cited_segments(&drafts, &all);
        let points_json: Vec<String> = drafts
            .iter()
            .map(|draft| serde_json::json!({"window": draft.window, "points": draft.points}).to_string())
            .collect();
        let user = format!(
            "과목: {}\n\n구간 요점:\n{}\n\n요점이 인용한 전사 구간:\n{}",
            fixture.course,
            points_json.join("\n"),
            lines(&cited)
        );
        let (body, attempts) = endpoint.ask(
            &format!("run{run}-note_from_drafts"),
            &prompts[3].1,
            &user,
            &generation_schema(NOTE_BODY_SCHEMA, &ids(&cited))?,
            NOTE_MAX_TOKENS,
            |completion| validate_note_body(&completion.content, &completion.finish_reason, &all),
        )?;
        let accepted = body.is_some();
        let record = call("note_body_from_drafts", run, None, attempts, accepted);
        let seconds = record.seconds;
        calls.push(record);
        println!("run {run} note_from_drafts accepted {accepted} in {seconds:.1}s");
        if let Some(body) = body {
            let note = assemble_note(body, &drafts);
            let expectations = check_note(&note.value, &fixture.traps);
            notes_record.push(Assembled {
                kind: "note_from_drafts",
                run,
                window: None,
                complete: true,
                seconds,
                repairs: note.repairs,
                expectations,
                value: note.value,
            });
        }

        // The precise note: each window from its own transcript, merged by the app, then one
        // call for topic and review over the merged concepts.
        let mut precise_windows = Vec::new();
        let mut precise_seconds = 0.0;
        let mut precise_complete = true;
        for (index, window) in windows.iter().enumerate() {
            let (answer, attempts) = endpoint.ask(
                &format!("run{run}-precise-window{}", index + 1),
                &prompts[4].1,
                &format!("과목: {}\n\n전사:\n{}", fixture.course, lines(window)),
                &generation_schema(PRECISE_WINDOW_SCHEMA, &ids(window))?,
                WINDOW_MAX_TOKENS,
                |completion| validate_precise_window(&completion.content, &completion.finish_reason, window),
            )?;
            let record = call("precise_window", run, Some(index + 1), attempts, answer.is_some());
            precise_seconds += record.seconds;
            precise_complete &= record.accepted;
            println!("run {run} precise window {} accepted {} in {:.1}s", index + 1, record.accepted, record.seconds);
            calls.push(record);
            precise_windows.push(answer.unwrap_or_else(empty_window));
        }
        let body = merge_precise(precise_windows, &all);
        let merged_cited = cited_by(&body.value, &all);
        if merged_cited.is_empty() {
            println!("run {run} note_precise skipped: no window cited anything");
            continue;
        }
        let (synthesis, attempts) = endpoint.ask(
            &format!("run{run}-precise-synthesis"),
            &prompts[5].1,
            &format!("과목: {}\n\n개념 목록:\n{}", fixture.course, synthesis_input(&body.value)),
            &generation_schema(PRECISE_SYNTHESIS_SCHEMA, &ids(&merged_cited))?,
            WINDOW_MAX_TOKENS,
            |completion| validate_precise_synthesis(&completion.content, &completion.finish_reason, &merged_cited),
        )?;
        let record = call("precise_synthesis", run, None, attempts, synthesis.is_some());
        precise_seconds += record.seconds;
        calls.push(record);
        println!("run {run} note_precise accepted {} in {precise_seconds:.1}s", synthesis.is_some());
        if let Some(synthesis) = synthesis {
            let note = assemble_precise_note(body, synthesis, &drafts);
            let expectations = check_note(&note.value, &fixture.traps);
            notes_record.push(Assembled {
                kind: "note_precise",
                run,
                window: None,
                complete: precise_complete,
                seconds: round(precise_seconds, 2),
                repairs: note.repairs,
                expectations,
                value: note.value,
            });
        }
```

`report`의 `max_tokens`와 `summarize` 호출을 바꾼다.

```rust
        "max_tokens": {"window_call": WINDOW_MAX_TOKENS, "note_body": NOTE_MAX_TOKENS,
                       "precise_window": WINDOW_MAX_TOKENS, "precise_synthesis": WINDOW_MAX_TOKENS},
        "summary": summarize(&calls, &drafts_record, &notes_record, runs, &windows),
```

- [ ] **Step 5: 빌드와 전체 테스트**

Run: `cd app/src-tauri && cargo build --release --example lecture_contract_check && cargo test`
Expected: 경고 없이 빌드되고 테스트가 모두 PASS(기존 5개 무시 포함).

- [ ] **Step 6: 1회 시험**

전원 상태를 읽어 AC·"최고 성능"인지 확인한다(바꾸지 않는다).

Run: `python -c "import json, scripts.bench_env as e; print(json.dumps({'power': e.power_status(), 'mode': e.power_mode()}))"`

Run: `app/src-tauri/target/release/examples/lecture_contract_check.exe "C:\temp_git\Just-a-Click-" "C:\temp_git\Just-a-Click-\artifacts\lecture-contract-v2\precise-trial" 1`
Expected: `run 1 precise window 1~3 accepted true`, `run 1 note_precise accepted true`, 끝에 `written to ...`. 요약의 `precise.cover_every_window`가 1이고 `duplicate_names`가 0인지 본다. 원응답 `raw/run1-precise-window*.json`을 읽어 개념이 창 내용과 맞는지, 복습·예고가 개념으로 들어가지 않았는지 확인한다. 문제가 있으면 프롬프트만 고치고 이 단계를 반복하며, 고친 내용을 보고서에 적는다.

- [ ] **Step 7: 커밋**

```bash
git add app/src-tauri/examples/lecture_contract_check.rs
git commit -m "test: run the precise note by window in the lecture harness"
```

---

### Task 5: 5회 측정

**Files:**
- 결과: `artifacts/lecture-contract-v2/precise/`(커밋하지 않음)

- [ ] **Step 1: 조건 확인**

전원 상태, 배터리(100%·충전 0), 쉴 때 CPU 사용률과 상위 프로세스를 기록한다. OneDrive 동기화가 돌고 있으면 멈출지 사용자에게 묻는다. 남은 `llama-server`·하네스 프로세스가 없는지 확인한다.

```powershell
python -c "import json, scripts.bench_env as e; print(json.dumps({'power': e.power_status(), 'mode': e.power_mode()}))"
Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus | Select-Object ChargeRate,RemainingCapacity,PowerOnline
Get-Process llama-server,lecture_contract_check,OneDrive* -ErrorAction SilentlyContinue | Select-Object ProcessName,Id
```

- [ ] **Step 2: 하네스와 카운터 수집기 시작**

```powershell
$root = 'C:\temp_git\Just-a-Click-'; $out = "$root\artifacts\lecture-contract-v2\precise"; New-Item -ItemType Directory -Force $out | Out-Null; $start = Get-Date; "start $($start.ToString('o'))" | Out-File -Encoding utf8 "$out\times.txt"; $harness = Start-Process "$root\app\src-tauri\target\release\examples\lecture_contract_check.exe" -ArgumentList "`"$root`"", "`"$out`"", '5' -RedirectStandardOutput "$out\harness.log" -RedirectStandardError "$out\harness.err" -PassThru -WindowStyle Hidden; $sampler = Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$root\scripts\sample_counters.ps1", '-Csv', "$out\counters.csv", '-WatchPid', $harness.Id, '-IntervalSeconds', '5' -PassThru -WindowStyle Hidden; "harness pid $($harness.Id) sampler pid $($sampler.Id)" | Out-File -Append -Encoding utf8 "$out\times.txt"; Get-Content "$out\times.txt"
```

`harness.log`에 `written to`가 나오거나 하네스가 끝날 때까지 기다린다. 오류·AC 해제·OneDrive 재시작을 감시한다.

- [ ] **Step 3: 끝 상태 기록과 집계**

끝의 전원 상태를 기록하고, 요약과 비교 지표를 뽑는다.

```bash
python - <<'EOF'
import json, statistics
d = json.load(open("artifacts/lecture-contract-v2/precise/lecture-contract.json", encoding="utf-8"))
s = d["summary"]
print("all_accepted", s["all_accepted"], "notes_missing", s["notes_missing"], "gating_all_passed", s["gating_all_passed"])
print("precise", json.dumps(s["precise"]))
for stage in ["precise_window", "precise_synthesis", "note_body_from_drafts"]:
    print(stage, json.dumps(s["stages"][stage]))
print("per_run", json.dumps(s["per_run"]))
for name, v in s["expectations"].items():
    print(name, v)
missing = ["세마포어", "탐지", "회복", "예방", "회피"]
for note in d["notes"]:
    if note["kind"] != "note_precise":
        continue
    names = " ".join(c["name"] for c in note["value"]["concepts"])
    print("run", note["run"], "concepts", len(note["value"]["concepts"]),
          "missing", [word for word in missing if word not in names],
          "repairs", sorted({r["kind"] for r in note["repairs"]}))
EOF
```

기존 정밀 정리(두 재측정 10건)와 비교할 값: 개념 수(4 또는 13~20), 창 포괄(4/10), 같은 이름 반복(긴 쪽에서 발생), 시간(60~354초).

---

### Task 6: 보고서와 결정 갱신

**Files:**
- Create: `docs/validation/<측정 날짜>-precise-note-by-window.md`
- Modify: `docs/decisions/0010-lecture-note-contract.md`, `docs/ROADMAP.md`(작업 8 결과 문단 끝), `README.md`(검증 목록)

- [ ] **Step 1: 보고서 작성**

v2 보고서와 같은 머리(날짜·범위·조건·구성·입력·결과 파일)를 쓰고 다음 절을 둔다.

1. 요약: 채택 여부와 핵심 수치.
2. 기존 한 번 읽기 방식과 비교: 개념 수 범위, 창 포괄, 같은 이름 반복, 빠졌던 개념(세마포어·탐지·회복·예방·회피) 포함 여부, 판정용 기대치, 호출 채택.
3. 호출별 결과: `precise_window`·`precise_synthesis`의 건수·첫 시도 깨끗함·채택·출력 토큰·시간·정리 종류. 병합 기록(`concept_merged` 등) 건수.
4. 시간 추정: 창 호출 중앙값 × 30 + 합성 호출로 2.5시간 강의의 정밀 정리 시간을 추정하고 다시 요약 1시간 목표와 비교한다(추정 표시). 합성 입력 토큰이 개념 수에 비례해 늘어나므로, 측정한 합성 입력 토큰에서 창 30개일 때의 입력 크기를 추정해 컨텍스트 8,192와 비교한다.
5. 원응답에서 본 문제: 덧붙인 설명의 읽기 흐름, 창 경계에 나뉜 설명, 표현만 다른 이름.
6. 판정: 설계 6절 표의 기준별 통과 여부.
7. 한계.

- [ ] **Step 2: 결정·로드맵·README 갱신**

결정 0010에 `## 7. 정밀 정리: 창별 호출 (<날짜>)` 절을 더해 결정과 결과를 적고, 머리의 근거 측정에 보고서 링크를 더한다. 로드맵 작업 8 결과 문단 끝에 한 문장과 링크를, README 검증 목록에 보고서 링크를 더한다.

Run: `python artifacts/check_docs.py`
Expected: `broken links: none`

- [ ] **Step 3: 커밋**

```bash
git add docs/validation/<측정 날짜>-precise-note-by-window.md docs/decisions/0010-lecture-note-contract.md docs/ROADMAP.md README.md
git commit -m "docs: measure the precise note read by window"
```

---

### Task 4b: 요점 목록과 포괄 검사 (시험 후 보완)

설계 10절. Task 4의 시험 두 번 가운데 두 번째에서 3창 호출이 개념 1개만 쓰고 멈췄다.

**Files:**
- Modify: `app/src-tauri/src/lecture.rs`(`precise_window_prompt`, `pub fn uncovered_points(window: &PreciseWindow, checklist: &[Item]) -> Vec<Item>`)
- Modify: `app/src-tauri/src/lecture_merge.rs`(`pub fn precise_checklist(draft: &Draft) -> Vec<Item>`)
- Modify: `app/src-tauri/examples/lecture_contract_check.rs`(창 입력에 요점 목록, 빠진 요점 재질문, `Call.uncovered`, 요약 `precise.uncovered_points`)

- [ ] 테스트: `precise_checklist`가 공지·코드 구간만 인용한 요점을 빼는지, `uncovered_points`가 개념·예제·용어 중 어느 것이 요점 구간을 하나라도 인용하면 다룬 것으로 보는지.
- [ ] 구현과 `cargo test`.
- [ ] 1회 시험으로 목록·재질문이 동작하는지 확인하고 커밋한다.
