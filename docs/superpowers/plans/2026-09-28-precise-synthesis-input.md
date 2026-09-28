# 정밀 정리 합성 입력 줄이기 실행 계획

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 합성 호출이 개념 이름과 출처만 받게 하고, 호출 전에 실제 토큰 수로 크기를 검사한다. 측정해 둔 정밀 정리 10건과 긴 강의 규모의 목록으로 합성만 다시 돌려 채택 여부를 정한다.

**Architecture:** `llm.rs`에 `/tokenize` 호출(`count_tokens`)을 더한다. `lecture_merge.rs`의 `synthesis_input`은 이름·출처만 쓰고, `fits_context`가 예산을 판단한다. 하네스는 예산을 넘는 합성을 호출하지 않고 기록한다. 새 예제 `precise_synthesis_check`가 저장된 노트로 합성만 재생한다.

**Tech Stack:** Rust 1.98.1, `reqwest` blocking·`serde_json`(기존), llama.cpp `b10994` Vulkan `llama-server`, Qwen3-8B Q5_K_M.

## Global Constraints

- 설계는 [합성 입력 설계](../specs/2026-09-28-precise-synthesis-input-design.md)를 따른다.
- 새 의존성을 넣지 않는다. 합성 스키마 `lecture-precise-synthesis-v1`은 바꾸지 않는다.
- 예산은 컨텍스트 8,192 − 최대 출력 2,048 − 템플릿 몫 64 = 6,080토큰이다. 넘으면 입력을 자르지 않고 `input_too_large`로 기록한다.
- 서버 설정은 v2 측정과 같다(컨텍스트 8,192, `-np 1`, `--cache-ram 0`, `-ngl 99`, 2스레드, `--jinja --reasoning off`, 온도 0.2).
- 측정은 AC 전원·"최고 성능"에서 하고, 전원 상태와 배경 부하를 기록한다. 전원 설정과 사용자 앱은 사용자가 바꾼다.
- 결과는 `artifacts/lecture-contract-v2/` 아래에만 두고 커밋하지 않는다.
- 코드 주석·식별자는 영어, 프롬프트·문서는 한국어. Bash에서 cargo 전에 `export PATH="$PATH:/c/Users/jingg/.cargo/bin"`.
- 작업마다 커밋하고 메시지 끝에 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`를 넣는다. push하지 않는다.

## 파일 구조

| 파일 | 책임 |
| --- | --- |
| `app/src-tauri/src/llm.rs` | `tokenize_body`, `read_token_count`, `count_tokens` |
| `app/src-tauri/src/lecture.rs` | 합성 프롬프트 v2 |
| `app/src-tauri/src/lecture_merge.rs` | 이름·출처 `synthesis_input`, `fits_context`, `TEMPLATE_TOKENS` |
| `app/src-tauri/examples/lecture_contract_check.rs` | 합성 전 크기 검사와 기록 |
| `app/src-tauri/examples/precise_synthesis_check.rs` | 저장된 노트로 합성 재생, 규모 시험, 크기 검사 시험 |

---

### Task 1: 토큰 세기

**Files:**
- Modify: `app/src-tauri/src/llm.rs`

**Interfaces:**
- Produces: `pub fn tokenize_body(text: &str) -> serde_json::Value`, `pub fn read_token_count(answer: &serde_json::Value) -> Result<usize, String>`, `pub fn count_tokens(base: &str, key: &str, text: &str) -> Result<usize, String>`

- [ ] **Step 1: 실패하는 테스트**

```rust
    #[test]
    fn a_tokenize_request_sends_the_text_without_special_tokens() {
        let body = tokenize_body("교착 상태");
        assert_eq!(body["content"], serde_json::json!("교착 상태"));
        assert_eq!(body["add_special"], serde_json::json!(false));
    }

    #[test]
    fn the_token_count_is_the_length_of_the_token_list() {
        assert_eq!(read_token_count(&serde_json::json!({"tokens": [11, 22, 33]})).expect("count"), 3);
        assert!(read_token_count(&serde_json::json!({"error": "nope"})).is_err());
    }
```

- [ ] **Step 2:** `cargo test --lib llm::tests` → `cannot find function tokenize_body`로 컴파일 FAIL.

- [ ] **Step 3: 구현** (`complete_json` 아래)

```rust
/// Asks the server to tokenize `text` alone; the chat template's own tokens are not counted.
pub fn tokenize_body(text: &str) -> serde_json::Value {
    serde_json::json!({"content": text, "add_special": false})
}

pub fn read_token_count(answer: &serde_json::Value) -> Result<usize, String> {
    answer["tokens"]
        .as_array()
        .map(Vec::len)
        .ok_or_else(|| "the answer has no token list".to_string())
}

/// How many tokens the loaded model's tokenizer makes of `text`.
pub fn count_tokens(base: &str, key: &str, text: &str) -> Result<usize, String> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(format!("{base}/tokenize"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .body(tokenize_body(text).to_string())
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status().as_u16();
    let text = response.text().map_err(|error| format!("answer unreadable: {error}"))?;
    if status != 200 {
        let head: String = text.chars().take(300).collect();
        return Err(format!("server answered {status}: {head}"));
    }
    let answer: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("answer is not JSON: {error}"))?;
    read_token_count(&answer)
}
```

- [ ] **Step 4:** `cargo test --lib` → PASS.
- [ ] **Step 5:** `git add app/src-tauri/src/llm.rs` 후 `git commit -m "feat: count tokens with the loaded model's tokenizer"`

---

### Task 2: 이름·출처 입력과 예산 판단

**Files:**
- Modify: `app/src-tauri/src/lecture_merge.rs`, `app/src-tauri/src/lecture.rs`

**Interfaces:**
- Produces: `pub const TEMPLATE_TOKENS: usize = 64`, `pub fn fits_context(prompt_tokens: usize, max_tokens: u32, context_tokens: u32) -> bool`, `synthesis_input`(형식 `- 이름 [s1, s13]`), `PRECISE_SYNTHESIS_PROMPT_VERSION = "lecture-precise-synthesis-prompt-v2"`

- [ ] **Step 1: 실패하는 테스트** — `window_repairs_keep_their_window_and_the_synthesis_reads_the_merged_concepts`의 기대값을 바꾸고 새 테스트를 더한다.

```rust
        assert_eq!(synthesis_input(&merged.value), "- 교착 상태 [s1]");
```

```rust
    #[test]
    fn the_synthesis_input_fits_when_prompt_output_and_template_fit_the_context() {
        assert!(fits_context(6_080, 2_048, 8_192));
        assert!(!fits_context(6_081, 2_048, 8_192));
        let body = PreciseBody {
            concepts: vec![concept("교착 상태", "서로 기다리며 멈춘 상태", &["s1", "s13"]), concept("세마포어", "정수와 두 연산", &["s18"])],
            examples: vec![],
            terms: vec![],
        };
        assert_eq!(synthesis_input(&body), "- 교착 상태 [s1, s13]\n- 세마포어 [s18]");
    }
```

`lecture.rs`의 `the_precise_prompts_ask_for_their_own_lists_only`에 더한다.

```rust
        assert!(precise_synthesis_prompt().contains("개념 이름과 인용 구간"));
        assert_eq!(PRECISE_SYNTHESIS_PROMPT_VERSION, "lecture-precise-synthesis-prompt-v2");
```

- [ ] **Step 2:** `cargo test --lib` → 컴파일 FAIL(`fits_context` 없음).

- [ ] **Step 3: 구현**

`lecture_merge.rs`:

```rust
/// Room the chat template takes around the system and user text.
pub const TEMPLATE_TOKENS: usize = 64;

/// What the synthesis call reads: one merged concept per line, its name and sources only.
/// Explanations would make a long lecture's list outgrow the context (about 53 tokens a
/// concept against 11).
pub fn synthesis_input(body: &PreciseBody) -> String {
    body.concepts
        .iter()
        .map(|concept| format!("- {} [{}]", concept.name, concept.source_refs.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a request of `prompt_tokens` can still produce `max_tokens` within the context.
pub fn fits_context(prompt_tokens: usize, max_tokens: u32, context_tokens: u32) -> bool {
    prompt_tokens + TEMPLATE_TOKENS + max_tokens as usize <= context_tokens as usize
}
```

`lecture.rs`: `PRECISE_SYNTHESIS_PROMPT_VERSION`을 `"lecture-precise-synthesis-prompt-v2"`로 바꾸고 프롬프트 첫 문장을 바꾼다.

```rust
        "너는 한국어 대학 강의 한 회차의 개념 이름과 인용 구간 목록을 읽고 주제와 복습 항목을 쓴다. 출력 형식은 lecture-precise-synthesis-v1이다.\n{COMMON_RULES}\n\
```

- [ ] **Step 4:** `cargo test --lib` → PASS.
- [ ] **Step 5:** `git add app/src-tauri/src/lecture.rs app/src-tauri/src/lecture_merge.rs` 후 `git commit -m "feat: give the synthesis call concept names and sources only"`

---

### Task 3: 하네스의 크기 검사

**Files:**
- Modify: `app/src-tauri/examples/lecture_contract_check.rs`

**Interfaces:**
- Consumes: `count_tokens`, `fits_context`
- Produces: 보고서 `skipped_syntheses: [{run, prompt_tokens, reason: "input_too_large"}]`, `summary.precise.syntheses_skipped`

- [ ] **Step 1:** 가져오기에 `count_tokens`, `fits_context`를 더하고 상수 `const CONTEXT_TOKENS: u32 = 8192;`를 둔다(서버 설정의 `context_tokens`도 이 상수를 쓴다).
- [ ] **Step 2:** 합성 호출 직전에 크기를 센다.

```rust
        let synthesis_user = format!("과목: {}\n\n개념 목록:\n{}", fixture.course, synthesis_input(&body.value));
        let prompt_tokens = count_tokens(&endpoint.base, &endpoint.key, &format!("{}\n{}", prompts[5].1, synthesis_user))?;
        if !fits_context(prompt_tokens, WINDOW_MAX_TOKENS, CONTEXT_TOKENS) {
            println!("run {run} note_precise skipped: synthesis input of {prompt_tokens} tokens does not fit");
            skipped.push(serde_json::json!({"run": run, "prompt_tokens": prompt_tokens, "reason": "input_too_large"}));
            continue;
        }
```

  합성 호출의 사용자 입력은 `synthesis_user`를 쓴다. `let mut skipped = Vec::new();`를 반복 전에 두고, 보고서에 `"skipped_syntheses": skipped`, `summarize`의 `precise`에 `"syntheses_skipped": skipped.len()`을 더한다(`summarize`에 `skipped: usize` 인자 추가).
- [ ] **Step 3:** `cargo build --release --example lecture_contract_check` 경고 없음, `cargo test` PASS.
- [ ] **Step 4:** `git commit -m "test: skip the synthesis call when its input does not fit the context"`

---

### Task 4: 합성 재생 예제

**Files:**
- Create: `app/src-tauri/examples/precise_synthesis_check.rs`

**Interfaces:**
- Consumes: `lecture-contract.json`(측정 1·2)의 `notes[kind == "note_precise"]`, `count_tokens`, `fits_context`, `synthesis_input`, `cited_by`, `validate_precise_synthesis`, `precise_synthesis_prompt`
- Produces: `<out>/precise-synthesis.json`(`replays`, `scale`, `size_check`), `<out>/raw/`, `<out>/server.log`

- [ ] **Step 1:** 예제를 쓴다. 인자는 `<root> <out dir> <contract json>...`이다. 흐름은 다음과 같다.
  1. 픽스처로 전 구간과 잡담 ID를 읽는다.
  2. 서버를 띄우고 예열한다(`lecture_contract_check`와 같은 설정·`KeepAwake`).
  3. 파일마다 `note_precise` 노트의 `concepts`·`examples`·`terms`로 `PreciseBody`를 되살린다. 용어는 `term_en_source` 키를 지우고 읽는다.
  4. **재생:** 노트마다 이름·출처 입력으로 합성을 한 번 호출한다(위반 시 1회 재시도). 입력 토큰, 채택 여부, 주제·복습, 기존 주제·복습, 잡담 인용 여부를 기록한다.
  5. **규모:** 모든 노트의 개념을 이어 붙인 목록과 그 두 배 목록으로 같은 호출을 하고, 입력 토큰·시간·채택 여부를 기록한다.
  6. **크기 검사:** 두 배 목록을 설명까지 넣은 기존 형식으로 만들어 토큰을 세고, `fits_context`가 거짓이면 호출하지 않고 `input_too_large`로 기록한다.
- [ ] **Step 2:** `cargo build --release --example precise_synthesis_check` 경고 없음.
- [ ] **Step 3:** `git commit -m "test: replay the precise synthesis on saved notes and at lecture scale"`

---

### Task 5: 재생 측정과 기록

- [ ] **Step 1:** 전원 상태와 배경 부하(전체 CPU, OneDrive)를 기록한다.
- [ ] **Step 2:** 실행한다.

```bash
app/src-tauri/target/release/examples/precise_synthesis_check.exe "C:\temp_git\Just-a-Click-" "C:\temp_git\Just-a-Click-\artifacts\lecture-contract-v2\synthesis" "C:\temp_git\Just-a-Click-\artifacts\lecture-contract-v2\precise-1\lecture-contract.json" "C:\temp_git\Just-a-Click-\artifacts\lecture-contract-v2\precise\lecture-contract.json"
```

- [ ] **Step 3:** 결과를 읽고 설계 4절 기준으로 판정한다. 기존·새 주제와 복습을 나란히 읽고 품질을 적는다.
- [ ] **Step 4:** 보고서 `docs/validation/2026-09-28-precise-synthesis-input.md`를 쓰고, 결정 0010 7절에 한 문단, README 목록에 링크를 더한다. `python artifacts/check_docs.py` → `broken links: none`.
- [ ] **Step 5:** 커밋한다. 메시지는 `docs: measure the names-only synthesis input`로 한다.
