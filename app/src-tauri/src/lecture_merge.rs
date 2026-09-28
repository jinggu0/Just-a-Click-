//! Assembles what the separate calls return: a draft per window from its points, notices and
//! code, and the note from its body plus the notices and code of every draft.
//!
//! Merging only drops duplicates; it never rewrites an item. Every dropped item is recorded.
//!
//! The precise note is read one window at a time; its lists are merged here by name, and a
//! last call writes only the topic and the review over the merged concepts.
use std::collections::BTreeSet;

use serde::Serialize;

use crate::contract::Segment;
use crate::lecture::{
    collapse, english_source, Accepted, Code, Concept, Item, Notice, NoteBody, PreciseSynthesis, PreciseWindow, Repair,
    Term,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Window {
    pub first: String,
    pub last: String,
}

/// `lecture-draft-v2`: one window, assembled by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Draft {
    pub window: Window,
    pub points: Vec<Item>,
    pub notices: Vec<Notice>,
    pub code: Vec<Code>,
}

/// `lecture-note-v2`: the model's body plus the notices and code gathered from the drafts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub topic: Item,
    pub concepts: Vec<Concept>,
    pub examples: Vec<Item>,
    pub terms: Vec<Term>,
    pub notices: Vec<Notice>,
    pub code: Vec<Code>,
    pub review: Vec<Item>,
}

/// The precise note before its topic and review: every window's lists in order, with equal
/// names merged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PreciseBody {
    pub concepts: Vec<Concept>,
    pub examples: Vec<Item>,
    pub terms: Vec<Term>,
}

fn overlaps(left: &[String], right: &[String]) -> bool {
    left.iter().any(|id| right.contains(id))
}

/// Two notices are the same when kind and wording match, or when kind and date match and
/// they cite at least one segment in common.
fn same_notice(left: &Notice, right: &Notice) -> bool {
    left.kind == right.kind
        && (collapse(&left.content) == collapse(&right.content)
            || (left.date_text.is_some()
                && left.date_text == right.date_text
                && overlaps(&left.source_refs, &right.source_refs)))
}

/// Keeps the first of each group of equal notices.
pub fn merge_notices(notices: Vec<Notice>, path: &str) -> (Vec<Notice>, Vec<Repair>) {
    let mut kept: Vec<Notice> = Vec::new();
    let mut repairs = Vec::new();
    for (index, notice) in notices.into_iter().enumerate() {
        if let Some(first) = kept.iter().position(|existing| same_notice(existing, &notice)) {
            repairs.push(Repair {
                path: format!("{path}[{index}]"),
                kind: "notice_merged",
                detail: format!("same as kept notice {first}"),
            });
        } else {
            kept.push(notice);
        }
    }
    (kept, repairs)
}

/// Keeps the first of each piece of code that reads the same once spacing is ignored.
pub fn merge_code(code: Vec<Code>, path: &str) -> (Vec<Code>, Vec<Repair>) {
    let mut kept: Vec<Code> = Vec::new();
    let mut repairs = Vec::new();
    for (index, item) in code.into_iter().enumerate() {
        if let Some(first) = kept.iter().position(|existing| collapse(&existing.code) == collapse(&item.code)) {
            repairs.push(Repair {
                path: format!("{path}[{index}]"),
                kind: "code_merged",
                detail: format!("same as kept code {first}"),
            });
        } else {
            kept.push(item);
        }
    }
    (kept, repairs)
}

fn tagged(stage: &str, repairs: Vec<Repair>) -> impl Iterator<Item = Repair> + '_ {
    repairs.into_iter().map(move |repair| Repair {
        path: format!("{stage}:{}", repair.path),
        ..repair
    })
}

/// One window's draft. Repairs from each call keep a prefix naming the call.
pub fn assemble_draft(
    window: &[Segment],
    points: Accepted<Vec<Item>>,
    notices: Accepted<Vec<Notice>>,
    code: Accepted<Vec<Code>>,
) -> Accepted<Draft> {
    let (notices_kept, notice_repairs) = merge_notices(notices.value, "$.notices");
    let (code_kept, code_repairs) = merge_code(code.value, "$.code");
    let repairs = tagged("points", points.repairs)
        .chain(tagged("notices", notices.repairs))
        .chain(tagged("notices", notice_repairs))
        .chain(tagged("code", code.repairs))
        .chain(tagged("code", code_repairs))
        .collect();
    let window = Window {
        first: window.first().map(|segment| segment.id.clone()).unwrap_or_default(),
        last: window.last().map(|segment| segment.id.clone()).unwrap_or_default(),
    };
    Accepted {
        value: Draft { window, points: points.value, notices: notices_kept, code: code_kept },
        repairs,
    }
}

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

/// The draft points a precise window must cover. The precise note writes neither notices
/// nor code, so points citing only segments the draft's notices cite are left out, and so are
/// points that spell out one of the draft's commands. Code merely citing a point's segment is
/// not enough, and single words are not taken as commands: the code call sometimes turns
/// concept words into code ("kill" for a recovery segment, "wait" and "signal" for semaphores).
pub fn precise_checklist(draft: &Draft) -> Vec<Item> {
    let noticed: BTreeSet<&String> = draft.notices.iter().flat_map(|notice| &notice.source_refs).collect();
    let code: Vec<String> = draft
        .code
        .iter()
        .map(|item| collapse(&item.code).to_lowercase())
        .filter(|command| command.contains(' '))
        .collect();
    draft
        .points
        .iter()
        .filter(|point| {
            let only_notices = point.source_refs.iter().all(|id| noticed.contains(id));
            let text = collapse(&point.content).to_lowercase();
            let spells_code = code.iter().any(|item| !item.is_empty() && text.contains(item.as_str()));
            !only_notices && !spells_code
        })
        .cloned()
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_transcript;
    use crate::lecture::{Language, NoticeKind, NoticeStatus, PreciseSynthesis, PreciseWindow, TermSource};

    fn notice(kind: NoticeKind, content: &str, date: Option<&str>, refs: &[&str]) -> Notice {
        Notice {
            kind,
            content: content.to_string(),
            date_text: date.map(str::to_string),
            scope_text: None,
            status: NoticeStatus::Scheduled,
            source_refs: refs.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn code(text: &str, from_transcript: bool) -> Code {
        Code {
            code: text.to_string(),
            language: Language::Shell,
            explanation: "설명".into(),
            from_transcript,
            source_refs: vec!["s3".into()],
        }
    }

    fn accepted<T>(value: T) -> Accepted<T> {
        Accepted { value, repairs: Vec::new() }
    }

    fn kinds(repairs: &[Repair]) -> Vec<&'static str> {
        repairs.iter().map(|repair| repair.kind).collect()
    }

    #[test]
    fn equal_notices_are_merged_and_different_ones_kept() {
        let (kept, repairs) = merge_notices(
            vec![
                notice(NoticeKind::Exam, "중간고사  안내", Some("10월 21일"), &["s9"]),
                notice(NoticeKind::Exam, "중간고사 안내", None, &["s2"]),
                notice(NoticeKind::Exam, "중간고사 공지", Some("10월 21일"), &["s9", "s10"]),
                notice(NoticeKind::Assignment, "중간고사 안내", None, &["s9"]),
                notice(NoticeKind::Exam, "기말고사", Some("12월 16일"), &["s30"]),
            ],
            "$.notices",
        );
        assert_eq!(kept.len(), 3);
        assert_eq!(kinds(&repairs), vec!["notice_merged", "notice_merged"]);
        assert_eq!(repairs[0].path, "$.notices[1]");
        assert_eq!(repairs[1].path, "$.notices[2]");
    }

    #[test]
    fn code_that_differs_only_in_spacing_is_merged() {
        let (kept, repairs) = merge_code(
            vec![code("chmod 755 run.sh", true), code("chmod  755  run.sh", true), code("ps aux", false)],
            "$.code",
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kinds(&repairs), vec!["code_merged"]);
    }

    #[test]
    fn a_draft_takes_its_window_and_keeps_each_call_repairs_apart() {
        let window = parse_transcript("[s25 10:00] 실습\n[s26 10:25] 컴파일").expect("window");
        let points = Accepted {
            value: vec![Item { content: "실습 준비".into(), source_refs: vec!["s25".into()] }],
            repairs: vec![Repair { path: "$.points[1]".into(), kind: "repeat_removed", detail: String::new() }],
        };
        let draft = assemble_draft(
            &window,
            points,
            accepted(vec![notice(NoticeKind::Exam, "시험", None, &["s25"]), notice(NoticeKind::Exam, "시험", None, &["s26"])]),
            accepted(vec![code("gcc -o banker banker.c", true)]),
        );
        assert_eq!(draft.value.window, Window { first: "s25".into(), last: "s26".into() });
        assert_eq!(draft.value.notices.len(), 1);
        let paths: Vec<&str> = draft.repairs.iter().map(|repair| repair.path.as_str()).collect();
        assert_eq!(paths, vec!["points:$.points[1]", "notices:$.notices[1]"]);
    }

    #[test]
    fn a_note_gathers_notices_and_code_from_every_draft() {
        let first = Draft {
            window: Window { first: "s1".into(), last: "s12".into() },
            points: vec![],
            notices: vec![notice(NoticeKind::Exam, "중간고사", Some("10월 21일"), &["s9"])],
            code: vec![],
        };
        let second = Draft {
            window: Window { first: "s13".into(), last: "s24".into() },
            points: vec![],
            notices: vec![
                notice(NoticeKind::Exam, "중간고사", Some("10월 21일"), &["s9"]),
                notice(NoticeKind::Assignment, "은행원 알고리즘 구현", None, &["s22"]),
            ],
            code: vec![code("chmod 755 run.sh", true)],
        };
        let body = NoteBody {
            schema_version: "lecture-note-body-v1".into(),
            topic: Item { content: "교착 상태".into(), source_refs: vec!["s1".into()] },
            concepts: vec![],
            examples: vec![],
            terms: vec![],
            review: vec![],
        };
        let note = assemble_note(accepted(body), &[first, second]);
        assert_eq!(note.value.notices.len(), 2);
        assert_eq!(note.value.code.len(), 1);
        assert_eq!(kinds(&note.repairs), vec!["notice_merged"]);
        assert_eq!(note.repairs[0].path, "note:$.notices[1]");
    }

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
    fn the_checklist_leaves_out_points_that_only_cite_notices_or_code() {
        let draft = Draft {
            window: Window { first: "s21".into(), last: "s27".into() },
            points: vec![
                item("과제 안내", &["s22"]),
                item("컴파일은 gcc -o banker banker.c 로 한다", &["s25", "s26"]),
                item("은행원 알고리즘 과제와 구현", &["s22", "s23"]),
                item("프로세스 확인", &["s27"]),
            ],
            notices: vec![notice(NoticeKind::Assignment, "과제", None, &["s22"])],
            code: vec![Code { source_refs: refs(&["s25", "s26"]), ..code("gcc -o banker banker.c", true) }],
        };
        let contents: Vec<String> = precise_checklist(&draft).into_iter().map(|point| point.content).collect();
        assert_eq!(contents, vec!["은행원 알고리즘 과제와 구현", "프로세스 확인"]);
    }

    #[test]
    fn a_point_stays_on_the_checklist_when_code_merely_cites_its_segment() {
        // The code call once wrote "kill" for the recovery segment; the recovery point must stay.
        let draft = Draft {
            window: Window { first: "s25".into(), last: "s31".into() },
            points: vec![
                item("실습 명령어는 chmod  755 run.sh 이다", &["s25"]),
                item("교착 상태 회복은 프로세스 종료나 자원 회수로 한다", &["s31"]),
                item("세마포어는 wait, signal 두 연산을 쓴다", &["s18"]),
            ],
            notices: vec![],
            code: vec![
                Code { source_refs: refs(&["s25"]), ..code("chmod 755 run.sh", true) },
                Code { source_refs: refs(&["s31"]), ..code("kill", false) },
                Code { source_refs: refs(&["s18"]), ..code("wait", true) },
            ],
        };
        let contents: Vec<String> = precise_checklist(&draft).into_iter().map(|point| point.content).collect();
        assert_eq!(
            contents,
            vec!["교착 상태 회복은 프로세스 종료나 자원 회수로 한다", "세마포어는 wait, signal 두 연산을 쓴다"]
        );
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
        assert_eq!(synthesis_input(&merged.value), "- 교착 상태 [s1]");
        let cited: Vec<String> = cited_by(&merged.value, &lecture()).into_iter().map(|segment| segment.id).collect();
        assert_eq!(cited, refs(&["s1", "s2"]));
    }

    #[test]
    fn the_synthesis_input_fits_when_prompt_output_and_template_fit_the_context() {
        assert!(fits_context(6_080, 2_048, 8_192));
        assert!(!fits_context(6_081, 2_048, 8_192));
        let body = PreciseBody {
            concepts: vec![
                concept("교착 상태", "서로 기다리며 멈춘 상태", &["s1", "s13"]),
                concept("세마포어", "정수와 두 연산", &["s18"]),
            ],
            examples: vec![],
            terms: vec![],
        };
        assert_eq!(synthesis_input(&body), "- 교착 상태 [s1, s13]\n- 세마포어 [s18]");
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
}
