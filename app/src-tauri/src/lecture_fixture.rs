//! The synthetic lecture in `evaluation/fixtures/lecture-synthetic-v1.json` and the checks
//! for the traps written into it.
//!
//! These checks are regressions for this one fixture. They are not a quality score for any
//! other lecture.
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contract::{parse_transcript, Segment};
use crate::lecture::{Code, Draft, Note, Notice, NoticeKind, Term, TermSource};

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

/// A number followed by a score unit, or a word for a score, which the lecture never gives.
fn mentions_points(text: &str) -> bool {
    if text.contains("배점") || text.contains("만점") || text.contains("퍼센트") {
        return true;
    }
    let characters: Vec<char> = text.chars().collect();
    characters.windows(2).any(|pair| {
        pair[0].is_ascii_digit() && (pair[1] == '%' || pair[1] == '점')
    }) || characters.windows(3).any(|triple| {
        triple[0].is_ascii_digit() && triple[1] == ' ' && (triple[2] == '%' || triple[2] == '점')
    })
}

/// Every source list in the answer, so chatter can be looked for in all of them.
fn all_refs<'a>(lists: impl Iterator<Item = &'a Vec<String>>) -> Vec<&'a String> {
    lists.flatten().collect()
}

/// The notices that cite a given segment.
fn about<'a>(notices: &'a [Notice], id: &'a str) -> impl Iterator<Item = &'a Notice> + 'a {
    notices.iter().filter(move |notice| cites(&notice.source_refs, id))
}

fn notice_checks(notices: &[Notice], traps: &Traps, window: Option<&[Segment]>) -> Vec<Expectation> {
    let covers = |id: &str| window.map(|segments| segments.iter().any(|segment| segment.id == id)).unwrap_or(true);
    let about = |id| about(notices, id);
    let exam = covers(&traps.exam).then(|| {
        about(&traps.exam).any(|notice| {
            notice.kind == NoticeKind::Exam && notice.date_text.is_some() && notice.scope_text.is_some()
        })
    });
    let vague = covers(&traps.assignment).then(|| about(&traps.assignment).all(|notice| notice.date_text.is_none()));
    let quiz = covers(&traps.cancelled_quiz)
        .then(|| about(&traps.cancelled_quiz).all(|notice| notice.content.contains("취소")));
    let points = Some(!notices.iter().any(|notice| {
        mentions_points(&notice.content)
            || notice.date_text.as_deref().is_some_and(mentions_points)
            || notice.scope_text.as_deref().is_some_and(mentions_points)
    }));
    let captured = covers(&traps.assignment).then(|| {
        about(&traps.assignment).any(|notice| notice.kind == NoticeKind::Assignment)
    });
    vec![
        gate("exam_date_and_scope_kept", exam),
        gate("vague_deadline_left_null", vague),
        gate("cancelled_quiz_not_announced", quiz),
        gate("no_invented_points", points),
        reference("assignment_captured", captured),
    ]
}

fn code_checks(code: &[Code], traps: &Traps, window: Option<&[Segment]>) -> Vec<Expectation> {
    let covered = window
        .map(|segments| segments.iter().any(|segment| segment.id == traps.latin_command.segment))
        .unwrap_or(true);
    let latin = covered.then(|| {
        code.iter().any(|item| {
            item.from_transcript
                && item.code.split_whitespace().collect::<Vec<_>>().join(" ") == traps.latin_command.code
        })
    });
    let phonetic: Vec<&Code> = code
        .iter()
        .filter(|item| cites(&item.source_refs, &traps.phonetic_command))
        .collect();
    vec![
        reference("latin_command_verbatim", latin),
        reference(
            "phonetic_command_flagged",
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

fn chatter_check(refs: &[&String], traps: &Traps) -> Expectation {
    gate(
        "chatter_excluded",
        Some(!refs.iter().any(|id| traps.chatter.iter().any(|chatter| chatter == *id))),
    )
}

pub fn check_draft(draft: &Draft, traps: &Traps, window: &[Segment]) -> Vec<Expectation> {
    let refs = all_refs(
        draft
            .points
            .iter()
            .chain(&draft.examples)
            .map(|item| &item.source_refs)
            .chain(draft.concepts.iter().map(|concept| &concept.source_refs))
            .chain(draft.code.iter().map(|item| &item.source_refs))
            .chain(draft.notices.iter().map(|notice| &notice.source_refs)),
    );
    let mut checks = vec![chatter_check(&refs, traps)];
    checks.extend(notice_checks(&draft.notices, traps, Some(window)));
    checks.extend(code_checks(&draft.code, traps, Some(window)));
    checks
}

pub fn check_note(note: &Note, traps: &Traps) -> Vec<Expectation> {
    let refs = all_refs(
        std::iter::once(&note.topic.source_refs)
            .chain(note.examples.iter().chain(&note.review).map(|item| &item.source_refs))
            .chain(note.concepts.iter().map(|concept| &concept.source_refs))
            .chain(note.terms.iter().map(|term| &term.source_refs))
            .chain(note.code.iter().map(|item| &item.source_refs))
            .chain(note.notices.iter().map(|notice| &notice.source_refs)),
    );
    let mut checks = vec![chatter_check(&refs, traps)];
    checks.extend(notice_checks(&note.notices, traps, None));
    checks.extend(code_checks(&note.code, traps, None));
    checks.extend(term_checks(&note.terms, traps));
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lecture::{Concept, Item, Language};

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

    fn notice(kind: NoticeKind, content: &str, date: Option<&str>, scope: Option<&str>, refs: &[&str]) -> Notice {
        Notice {
            kind,
            content: content.to_string(),
            date_text: date.map(str::to_string),
            scope_text: scope.map(str::to_string),
            source_refs: refs.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn good_note() -> Note {
        Note {
            schema_version: "lecture-note-v1".into(),
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
                notice(NoticeKind::Exam, "중간고사", Some("10월 21일 화요일 오전 10시"), Some("3장부터 5장까지"), &["s9"]),
                notice(NoticeKind::Assignment, "은행원 알고리즘 구현", None, None, &["s22", "s23"]),
                notice(NoticeKind::Announcement, "퀴즈는 취소되었다", None, None, &["s21"]),
            ],
            code: vec![
                Code {
                    code: "chmod 755 run.sh".into(),
                    language: Language::Shell,
                    explanation: "실행 권한".into(),
                    from_transcript: true,
                    source_refs: vec!["s25".into()],
                },
                Code {
                    code: "ps aux | grep banker".into(),
                    language: Language::Shell,
                    explanation: "프로세스 찾기".into(),
                    from_transcript: false,
                    source_refs: vec!["s27".into()],
                },
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
    fn the_fixture_has_three_windows_of_twelve_segments() {
        let fixture = fixture();
        assert_eq!(fixture.windows.len(), 3);
        for index in 0..3 {
            assert_eq!(fixture.window(index).expect("window").len(), 12);
        }
        assert_eq!(fixture.all_segments().expect("segments").len(), 36);
        let all = fixture.all_segments().expect("segments");
        for id in [&fixture.traps.exam, &fixture.traps.assignment, &fixture.traps.cancelled_quiz] {
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
        note.notices[2] = notice(NoticeKind::Exam, "다음 주 수요일 퀴즈", Some("다음 주 수요일"), None, &["s21"]);
        note.notices.push(notice(NoticeKind::Assignment, "과제 배점 20점", None, None, &["s22"]));
        note.review.push(item("점심 메뉴", &["s4"]));
        note.code[1].from_transcript = true;
        note.terms[1].term_en_source = Some(TermSource::Transcript);
        assert_eq!(
            failed(&check_note(&note, &fixture.traps)),
            vec![
                "chatter_excluded",
                "exam_date_and_scope_kept",
                "vague_deadline_left_null",
                "cancelled_quiz_not_announced",
                "no_invented_points",
                "phonetic_command_flagged",
                "unspoken_english_marked_model",
            ]
        );
    }

    #[test]
    fn draft_checks_skip_traps_outside_the_window() {
        let fixture = fixture();
        let window = fixture.window(2).expect("window");
        let draft = Draft {
            schema_version: "lecture-draft-v1".into(),
            window: crate::lecture::Window { first: "s25".into(), last: "s36".into() },
            points: vec![item("실습 준비", &["s25"])],
            concepts: vec![],
            examples: vec![],
            code: vec![],
            notices: vec![],
        };
        let checks = check_draft(&draft, &fixture.traps, &window);
        let exam = checks.iter().find(|check| check.name == "exam_date_and_scope_kept").unwrap();
        assert_eq!(exam.passed, None, "the exam is announced in the first window");
        assert!(failed(&checks).contains(&"latin_command_verbatim"));
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
