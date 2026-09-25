//! Pieces shared by the summary contracts: the transcript format, the schema the server
//! constrains generation with, and the rule checks.
//!
//! Passing these checks proves structure, sources and verbatim values. It never proves that
//! a summary means what its sources say; that is judged separately.
use std::collections::{BTreeMap, HashSet};

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde::Serialize;

pub const POINTS_SCHEMA: &str = include_str!("../../../schemas/lecture-points-v1.json");
pub const NOTICES_SCHEMA: &str = include_str!("../../../schemas/lecture-notices-v1.json");
pub const CODE_SCHEMA: &str = include_str!("../../../schemas/lecture-code-v1.json");
pub const NOTE_BODY_SCHEMA: &str = include_str!("../../../schemas/lecture-note-body-v1.json");

/// Every lecture schema the model is constrained with.
pub const LECTURE_SCHEMAS: [&str; 4] = [POINTS_SCHEMA, NOTICES_SCHEMA, CODE_SCHEMA, NOTE_BODY_SCHEMA];

/// Words that stand in for missing content. The screen shows those labels itself, so an
/// item that only says them is filler.
const PLACEHOLDERS: [&str; 7] = ["언급 없음", "없음", "미정", "확인 필요", "해당 없음", "N/A", "n/a"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub id: String,
    pub time: String,
    pub text: String,
}

impl Segment {
    pub fn line(&self) -> String {
        format!("[{} {}] {}", self.id, self.time, self.text)
    }
}

/// Reads `[s12 05:31] text` lines. Every line must match and every id must be unique.
pub fn parse_transcript(text: &str) -> Result<Vec<Segment>, String> {
    let mut segments = Vec::new();
    let mut seen = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let segment = parse_line(line.trim())
            .ok_or_else(|| format!("line {}: not a transcript segment", index + 1))?;
        if !seen.insert(segment.id.clone()) {
            return Err(format!("line {}: duplicate segment {}", index + 1, segment.id));
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        return Err("empty transcript".to_string());
    }
    Ok(segments)
}

fn parse_line(line: &str) -> Option<Segment> {
    let rest = line.strip_prefix('[')?;
    let (head, body) = rest.split_once(']')?;
    let (id, time) = head.split_once(' ')?;
    let digits = id.strip_prefix('s')?;
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    if time.is_empty() || !time.chars().all(|character| character.is_ascii_digit() || character == ':') {
        return None;
    }
    let text = body.trim();
    if text.is_empty() {
        return None;
    }
    Some(Segment {
        id: id.to_string(),
        time: time.to_string(),
        text: text.to_string(),
    })
}

/// The checked-in schema with the given segment ids as the only allowed sources.
pub fn generation_schema(schema: &str, ids: &[String]) -> Result<serde_json::Value, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(schema).map_err(|error| format!("schema is not JSON: {error}"))?;
    let segment = value
        .pointer_mut("/$defs/segment")
        .ok_or("schema has no $defs/segment")?;
    segment["enum"] = serde_json::json!(ids);
    Ok(value)
}

/// serde keeps the last of two equal keys; the contract refuses the answer instead.
pub fn reject_duplicate_keys(text: &str) -> Result<(), String> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    UniqueKeys
        .deserialize(&mut deserializer)
        .map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())
}

struct UniqueKeys;

impl<'de> DeserializeSeed<'de> for UniqueKeys {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueKeys {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate key: {key}")));
            }
            map.next_value_seed(UniqueKeys)?;
        }
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence.next_element_seed(UniqueKeys)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }
}

/// One broken rule, with the JSON path it was found at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub path: String,
    pub rule: &'static str,
    pub detail: String,
}

/// How a value is compared with the text it cites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// Character for character; used for dates and scopes.
    Exact,
    /// Runs of whitespace count as one space; used for code.
    Spacing,
    /// Letter case is ignored; used for English terms.
    Case,
}

/// Collects every violation instead of stopping at the first, so a failed answer shows
/// everything that has to change.
pub struct Checker<'a> {
    texts: BTreeMap<&'a str, &'a str>,
    pub violations: Vec<Violation>,
}

impl<'a> Checker<'a> {
    pub fn new(segments: &'a [Segment]) -> Self {
        Self {
            texts: segments
                .iter()
                .map(|segment| (segment.id.as_str(), segment.text.as_str()))
                .collect(),
            violations: Vec::new(),
        }
    }

    pub fn fail(&mut self, path: &str, rule: &'static str, detail: String) {
        self.violations.push(Violation {
            path: path.to_string(),
            rule,
            detail,
        });
    }

    /// Sources must exist, be present and not repeat.
    pub fn refs(&mut self, path: &str, refs: &[String]) {
        let path = format!("{path}.source_refs");
        if refs.is_empty() {
            self.fail(&path, "empty_sources", "an item needs at least one source".into());
        }
        let mut seen = HashSet::new();
        for id in refs {
            if !self.texts.contains_key(id.as_str()) {
                self.fail(&path, "unknown_source", format!("{id} is not in the transcript"));
            }
            if !seen.insert(id) {
                self.fail(&path, "duplicate_source", format!("{id} is listed twice"));
            }
        }
    }

    /// Text must say something and must not be a stand-in for missing content.
    pub fn text(&mut self, path: &str, value: &str) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            self.fail(path, "empty_text", "the text is empty".into());
        } else if PLACEHOLDERS.contains(&trimmed) {
            self.fail(path, "placeholder", format!("\"{trimmed}\" stands in for missing content"));
        }
    }

    /// The value must appear in the text of the segments the item cites.
    pub fn verbatim(&mut self, path: &str, value: &str, refs: &[String], mode: Match) {
        self.text(path, value);
        let cited: Vec<&str> = refs
            .iter()
            .filter_map(|id| self.texts.get(id.as_str()).copied())
            .collect();
        let cited = cited.join("\n");
        let found = match mode {
            Match::Exact => cited.contains(value),
            Match::Spacing => collapse(&cited).contains(&collapse(value)),
            Match::Case => cited.to_lowercase().contains(&value.to_lowercase()),
        };
        if !found {
            self.fail(path, "not_verbatim", format!("\"{value}\" is not in the cited segments"));
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments() -> Vec<Segment> {
        parse_transcript(
            "[s1 00:00] 중간고사는 10월 21일 화요일입니다.\n\
             [s2 00:20] chmod   755 run.sh 를 입력하세요.\n\
             [s3 00:40] 영어로는 Deadlock이라고 합니다.",
        )
        .expect("transcript")
    }

    #[test]
    fn transcript_lines_become_segments() {
        let parsed = segments();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].id, "s1");
        assert_eq!(parsed[0].time, "00:00");
        assert_eq!(parsed[0].text, "중간고사는 10월 21일 화요일입니다.");
        assert_eq!(parsed[0].line(), "[s1 00:00] 중간고사는 10월 21일 화요일입니다.");
    }

    #[test]
    fn malformed_duplicate_and_empty_transcripts_are_refused() {
        assert!(parse_transcript("s1 00:00 본문").is_err());
        assert!(parse_transcript("[x1 00:00] 본문").is_err());
        assert!(parse_transcript("[s1 00:00]   ").is_err());
        assert!(parse_transcript("[s1 00:00] 가\n[s1 00:10] 나").is_err());
        assert!(parse_transcript("").is_err());
    }

    #[test]
    fn the_generation_schema_allows_only_the_given_segments() {
        let ids = vec!["s1".to_string(), "s2".to_string()];
        for schema in LECTURE_SCHEMAS {
            let value = generation_schema(schema, &ids).expect("schema");
            assert_eq!(value["$defs"]["segment"]["enum"], serde_json::json!(["s1", "s2"]));
        }
    }

    #[test]
    fn duplicate_keys_are_found_at_any_depth() {
        assert!(reject_duplicate_keys(r#"{"a":1,"b":[{"c":2}]}"#).is_ok());
        assert!(reject_duplicate_keys(r#"{"a":1,"a":2}"#).is_err());
        assert!(reject_duplicate_keys(r#"{"a":[{"c":1,"c":2}]}"#).is_err());
        assert!(reject_duplicate_keys(r#"{"a":1} trailing"#).is_err());
    }

    #[test]
    fn sources_must_exist_be_present_and_not_repeat() {
        let parsed = segments();
        let mut checker = Checker::new(&parsed);
        checker.refs("$.ok", &["s1".into()]);
        assert!(checker.violations.is_empty());
        checker.refs("$.empty", &[]);
        checker.refs("$.unknown", &["s9".into()]);
        checker.refs("$.twice", &["s1".into(), "s1".into()]);
        let rules: Vec<&str> = checker.violations.iter().map(|violation| violation.rule).collect();
        assert_eq!(rules, vec!["empty_sources", "unknown_source", "duplicate_source"]);
    }

    #[test]
    fn empty_and_placeholder_text_is_refused() {
        let parsed = segments();
        let mut checker = Checker::new(&parsed);
        checker.text("$.a", "교착 상태의 정의");
        checker.text("$.b", "   ");
        checker.text("$.c", " 언급 없음 ");
        let rules: Vec<&str> = checker.violations.iter().map(|violation| violation.rule).collect();
        assert_eq!(rules, vec!["empty_text", "placeholder"]);
    }

    #[test]
    fn verbatim_values_are_compared_the_way_each_field_needs() {
        let parsed = segments();
        let mut checker = Checker::new(&parsed);
        checker.verbatim("$.date", "10월 21일 화요일", &["s1".into()], Match::Exact);
        checker.verbatim("$.code", "chmod 755 run.sh", &["s2".into()], Match::Spacing);
        checker.verbatim("$.term", "deadlock", &["s3".into()], Match::Case);
        assert!(checker.violations.is_empty(), "{:?}", checker.violations);
        checker.verbatim("$.date", "10월 22일", &["s1".into()], Match::Exact);
        checker.verbatim("$.date", "10월 21일 화요일", &["s2".into()], Match::Exact);
        checker.verbatim("$.code", "chmod 700 run.sh", &["s2".into()], Match::Spacing);
        assert_eq!(checker.violations.len(), 3);
        assert!(checker.violations.iter().all(|violation| violation.rule == "not_verbatim"));
    }
}
