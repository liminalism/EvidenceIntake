//! Deterministic, explainable candidates for the keyboard enrichment workspace.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{OptionalExtension, params};

use super::{Candidate, Store};
use crate::{
    CaseId, ContentForm, ContentInterpretation, EdgeKind, Error, InterpretationTarget, Materiality,
    NodeKind, ProposedInterpretation, ProposedSourceProfile, Result, ReviewState, SourceProfile,
    SourceRole, SuggestionKind,
};

/// Non-edge output from one enrichment rule.
pub(super) struct EnrichmentRuleResult {
    pub(super) interpretations: Vec<ContentInterpretation>,
    pub(super) profiles: Vec<SourceProfile>,
    pub(super) already_recorded: u32,
}

impl EnrichmentRuleResult {
    fn empty() -> Self {
        Self {
            interpretations: Vec::new(),
            profiles: Vec::new(),
            already_recorded: 0,
        }
    }
}

#[derive(Debug)]
struct Detection {
    char_start: Option<u32>,
    char_end: Option<u32>,
    content_form: Option<ContentForm>,
    asserted_start: Option<String>,
    field: &'static str,
    basis: String,
}

impl Store {
    /// Runs an enrichment rule whose output is an interpretation/profile rather
    /// than an edge. Rules never revise a previous decision: any history for
    /// the same target and span is enough to suppress the candidate forever.
    pub(super) fn enrichment_rule(
        &mut self,
        case_id: &CaseId,
        kind: SuggestionKind,
    ) -> Result<EnrichmentRuleResult> {
        match kind {
            SuggestionKind::InheritProfile => self.inherited_profile_count(case_id),
            SuggestionKind::HeaderDate => self.header_date_candidates(case_id),
            SuggestionKind::ReportedStatement
            | SuggestionKind::QuotedStatement
            | SuggestionKind::MeasuredResult
            | SuggestionKind::OfficialCharacterization
            | SuggestionKind::EvidenceReference
            | SuggestionKind::AssertedClock => self.interpretation_candidates(case_id, kind),
            _ => Err(Error::InvalidInterpretation(format!(
                "`{}` is not an interpretation or source-profile rule",
                kind.as_str()
            ))),
        }
    }

    fn inherited_profile_count(&self, case_id: &CaseId) -> Result<EnrichmentRuleResult> {
        let inherited: u32 = self.connection.query_row(
            "SELECT count(*)
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN current_source_profiles profile ON profile.source_id = seg.source_id
             WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
               AND profile.review_state IN ('reviewed','verified')
               AND (profile.author_entity_id IS NOT NULL
                    OR profile.created_at_claim IS NOT NULL
                    OR profile.default_content_form IS NOT NULL
                    OR profile.default_temporal_stance IS NOT NULL
                    OR profile.default_perception_basis IS NOT NULL)
               AND NOT EXISTS (
                 SELECT 1 FROM current_content_interpretations explicit
                 WHERE explicit.content_id = c.id AND explicit.char_start IS NULL
                   AND explicit.review_state <> 'rejected'
               )",
            [&case_id.0],
            |row| row.get(0),
        )?;
        Ok(EnrichmentRuleResult {
            already_recorded: inherited,
            ..EnrichmentRuleResult::empty()
        })
    }

    fn interpretation_candidates(
        &mut self,
        case_id: &CaseId,
        kind: SuggestionKind,
    ) -> Result<EnrichmentRuleResult> {
        let passages = {
            let mut statement = self.connection.prepare(
                "SELECT c.id, c.text
                 FROM content c
                 WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
                 ORDER BY c.id",
            )?;
            statement
                .query_map([&case_id.0], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        let attribution = kind.attribution();
        let mut output = EnrichmentRuleResult::empty();
        for (content_id, text) in passages {
            for detection in detections(kind, &text) {
                if self.interpretation_span_has_history(
                    case_id,
                    &content_id,
                    detection.char_start,
                    detection.char_end,
                )? {
                    output.already_recorded = output.already_recorded.saturating_add(1);
                    continue;
                }
                let mut provenance = BTreeMap::new();
                provenance.insert(
                    detection.field.to_owned(),
                    format!("suggested:{}", kind.as_str()),
                );
                let candidate = ProposedInterpretation {
                    id: None,
                    target: InterpretationTarget::Content {
                        id: content_id.clone(),
                    },
                    char_start: detection.char_start,
                    char_end: detection.char_end,
                    content_form: detection.content_form,
                    perception_basis: None,
                    temporal_stance: None,
                    speaker_entity_id: None,
                    attributed_entity_id: None,
                    reporting_parent_interpretation_id: None,
                    content_created_at: None,
                    asserted_start: detection.asserted_start,
                    asserted_end: None,
                    normalized_start: None,
                    normalized_end: None,
                    time_alignment_basis: None,
                    location_text: None,
                    location_entity_id: None,
                    materiality: Materiality::Unknown,
                    field_provenance: provenance,
                    basis: Some(detection.basis),
                    review_state: ReviewState::Suggested,
                    created_by: attribution.clone(),
                    supersedes_interpretation_id: None,
                };
                output
                    .interpretations
                    .push(self.append_suggested_interpretation(case_id, &candidate)?);
            }
        }
        Ok(output)
    }

    fn interpretation_span_has_history(
        &self,
        case_id: &CaseId,
        content_id: &str,
        char_start: Option<u32>,
        char_end: Option<u32>,
    ) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM content_interpretations
                 WHERE case_id = ?1 AND content_id = ?2
                   AND char_start IS ?3 AND char_end IS ?4
                 LIMIT 1",
                params![case_id.0, content_id, char_start, char_end],
                |_| Ok(true),
            )
            .optional()
            .map(|found| found.unwrap_or(false))
            .map_err(Into::into)
    }

    fn header_date_candidates(&mut self, case_id: &CaseId) -> Result<EnrichmentRuleResult> {
        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT src.id, src.logical_name, c.text
                 FROM sources src
                 JOIN source_segments seg ON seg.source_id = src.id AND seg.page = 1
                 JOIN content c ON c.segment_id = seg.id AND c.review_state <> 'rejected'
                 WHERE src.case_id = ?1
                 ORDER BY src.id, seg.locator, c.id",
            )?;
            statement
                .query_map([&case_id.0], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut first_dates = BTreeMap::<String, (String, String)>::new();
        for (source_id, logical_name, text) in rows {
            if first_dates.contains_key(&source_id) {
                continue;
            }
            let header: String = text.chars().take(320).collect();
            if let Some(date) = find_date(&header) {
                first_dates.insert(source_id, (logical_name, date));
            }
        }

        let attribution = SuggestionKind::HeaderDate.attribution();
        let mut output = EnrichmentRuleResult::empty();
        for (source_id, (logical_name, date)) in first_dates {
            let held = self
                .connection
                .query_row(
                    "SELECT 1 FROM source_profiles WHERE case_id = ?1 AND source_id = ?2 LIMIT 1",
                    params![case_id.0, source_id],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if held {
                output.already_recorded = output.already_recorded.saturating_add(1);
                continue;
            }
            let profile = ProposedSourceProfile {
                id: None,
                source_id,
                source_role: infer_source_role(&logical_name),
                author_entity_id: None,
                created_at_claim: Some(date),
                default_content_form: None,
                default_temporal_stance: None,
                default_perception_basis: None,
                clock_offset_ms: None,
                clock_offset_basis: None,
                review_state: ReviewState::Suggested,
                created_by: attribution.clone(),
                supersedes_profile_id: None,
            };
            output
                .profiles
                .push(self.append_suggested_source_profile(case_id, &profile)?);
        }
        Ok(output)
    }

    /// Returns relationship candidates produced by enrichment rules.
    pub(super) fn enrichment_edge_candidates(
        &self,
        case_id: &CaseId,
        kind: SuggestionKind,
    ) -> Result<Vec<Candidate>> {
        match kind {
            SuggestionKind::DiarizationSpeaker => self.diarization_candidates(case_id),
            SuggestionKind::CrossDocumentEcho => self.cross_document_echoes(case_id),
            SuggestionKind::SharedAnchor => self.shared_anchor_candidates(case_id),
            _ => Err(Error::InvalidAuthoring(format!(
                "`{}` is not an enrichment edge rule",
                kind.as_str()
            ))),
        }
    }

    /// Proposes `refers_to` only when reference language also names an original
    /// already present in the same case.
    pub(super) fn evidence_reference_edge_candidates(
        &self,
        case_id: &CaseId,
    ) -> Result<Vec<Candidate>> {
        let passages = {
            let mut statement = self.connection.prepare(
                "SELECT c.id, seg.source_id, c.text
                 FROM content c
                 JOIN source_segments seg ON seg.id = c.segment_id
                 WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
                 ORDER BY c.id",
            )?;
            statement
                .query_map([&case_id.0], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let sources = {
            let mut statement = self
                .connection
                .prepare("SELECT id, logical_name FROM sources WHERE case_id = ?1 ORDER BY id")?;
            statement
                .query_map([&case_id.0], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut candidates = Vec::new();
        for (content_id, own_source, text) in passages {
            if !is_evidence_reference(&text) {
                continue;
            }
            for (source_id, logical_name) in &sources {
                if source_id == &own_source || !name_mentioned(&text, logical_name) {
                    continue;
                }
                candidates.push(Candidate {
                    from_kind: NodeKind::Content,
                    to_kind: NodeKind::Source,
                    relation: EdgeKind::RefersTo,
                    from: content_id.clone(),
                    to: source_id.clone(),
                    rationale: format!(
                        "This passage uses a controlled evidence-reference phrase and names the produced original `{logical_name}`. The edge records that textual reference only; it does not establish authenticity or completeness."
                    ),
                });
            }
        }
        Ok(candidates)
    }

    fn diarization_candidates(&self, case_id: &CaseId) -> Result<Vec<Candidate>> {
        let mut statement = self.connection.prepare(
            "SELECT statement.id, label.id, label.text
             FROM content label
             JOIN content statement ON statement.segment_id = label.segment_id
             WHERE label.case_id = ?1 AND label.extractor = 'whisperx_diarize'
               AND label.review_state <> 'rejected' AND statement.review_state <> 'rejected'
               AND statement.kind = 'statement' AND statement.id <> label.id
             ORDER BY statement.id, label.id",
        )?;
        statement
            .query_map([&case_id.0], |row| {
                let label: String = row.get(2)?;
                Ok(Candidate {
                    from_kind: NodeKind::Content,
                    to_kind: NodeKind::Content,
                    relation: EdgeKind::SpeakerCandidate,
                    from: row.get(0)?,
                    to: row.get(1)?,
                    rationale: format!(
                        "The diarization adapter aligned `{label}` to this statement's exact segment. This proposes only a speaker label association; it does not identify a person."
                    ),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn cross_document_echoes(&self, case_id: &CaseId) -> Result<Vec<Candidate>> {
        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT c.id, seg.source_id, c.text,
                        COALESCE(c.content_created_at,
                          (SELECT profile.created_at_claim
                           FROM current_source_profiles profile
                           WHERE profile.source_id = seg.source_id
                             AND profile.review_state IN ('reviewed','verified')
                           ORDER BY profile.created_at DESC, profile.id DESC LIMIT 1))
                 FROM content c
                 JOIN source_segments seg ON seg.id = c.segment_id
                 WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
                 ORDER BY c.id",
            )?;
            statement
                .query_map([&case_id.0], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut candidates = Vec::new();
        for (index, left) in rows.iter().enumerate() {
            for right in &rows[index + 1..] {
                if left.1 == right.1 {
                    continue;
                }
                let Some(run) = shared_token_run(&left.2, &right.2, 12) else {
                    continue;
                };
                let (Some(left_date), Some(right_date)) = (&left.3, &right.3) else {
                    continue;
                };
                if left_date == right_date {
                    continue;
                }
                let (from, to, later, earlier) = if left_date > right_date {
                    (&left.0, &right.0, left_date, right_date)
                } else {
                    (&right.0, &left.0, right_date, left_date)
                };
                candidates.push(Candidate {
                    from_kind: NodeKind::Content,
                    to_kind: NodeKind::Content,
                    relation: EdgeKind::Quotes,
                    from: from.clone(),
                    to: to.clone(),
                    rationale: format!(
                        "Different originals contain this exact 12-token run: `{run}`. The {later} passage is provisionally directed toward the {earlier} passage using reviewed creation claims; a person must decide whether it is quotation, common-source reuse, or coincidence."
                    ),
                });
            }
        }
        Ok(candidates)
    }

    fn shared_anchor_candidates(&self, case_id: &CaseId) -> Result<Vec<Candidate>> {
        let mut statement = self.connection.prepare(
            "WITH anchors AS (
               SELECT interpretation.content_id AS content_id, seg.source_id AS source_id,
                      interpretation.normalized_start AS at_time,
                      lower(trim(interpretation.location_text)) AS at_place
               FROM current_content_interpretations interpretation
               JOIN content c ON c.id = interpretation.content_id
               JOIN source_segments seg ON seg.id = c.segment_id
               WHERE interpretation.case_id = ?1
                 AND interpretation.review_state IN ('reviewed','verified')
                 AND interpretation.normalized_start IS NOT NULL
                 AND length(trim(interpretation.location_text)) > 0
               UNION
               SELECT c.id, seg.source_id, c.normalized_start, lower(trim(c.location_text))
               FROM content c
               JOIN source_segments seg ON seg.id = c.segment_id
               WHERE c.case_id = ?1 AND c.review_state IN ('reviewed','verified')
                 AND c.normalized_start IS NOT NULL AND length(trim(c.location_text)) > 0
             )
             SELECT MIN(a.content_id, b.content_id), MAX(a.content_id, b.content_id),
                    a.at_time, a.at_place
             FROM anchors a
             JOIN anchors b ON b.content_id > a.content_id AND b.source_id <> a.source_id
                           AND b.at_time = a.at_time AND b.at_place = a.at_place
             GROUP BY 1, 2, 3, 4
             ORDER BY 1, 2",
        )?;
        statement
            .query_map([&case_id.0], |row| {
                let at_time: String = row.get(2)?;
                let at_place: String = row.get(3)?;
                Ok(Candidate {
                    from_kind: NodeKind::Content,
                    to_kind: NodeKind::Content,
                    relation: EdgeKind::CandidateSameOccurrence,
                    from: row.get(0)?,
                    to: row.get(1)?,
                    rationale: format!(
                        "Two different originals carry the same reviewed normalized time `{at_time}` and location `{at_place}`. This is a shared anchor for same-occurrence review, not a conclusion that the passages describe one event."
                    ),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn detections(kind: SuggestionKind, text: &str) -> Vec<Detection> {
    match kind {
        SuggestionKind::ReportedStatement if has_reported_statement_cue(text) => vec![Detection {
            char_start: None,
            char_end: None,
            content_form: Some(ContentForm::ReportedStatement),
            asserted_start: None,
            field: "content_form",
            basis: "A controlled reported-speech cue occurs as complete words in the passage. The rule classifies form only; it does not identify the attributed speaker or endorse the report.".to_owned(),
        }],
        SuggestionKind::QuotedStatement => quote_spans(text)
            .into_iter()
            .map(|(start, end)| Detection {
                char_start: Some(start),
                char_end: Some(end),
                content_form: Some(ContentForm::QuotedStatement),
                asserted_start: None,
                field: "content_form",
                basis: "Balanced quotation marks enclose at least six whitespace-delimited words. The marks define the proposed span; attribution remains unset.".to_owned(),
            })
            .collect(),
        SuggestionKind::MeasuredResult if contains_measurement(text) => vec![Detection {
            char_start: None,
            char_end: None,
            content_form: Some(ContentForm::MeasuredResult),
            asserted_start: None,
            field: "content_form",
            basis: "A numeric token is immediately followed by a controlled measurement unit. This proposes measured-result form without deciding who made or validated the measurement.".to_owned(),
        }],
        SuggestionKind::OfficialCharacterization if contains_any_phrase(
            text,
            &[
                &["probable", "cause"],
                &["pursuant", "to"],
                &["in", "violation", "of"],
                &["charged", "with"],
                &["statutory", "elements"],
            ],
        ) => vec![Detection {
            char_start: None,
            char_end: None,
            content_form: Some(ContentForm::OfficialCharacterization),
            asserted_start: None,
            field: "content_form",
            basis: "A controlled legal/official phrase occurs as complete words. The proposal distinguishes characterization from direct observation; it makes no legal finding.".to_owned(),
        }],
        SuggestionKind::EvidenceReference if is_evidence_reference(text) => vec![Detection {
            char_start: None,
            char_end: None,
            content_form: Some(ContentForm::EvidenceReference),
            asserted_start: None,
            field: "content_form",
            basis: "A controlled evidence-reference phrase occurs as complete words. Resolution to a produced original remains separate review work.".to_owned(),
        }],
        SuggestionKind::AssertedClock => clock_tokens(text)
            .into_iter()
            .take(1)
            .map(|clock| Detection {
                char_start: None,
                char_end: None,
                content_form: None,
                asserted_start: Some(clock.clone()),
                field: "asserted_start",
                basis: format!(
                    "`{clock}` has a valid clock-token shape. It is retained verbatim as an asserted time, not normalized to the case clock."
                ),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric() && character != '%')
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn contains_any_phrase(text: &str, phrases: &[&[&str]]) -> bool {
    let tokens = words(text);
    phrases.iter().any(|phrase| {
        tokens
            .windows(phrase.len())
            .any(|window| window.iter().map(String::as_str).eq(phrase.iter().copied()))
    })
}

fn has_reported_statement_cue(text: &str) -> bool {
    contains_any_phrase(
        text,
        &[
            &["stated", "that"],
            &["said", "that"],
            &["reported", "that"],
            &["advised", "that"],
            &["told", "me"],
            &["according", "to"],
        ],
    )
}

fn is_evidence_reference(text: &str) -> bool {
    contains_any_phrase(
        text,
        &[
            &["see", "exhibit"],
            &["attached", "photograph"],
            &["evidence", "item"],
            &["property", "receipt"],
            &["body", "camera"],
            &["see", "attachment"],
            &["see", "recording"],
        ],
    )
}

fn name_mentioned(text: &str, logical_name: &str) -> bool {
    let passage = words(text);
    let stem = logical_name
        .rsplit_once('.')
        .map_or(logical_name, |(before, _)| before);
    let name = words(stem);
    !name.is_empty()
        && passage.windows(name.len()).any(|window| window == name)
        && name.iter().any(|token| token.len() >= 3)
}

fn contains_measurement(text: &str) -> bool {
    const UNITS: &[&str] = &[
        "mg",
        "g",
        "kg",
        "ml",
        "l",
        "mm",
        "cm",
        "m",
        "km",
        "mph",
        "kph",
        "percent",
        "%",
        "seconds",
        "minutes",
        "hours",
        "degrees",
        "celsius",
        "fahrenheit",
    ];
    let raw = text.split_whitespace().collect::<Vec<_>>();
    raw.windows(2).any(|pair| {
        let number = pair[0].trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-');
        let unit = pair[1]
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '%')
            .to_lowercase();
        number.parse::<f64>().is_ok() && UNITS.contains(&unit.as_str())
    })
}

fn quote_spans(text: &str) -> Vec<(u32, u32)> {
    let mut spans = Vec::new();
    collect_quote_spans(text, '"', '"', &mut spans);
    collect_quote_spans(text, '“', '”', &mut spans);
    spans.sort_unstable();
    spans.dedup();
    spans
}

fn collect_quote_spans(text: &str, open: char, close: char, spans: &mut Vec<(u32, u32)>) {
    let mut start = None;
    for (byte, character) in text.char_indices() {
        if start.is_none() && character == open {
            start = Some(byte + character.len_utf8());
        } else if let Some(inner_start) = start
            && character == close
        {
            let quoted = &text[inner_start..byte];
            if quoted.split_whitespace().count() >= 6
                && let (Ok(open_at), Ok(close_at)) =
                    (u32::try_from(inner_start), u32::try_from(byte))
            {
                spans.push((open_at, close_at));
            }
            start = None;
        }
    }
}

fn clock_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|raw| {
            let token = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':');
            let (hour, minute) = token.split_once(':')?;
            if hour.is_empty() || hour.len() > 2 || minute.len() != 2 {
                return None;
            }
            let hour = hour.parse::<u8>().ok()?;
            let minute = minute.parse::<u8>().ok()?;
            (hour <= 23 && minute <= 59).then(|| token.to_owned())
        })
        .collect()
}

fn find_date(text: &str) -> Option<String> {
    let raw = text.split_whitespace().collect::<Vec<_>>();
    for (index, token) in raw.iter().enumerate() {
        let clean =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '/');
        if valid_iso_date(clean) || valid_slash_date(clean) {
            return Some(clean.to_owned());
        }
        if is_month(clean) && index + 2 < raw.len() {
            let day = raw[index + 1].trim_matches(|c: char| !c.is_ascii_digit());
            let year = raw[index + 2].trim_matches(|c: char| !c.is_ascii_digit());
            if day
                .parse::<u8>()
                .is_ok_and(|value| (1..=31).contains(&value))
                && year.len() == 4
                && year
                    .parse::<u16>()
                    .is_ok_and(|value| (1900..=2200).contains(&value))
            {
                return Some(format!("{clean} {day}, {year}"));
            }
        }
    }
    None
}

fn valid_iso_date(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[0]
            .parse::<u16>()
            .is_ok_and(|year| (1900..=2200).contains(&year))
        && valid_month_day(parts[1], parts[2])
}

fn valid_slash_date(value: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts[2].len() == 4
        && parts[2]
            .parse::<u16>()
            .is_ok_and(|year| (1900..=2200).contains(&year))
        && valid_month_day(parts[0], parts[1])
}

fn valid_month_day(month: &str, day: &str) -> bool {
    month
        .parse::<u8>()
        .is_ok_and(|value| (1..=12).contains(&value))
        && day
            .parse::<u8>()
            .is_ok_and(|value| (1..=31).contains(&value))
}

fn is_month(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    )
}

fn infer_source_role(logical_name: &str) -> SourceRole {
    let name = logical_name.to_ascii_lowercase();
    if name.contains("supplement") {
        SourceRole::SupplementalReport
    } else if name.contains("police") || name.contains("incident") || name.contains("offense") {
        SourceRole::PoliceReport
    } else if name.contains("lab") || name.contains("toxicology") {
        SourceRole::LabReport
    } else if name.contains("cad") || name.contains("dispatch log") {
        SourceRole::CadLog
    } else if name.contains("receipt") {
        SourceRole::Receipt
    } else if name.contains("medical") {
        SourceRole::MedicalRecord
    } else {
        SourceRole::Other
    }
}

fn shared_token_run(left: &str, right: &str, length: usize) -> Option<String> {
    let left_words = words(left);
    let right_words = words(right);
    if left_words.len() < length || right_words.len() < length {
        return None;
    }
    let right_runs = right_words
        .windows(length)
        .map(|run| run.join(" "))
        .collect::<HashSet<_>>();
    left_words
        .windows(length)
        .map(|run| run.join(" "))
        .find(|run| right_runs.contains(run))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cues_require_complete_token_sequences() {
        assert!(has_reported_statement_cue(
            "The witness stated that it was dark."
        ));
        assert!(!has_reported_statement_cue(
            "The report statedly catalogued thatch."
        ));
        assert!(contains_measurement("The sample weighed 12.5 mg."));
        assert!(!contains_measurement("Evidence item 12 was sealed."));
    }

    #[test]
    fn quote_rule_uses_utf8_byte_spans_and_minimum_length() {
        let text = "Prefix “one two three four five six” suffix \"too short\".";
        let spans = quote_spans(text);
        assert_eq!(spans.len(), 1);
        let (start, end) = spans[0];
        assert_eq!(
            &text[start as usize..end as usize],
            "one two three four five six"
        );
    }

    #[test]
    fn dates_and_clocks_are_shape_checked() {
        assert_eq!(
            find_date("REPORT DATE 2025-02-09"),
            Some("2025-02-09".to_owned())
        );
        assert_eq!(
            find_date("dated February 9, 2025"),
            Some("February 9, 2025".to_owned())
        );
        assert_eq!(clock_tokens("At 23:59, not 25:90."), vec!["23:59"]);
    }

    #[test]
    fn echo_requires_exact_controlled_run() {
        let shared = "one two three four five six seven eight nine ten eleven twelve";
        assert_eq!(
            shared_token_run(shared, &format!("before {shared} after"), 12).as_deref(),
            Some(shared)
        );
        assert!(
            shared_token_run(
                shared,
                "one two three four five six seven eight nine ten eleven changed",
                12
            )
            .is_none()
        );
    }
}
