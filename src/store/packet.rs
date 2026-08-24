//! Source-linked proposition packets.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::{
    CaseId, ContentForm, InterpretationTarget, NodeKind, NodeRef, PacketItem, PacketSection,
    PropositionPacket, Result, ReviewState, TemporalStance,
};

#[derive(Debug)]
struct LinkedPassage {
    item: PacketItem,
    ready: bool,
    has_speaker: bool,
}

#[derive(Debug, Default)]
struct SemanticSnapshot {
    content_form: Option<ContentForm>,
    temporal_stance: Option<TemporalStance>,
    content_created_at: Option<String>,
    asserted_start: Option<String>,
    normalized_start: Option<String>,
    location: Option<String>,
    state: String,
    reviewed: bool,
    has_speaker: bool,
}

impl Store {
    /// Builds one packet per contested proposition, in proposition order.
    ///
    /// The order is the propositions' own text order, not an assessment of
    /// which matters most: nothing here ranks a proposition, and a reader who
    /// wants a different order is the one who decides it.
    pub fn proposition_packets(&self, case_id: &CaseId) -> Result<Vec<PropositionPacket>> {
        self.require_case(case_id)?;
        self.digest_proposition_ids(case_id)?
            .into_iter()
            .map(|id| self.proposition_packet(case_id, &id))
            .collect()
    }

    /// Builds the fixed, non-scoring workspace around one proposition.
    pub fn proposition_packet(
        &self,
        case_id: &CaseId,
        proposition_id: &str,
    ) -> Result<PropositionPacket> {
        self.require_case(case_id)?;
        let (proposition, status): (String, String) = self
            .connection
            .query_row(
                "SELECT text, status FROM propositions WHERE case_id = ?1 AND id = ?2",
                params![case_id.0, proposition_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| crate::Error::NotFound {
                kind: "proposition",
                id: proposition_id.to_owned(),
            })?;

        let linked = self.packet_linked_passages(case_id, proposition_id)?;
        let mut by_key = BTreeMap::<&'static str, Vec<PacketItem>>::new();
        for key in [
            "direct_capture",
            "contemporaneous_accounts",
            "later_accounts",
            "official_assertions",
            "reporting_dependencies",
            "evaluative_relations",
            "raw_conflicts",
            "missing_unreviewed",
        ] {
            by_key.insert(key, Vec::new());
        }

        for passage in &linked {
            let item = &passage.item;
            if !passage.ready || !reviewed_state(&item.relation_state) {
                push_unique(
                    by_key.entry("missing_unreviewed").or_default(),
                    item.clone(),
                );
                continue;
            }

            if is_evaluative(&item.relation) {
                push_unique(
                    by_key.entry("evaluative_relations").or_default(),
                    item.clone(),
                );
            }
            if item.lineage_roots.len() > 1
                || self.has_packet_dependency(case_id, &item.content_id)?
            {
                push_unique(
                    by_key.entry("reporting_dependencies").or_default(),
                    item.clone(),
                );
            }

            let destination = match item.temporal_stance.as_deref() {
                Some("contemporaneous_capture") => "direct_capture",
                Some("contemporaneous_account") => "contemporaneous_accounts",
                Some(
                    "retrospective_recollection"
                    | "report_of_prior_statement"
                    | "later_measurement"
                    | "later_analysis",
                ) => "later_accounts",
                _ if matches!(
                    item.content_form.as_deref(),
                    Some("official_characterization" | "authored_assertion")
                ) =>
                {
                    "official_assertions"
                }
                _ => "missing_unreviewed",
            };
            push_unique(by_key.entry(destination).or_default(), item.clone());

            if (!passage.has_speaker
                && self.has_unresolved_speaker_candidate(case_id, &item.content_id)?)
                || (item.asserted_start.is_some() && item.normalized_start.is_none())
            {
                push_unique(
                    by_key.entry("missing_unreviewed").or_default(),
                    item.clone(),
                );
            }
        }

        let ready = linked
            .iter()
            .filter(|passage| passage.ready && reviewed_state(&passage.item.relation_state))
            .collect::<Vec<_>>();
        let times = ready
            .iter()
            .filter_map(|passage| {
                passage
                    .item
                    .normalized_start
                    .as_ref()
                    .or(passage.item.asserted_start.as_ref())
            })
            .collect::<BTreeSet<_>>();
        let locations = ready
            .iter()
            .filter_map(|passage| passage.item.location.as_ref())
            .map(|location| location.trim().to_ascii_lowercase())
            .filter(|location| !location.is_empty())
            .collect::<BTreeSet<_>>();
        if times.len() > 1 || locations.len() > 1 {
            for passage in &ready {
                push_unique(
                    by_key.entry("raw_conflicts").or_default(),
                    passage.item.clone(),
                );
            }
        }

        let mut notes = BTreeMap::<&'static str, Vec<String>>::new();
        if by_key.get("direct_capture").is_none_or(Vec::is_empty) {
            notes.entry("missing_unreviewed").or_default().push(format!(
                "No reviewed linked original directly captures the contested proposition: {proposition}"
            ));
        }
        if times.len() > 1 {
            notes.entry("raw_conflicts").or_default().push(
                "Reviewed passages carry different asserted or normalized times; no value was selected as correct."
                    .to_owned(),
            );
        }
        if locations.len() > 1 {
            notes.entry("raw_conflicts").or_default().push(
                "Reviewed passages carry different location wording; no location was selected as correct."
                    .to_owned(),
            );
        }

        let section_specs = [
            ("direct_capture", "Directly captured material"),
            ("contemporaneous_accounts", "Contemporaneous accounts"),
            ("later_accounts", "Later first-person and reported accounts"),
            ("official_assertions", "Official and documentary assertions"),
            ("reporting_dependencies", "Reporting and derivation chains"),
            ("evaluative_relations", "Evaluative relations"),
            ("raw_conflicts", "Unresolved time and location conflicts"),
            ("missing_unreviewed", "Missing and unreviewed material"),
        ];
        let mut sections = Vec::with_capacity(section_specs.len());
        for (key, title) in section_specs {
            let items = by_key.remove(key).unwrap_or_default();
            let counted = item_nodes(&items);
            sections.push(PacketSection {
                key: key.to_owned(),
                title: title.to_owned(),
                count: self.structural_count(case_id, &counted)?,
                items,
                notes: notes.remove(key).unwrap_or_default(),
            });
        }

        let all_nodes = linked
            .iter()
            .map(|passage| NodeRef::new(NodeKind::Content, passage.item.content_id.clone()))
            .collect::<Vec<_>>();
        Ok(PropositionPacket {
            case_id: case_id.0.clone(),
            proposition_id: proposition_id.to_owned(),
            proposition,
            status,
            sections,
            count: self.structural_count(case_id, &all_nodes)?,
        })
    }

    fn packet_linked_passages(
        &self,
        case_id: &CaseId,
        proposition_id: &str,
    ) -> Result<Vec<LinkedPassage>> {
        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT c.id, seg.source_id, src.logical_name, seg.locator, c.text,
                        c.content_created_at, c.asserted_time, c.normalized_start,
                        c.location_text, c.review_state, edge.relation,
                        edge.review_state, edge.rationale
                 FROM edges edge
                 JOIN content c ON edge.source_kind = 'content' AND c.id = edge.source_id
                 JOIN source_segments seg ON seg.id = c.segment_id
                 JOIN sources src ON src.id = seg.source_id
                 WHERE edge.case_id = ?1 AND edge.target_kind = 'proposition'
                   AND edge.target_id = ?2 AND edge.review_state <> 'rejected'
                 ORDER BY src.logical_name, seg.locator, c.id, edge.relation",
            )?;
            statement
                .query_map(params![case_id.0, proposition_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        rows.into_iter()
            .map(
                |(
                    content_id,
                    source_id,
                    original,
                    locator,
                    text,
                    raw_created,
                    raw_asserted,
                    raw_normalized,
                    raw_location,
                    content_state,
                    relation,
                    relation_state,
                    rationale,
                )| {
                    let semantic = self.reviewed_semantic_snapshot(
                        case_id,
                        &content_id,
                        &source_id,
                        &content_state,
                        raw_created,
                        raw_asserted,
                        raw_normalized,
                        raw_location,
                    )?;
                    let node = NodeRef::new(NodeKind::Content, content_id.clone());
                    let lineage = self.lineage(case_id, &node)?;
                    Ok(LinkedPassage {
                        ready: reviewed_state(&content_state) && semantic.reviewed,
                        has_speaker: semantic.has_speaker,
                        item: PacketItem {
                            content_id,
                            relation,
                            text,
                            original,
                            locator,
                            content_form: semantic
                                .content_form
                                .map(ContentForm::as_str)
                                .map(str::to_owned),
                            temporal_stance: semantic
                                .temporal_stance
                                .map(TemporalStance::as_str)
                                .map(str::to_owned),
                            content_created_at: semantic.content_created_at,
                            asserted_start: semantic.asserted_start,
                            normalized_start: semantic.normalized_start,
                            location: semantic.location,
                            interpretation_state: semantic.state,
                            relation_state,
                            rationale,
                            lineage_roots: lineage.roots,
                        },
                    })
                },
            )
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn reviewed_semantic_snapshot(
        &self,
        case_id: &CaseId,
        content_id: &str,
        source_id: &str,
        content_state: &str,
        raw_created: Option<String>,
        raw_asserted: Option<String>,
        raw_normalized: Option<String>,
        raw_location: Option<String>,
    ) -> Result<SemanticSnapshot> {
        let target = InterpretationTarget::Content {
            id: content_id.to_owned(),
        };
        let explicit = self
            .current_interpretation(case_id, &target, None, None)?
            .filter(|item| {
                matches!(
                    item.review_state,
                    ReviewState::Reviewed | ReviewState::Verified
                )
            });
        let profile = self
            .current_source_profile(case_id, source_id)?
            .filter(|item| {
                matches!(
                    item.review_state,
                    ReviewState::Reviewed | ReviewState::Verified
                )
            });
        let state = explicit.as_ref().map_or_else(
            || {
                profile.as_ref().map_or_else(
                    || "unset".to_owned(),
                    |item| format!("{}:source_profile", item.review_state.as_str()),
                )
            },
            |item| item.review_state.as_str().to_owned(),
        );
        let raw_is_reviewed = reviewed_state(content_state);
        Ok(SemanticSnapshot {
            content_form: explicit
                .as_ref()
                .and_then(|item| item.content_form)
                .or_else(|| profile.as_ref().and_then(|item| item.default_content_form)),
            temporal_stance: explicit
                .as_ref()
                .and_then(|item| item.temporal_stance)
                .or_else(|| {
                    profile
                        .as_ref()
                        .and_then(|item| item.default_temporal_stance)
                }),
            content_created_at: explicit
                .as_ref()
                .and_then(|item| item.content_created_at.clone())
                .or_else(|| {
                    profile
                        .as_ref()
                        .and_then(|item| item.created_at_claim.clone())
                })
                .or_else(|| raw_is_reviewed.then_some(raw_created).flatten()),
            asserted_start: explicit
                .as_ref()
                .and_then(|item| item.asserted_start.clone())
                .or_else(|| raw_is_reviewed.then_some(raw_asserted).flatten()),
            normalized_start: explicit
                .as_ref()
                .and_then(|item| item.normalized_start.clone())
                .or_else(|| raw_is_reviewed.then_some(raw_normalized).flatten()),
            location: explicit
                .as_ref()
                .and_then(|item| item.location_text.clone())
                .or_else(|| raw_is_reviewed.then_some(raw_location).flatten()),
            reviewed: explicit.is_some() || profile.is_some(),
            has_speaker: explicit
                .as_ref()
                .and_then(|item| item.speaker_entity_id.as_ref())
                .or_else(|| {
                    profile
                        .as_ref()
                        .and_then(|item| item.author_entity_id.as_ref())
                })
                .is_some(),
            state,
        })
    }

    fn has_packet_dependency(&self, case_id: &CaseId, content_id: &str) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM edges
                 WHERE case_id = ?1 AND source_kind = 'content' AND source_id = ?2
                   AND review_state IN ('reviewed','verified')
                   AND relation IN ('quotes','reports','summarizes','transcribes','based_on',
                                    'derived_from','records_utterance') LIMIT 1",
                params![case_id.0, content_id],
                |_| Ok(true),
            )
            .optional()
            .map(|found| found.unwrap_or(false))
            .map_err(Into::into)
    }

    fn has_unresolved_speaker_candidate(&self, case_id: &CaseId, content_id: &str) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM edges
                 WHERE case_id = ?1 AND relation = 'speaker_candidate'
                   AND review_state <> 'rejected'
                   AND ((source_kind = 'content' AND source_id = ?2)
                     OR (target_kind = 'content' AND target_id = ?2))
                 LIMIT 1",
                params![case_id.0, content_id],
                |_| Ok(true),
            )
            .optional()
            .map(|found| found.unwrap_or(false))
            .map_err(Into::into)
    }
}

fn reviewed_state(state: &str) -> bool {
    matches!(state, "reviewed" | "verified")
}

fn is_evaluative(relation: &str) -> bool {
    matches!(
        relation,
        "supports"
            | "contradicts"
            | "qualifies"
            | "explains"
            | "impeaches"
            | "consistent_with"
            | "independently_corroborates"
    )
}

fn push_unique(items: &mut Vec<PacketItem>, item: PacketItem) {
    if !items
        .iter()
        .any(|held| held.content_id == item.content_id && held.relation == item.relation)
    {
        items.push(item);
    }
}

fn item_nodes(items: &[PacketItem]) -> Vec<NodeRef> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|item| seen.insert(item.content_id.clone()))
        .map(|item| NodeRef::new(NodeKind::Content, item.content_id.clone()))
        .collect()
}
