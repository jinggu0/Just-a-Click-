//! Assembles what the separate calls return: a draft per window from its points, notices and
//! code, and the note from its body plus the notices and code of every draft.
//!
//! Merging only drops duplicates; it never rewrites an item. Every dropped item is recorded.
use serde::Serialize;

use crate::contract::Segment;
use crate::lecture::{collapse, Accepted, Code, Concept, Item, Notice, NoteBody, Repair, Term};

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

/// The note: the body as written, then every draft's notices and code with duplicates
/// across windows dropped.
pub fn assemble_note(body: Accepted<NoteBody>, drafts: &[Draft]) -> Accepted<Note> {
    let notices: Vec<Notice> = drafts.iter().flat_map(|draft| draft.notices.clone()).collect();
    let code: Vec<Code> = drafts.iter().flat_map(|draft| draft.code.clone()).collect();
    let (notices, notice_repairs) = merge_notices(notices, "$.notices");
    let (code, code_repairs) = merge_code(code, "$.code");
    let repairs = tagged("body", body.repairs)
        .chain(tagged("note", notice_repairs))
        .chain(tagged("note", code_repairs))
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_transcript;
    use crate::lecture::{Language, NoticeKind, NoticeStatus};

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
}
