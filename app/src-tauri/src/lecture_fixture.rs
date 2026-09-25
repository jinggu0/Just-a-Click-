//! The synthetic lecture in `evaluation/fixtures/lecture-synthetic-v1.json` and the checks
//! for the traps written into it.
//!
//! These checks are regressions for this one fixture. They are not a quality score for any
//! other lecture.
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contract::{parse_transcript, Segment};
use crate::lecture::{collapse, Code, Notice, NoticeKind, NoticeStatus, Term, TermSource};
use crate::lecture_merge::{Draft, Note};

#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub fixture_id: String,
    pub course: String,
    pub windows: Vec<Vec<String>>,
    pub traps: Traps,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Traps {
    pub exam: String,
    pub assignment: String,
    pub cancelled_quiz: String,
    pub chatter: Vec<String>,
    pub latin_command: CodeTrap,
    pub phonetic_command: String,
    pub spoken_english_term: SpokenTerm,
    pub unspoken_english_term: UnspokenTerm,
    pub not_a_notice: Vec<String>,
    pub second_latin_command: CodeTrap,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodeTrap {
    pub segment: String,
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpokenTerm {
    pub segment: String,
    pub term_en: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnspokenTerm {
    pub segment: String,
    pub term_ko: String,
}

impl Fixture {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| format!("fixture unreadable: {error}"))?;
        serde_json::from_str(&text).map_err(|error| format!("fixture is malformed: {error}"))
    }

    pub fn window(&self, index: usize) -> Result<Vec<Segment>, String> {
        let lines = self.windows.get(index).ok_or("no such window")?;
        parse_transcript(&lines.join("\n"))
    }

    pub fn all_segments(&self) -> Result<Vec<Segment>, String> {
        parse_transcript(&self.windows.concat().join("\n"))
    }
}

/// One expectation. `gating` ones decide adoption; the rest are recorded as reference.
/// `passed` is None when the answer has nothing the expectation could apply to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Expectation {
    pub name: &'static str,
    pub gating: bool,
    pub passed: Option<bool>,
}

fn gate(name: &'static str, passed: Option<bool>) -> Expectation {
    Expectation { name, gating: true, passed }
}

fn reference(name: &'static str, passed: Option<bool>) -> Expectation {
    Expectation { name, gating: false, passed }
}

fn cites(refs: &[String], id: &str) -> bool {
    refs.iter().any(|reference| reference == id)
}

/// Whether the window (or, for a note, the whole lecture) contains a segment.
fn covers(window: Option<&[Segment]>, id: &str) -> bool {
    window.map(|segments| segments.iter().any(|segment| segment.id == id)).unwrap_or(true)
}

/// A number followed by a score unit, or a word for a score, which the lecture never gives.
fn mentions_points(text: &str) -> bool {
    if text.contains("배점") || text.contains("만점") || text.contains("퍼센트") {
        return true;
    }
    let characters: Vec<char> = text.chars().filter(|character| *character != ' ').collect();
    characters
        .windows(2)
        .any(|pair| pair[0].is_ascii_digit() && (pair[1] == '%' || pair[1] == '점'))
}

/// The notices that cite a given segment.
fn about<'a>(notices: &'a [Notice], id: &'a str) -> impl Iterator<Item = &'a Notice> + 'a {
    notices.iter().filter(move |notice| cites(&notice.source_refs, id))
}

fn notice_checks(notices: &[Notice], traps: &Traps, window: Option<&[Segment]>) -> Vec<Expectation> {
    let exam = covers(window, &traps.exam).then(|| {
        about(notices, &traps.exam).any(|notice| {
            notice.kind == NoticeKind::Exam && notice.date_text.is_some() && notice.scope_text.is_some()
        })
    });
    let vague = covers(window, &traps.assignment)
        .then(|| about(notices, &traps.assignment).all(|notice| notice.date_text.is_none()));
    let quiz = covers(window, &traps.cancelled_quiz)
        .then(|| about(notices, &traps.cancelled_quiz).all(|notice| notice.status == NoticeStatus::Cancelled));
    let breaks = traps.not_a_notice.iter().any(|id| covers(window, id)).then(|| {
        !traps.not_a_notice.iter().any(|id| about(notices, id).next().is_some())
    });
    let points = Some(!notices.iter().any(|notice| {
        mentions_points(&notice.content)
            || notice.date_text.as_deref().is_some_and(mentions_points)
            || notice.scope_text.as_deref().is_some_and(mentions_points)
    }));
    let captured = covers(window, &traps.assignment)
        .then(|| about(notices, &traps.assignment).any(|notice| notice.kind == NoticeKind::Assignment));
    vec![
        gate("exam_date_and_scope_kept", exam),
        gate("vague_deadline_left_null", vague),
        gate("cancelled_quiz_marked_cancelled", quiz),
        gate("no_notice_from_class_flow", breaks),
        gate("no_invented_points", points),
        reference("assignment_captured", captured),
    ]
}

fn has_code(code: &[Code], wanted: &str) -> bool {
    code.iter().any(|item| item.from_transcript && collapse(&item.code) == collapse(wanted))
}

/// Capturing the verbatim command decides adoption for a note; in a single draft it is only
/// recorded, because a draft is not what the student reads.
fn code_checks(code: &[Code], traps: &Traps, window: Option<&[Segment]>, decides: bool) -> Vec<Expectation> {
    let latin = covers(window, &traps.latin_command.segment).then(|| has_code(code, &traps.latin_command.code));
    let second = covers(window, &traps.second_latin_command.segment)
        .then(|| has_code(code, &traps.second_latin_command.code));
    let phonetic: Vec<&Code> = code
        .iter()
        .filter(|item| cites(&item.source_refs, &traps.phonetic_command))
        .collect();
    vec![
        Expectation { name: "latin_command_captured", gating: decides, passed: latin },
        reference("second_latin_command_captured", second),
        reference(
            "phonetic_command_marked_restored",
            (!phonetic.is_empty()).then(|| phonetic.iter().all(|item| !item.from_transcript)),
        ),
    ]
}

fn term_checks(terms: &[Term], traps: &Traps) -> Vec<Expectation> {
    let spoken: Vec<&Term> = terms
        .iter()
        .filter(|term| {
            term.term_en
                .as_deref()
                .is_some_and(|english| english.eq_ignore_ascii_case(&traps.spoken_english_term.term_en))
        })
        .collect();
    let unspoken: Vec<&Term> = terms
        .iter()
        .filter(|term| term.term_ko.contains(&traps.unspoken_english_term.term_ko) && term.term_en.is_some())
        .collect();
    vec![
        reference(
            "spoken_english_marked_transcript",
            (!spoken.is_empty()).then(|| spoken.iter().all(|term| term.term_en_source == Some(TermSource::Transcript))),
        ),
        reference(
            "unspoken_english_marked_model",
            (!unspoken.is_empty()).then(|| unspoken.iter().all(|term| term.term_en_source == Some(TermSource::Model))),
        ),
    ]
}

fn chatter_check<'a>(mut refs: impl Iterator<Item = &'a String>, traps: &Traps) -> Expectation {
    gate(
        "chatter_excluded",
        Some(!refs.any(|id| traps.chatter.iter().any(|chatter| chatter == id))),
    )
}

pub fn check_draft(draft: &Draft, traps: &Traps, window: &[Segment]) -> Vec<Expectation> {
    let refs = draft
        .points
        .iter()
        .flat_map(|item| &item.source_refs)
        .chain(draft.notices.iter().flat_map(|notice| &notice.source_refs))
        .chain(draft.code.iter().flat_map(|item| &item.source_refs));
    let mut checks = vec![chatter_check(refs, traps)];
    checks.extend(notice_checks(&draft.notices, traps, Some(window)));
    checks.extend(code_checks(&draft.code, traps, Some(window), false));
    checks
}

pub fn check_note(note: &Note, traps: &Traps) -> Vec<Expectation> {
    let refs = note
        .topic
        .source_refs
        .iter()
        .chain(note.concepts.iter().flat_map(|concept| &concept.source_refs))
        .chain(note.examples.iter().chain(&note.review).flat_map(|item| &item.source_refs))
        .chain(note.terms.iter().flat_map(|term| &term.source_refs))
        .chain(note.notices.iter().flat_map(|notice| &notice.source_refs))
        .chain(note.code.iter().flat_map(|item| &item.source_refs));
    let mut checks = vec![chatter_check(refs, traps)];
    checks.extend(notice_checks(&note.notices, traps, None));
    checks.extend(code_checks(&note.code, traps, None, true));
    checks.extend(term_checks(&note.terms, traps));
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lecture::{Concept, Item, Language};
    use crate::lecture_merge::Window;

    fn fixture() -> Fixture {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evaluation/fixtures/lecture-synthetic-v1.json");
        Fixture::load(&path).expect("fixture")
    }

    fn item(content: &str, refs: &[&str]) -> Item {
        Item {
            content: content.to_string(),
            source_refs: refs.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn notice(kind: NoticeKind, content: &str, date: Option<&str>, scope: Option<&str>, cancelled: bool, refs: &[&str]) -> Notice {
        let status = if cancelled { NoticeStatus::Cancelled } else { NoticeStatus::Scheduled };
        Notice {
            kind,
            content: content.to_string(),
            date_text: date.map(str::to_string),
            scope_text: scope.map(str::to_string),
            status,
            source_refs: refs.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn code(text: &str, from_transcript: bool, refs: &[&str]) -> Code {
        Code {
            code: text.into(),
            language: Language::Shell,
            explanation: "설명".into(),
            from_transcript,
            source_refs: refs.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn good_note() -> Note {
        Note {
            topic: item("교착 상태", &["s1"]),
            concepts: vec![Concept {
                name: "교착 상태".into(),
                explanation: "서로의 자원을 기다리며 멈춘 상태".into(),
                source_refs: vec!["s2".into()],
            }],
            examples: vec![item("은행원 알고리즘 예제", &["s16", "s17"])],
            terms: vec![
                Term {
                    term_ko: "교착 상태".into(),
                    definition: "멈춘 상태".into(),
                    term_en: Some("Deadlock".into()),
                    term_en_source: Some(TermSource::Transcript),
                    source_refs: vec!["s1".into()],
                },
                Term {
                    term_ko: "세마포어".into(),
                    definition: "진입 수를 제한하는 도구".into(),
                    term_en: Some("semaphore".into()),
                    term_en_source: Some(TermSource::Model),
                    source_refs: vec!["s18".into()],
                },
            ],
            notices: vec![
                notice(NoticeKind::Exam, "중간고사", Some("10월 21일 화요일 오전 10시"), Some("3장부터 5장까지"), false, &["s9"]),
                notice(NoticeKind::Assignment, "은행원 알고리즘 구현", None, None, false, &["s22", "s23"]),
                notice(NoticeKind::Exam, "퀴즈 취소", None, None, true, &["s21"]),
            ],
            code: vec![
                code("chmod 755 run.sh", true, &["s25"]),
                code("gcc -o banker banker.c", true, &["s26"]),
                code("ps aux | grep banker", false, &["s27"]),
            ],
            review: vec![item("교착 상태의 네 조건", &["s33"])],
        }
    }

    fn failed(checks: &[Expectation]) -> Vec<&'static str> {
        checks
            .iter()
            .filter(|check| check.passed == Some(false))
            .map(|check| check.name)
            .collect()
    }

    #[test]
    fn the_fixture_has_three_windows_and_every_trap_points_at_a_segment() {
        let fixture = fixture();
        assert_eq!(fixture.windows.len(), 3);
        for index in 0..3 {
            assert_eq!(fixture.window(index).expect("window").len(), 12);
        }
        let all = fixture.all_segments().expect("segments");
        assert_eq!(all.len(), 36);
        let traps = &fixture.traps;
        let mut ids = vec![&traps.exam, &traps.assignment, &traps.cancelled_quiz, &traps.phonetic_command];
        ids.extend(traps.chatter.iter().chain(&traps.not_a_notice));
        ids.push(&traps.latin_command.segment);
        ids.push(&traps.second_latin_command.segment);
        for id in ids {
            assert!(all.iter().any(|segment| &segment.id == id), "{id} missing");
        }
    }

    #[test]
    fn a_note_that_handles_every_trap_passes_every_check() {
        let fixture = fixture();
        let checks = check_note(&good_note(), &fixture.traps);
        assert!(failed(&checks).is_empty(), "{checks:?}");
        assert!(checks.iter().filter(|check| check.gating).all(|check| check.passed == Some(true)));
    }

    #[test]
    fn a_note_that_falls_for_the_traps_fails_the_matching_checks() {
        let fixture = fixture();
        let mut note = good_note();
        note.notices[0].date_text = None;
        note.notices[1].date_text = Some("다음 주쯤".into());
        note.notices[2].status = NoticeStatus::Scheduled;
        note.notices.push(notice(NoticeKind::Assignment, "과제 배점 20점", None, None, false, &["s22"]));
        note.notices.push(notice(NoticeKind::Assignment, "실습실로 이동", None, None, false, &["s24"]));
        note.review.push(item("점심 메뉴", &["s4"]));
        note.code.remove(0);
        note.code[1].from_transcript = true;
        note.terms[1].term_en_source = Some(TermSource::Transcript);
        assert_eq!(
            failed(&check_note(&note, &fixture.traps)),
            vec![
                "chatter_excluded",
                "exam_date_and_scope_kept",
                "vague_deadline_left_null",
                "cancelled_quiz_marked_cancelled",
                "no_notice_from_class_flow",
                "no_invented_points",
                "latin_command_captured",
                "phonetic_command_marked_restored",
                "unspoken_english_marked_model",
            ]
        );
    }

    #[test]
    fn draft_checks_skip_traps_outside_the_window_and_only_record_code() {
        let fixture = fixture();
        let window = fixture.window(2).expect("window");
        let draft = Draft {
            window: Window { first: "s25".into(), last: "s36".into() },
            points: vec![item("실습 준비", &["s25"])],
            notices: vec![],
            code: vec![],
        };
        let checks = check_draft(&draft, &fixture.traps, &window);
        let find = |name: &str| checks.iter().find(|check| check.name == name).unwrap().clone();
        assert_eq!(find("exam_date_and_scope_kept").passed, None, "the exam is in the first window");
        assert_eq!(find("latin_command_captured").passed, Some(false));
        assert!(!find("latin_command_captured").gating, "a draft only records code capture");
    }

    #[test]
    fn score_mentions_are_recognised() {
        assert!(mentions_points("과제 배점은 높다"));
        assert!(mentions_points("20점 만점"));
        assert!(mentions_points("30 %"));
        assert!(!mentions_points("10월 21일 화요일 오전 10시"));
        assert!(!mentions_points("3장부터 5장까지"));
    }
}
