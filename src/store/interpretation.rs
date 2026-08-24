//! Persistence for append-only semantic interpretation.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rusqlite::{OptionalExtension, Row, Transaction, params, types::Type};
use uuid::Uuid;

use super::Store;
use crate::{
    CaseId, ContentForm, ContentInterpretation, EffectiveInterpretation, EnrichmentPassage,
    EnrichmentSource, EntityCandidate, Error, InterpretationBatch, InterpretationTarget,
    Materiality, NodeKind, PerceptionBasis, PreviewDescriptor, ProposedContentGroup,
    ProposedInterpretation, ProposedSourceProfile, Result, ReviewState, SourceLocation,
    SourceProfile, SourceRole, TemporalStance,
};

const INTERPRETATION_COLUMNS: &str =
    "id, content_id, content_group_id, char_start, char_end, content_form,
     perception_basis, temporal_stance, speaker_entity_id, attributed_entity_id,
     reporting_parent_interpretation_id, content_created_at, asserted_start, asserted_end,
     normalized_start, normalized_end, time_alignment_basis, location_text,
     location_entity_id, materiality, field_provenance_json, basis, review_state,
     created_by, supersedes_interpretation_id, created_at";

/// The head of a passage's interpretation chain, chosen exactly as
/// `current_interpretation` chooses it, so the source picker's arithmetic and
/// the grid's rows can never disagree about what the current reading is.
const CURRENT_HEAD_STATE: &str = "(SELECT ci.review_state FROM current_content_interpretations ci
                                   WHERE ci.case_id = c.case_id AND ci.content_id = c.id
                                     AND ci.content_group_id IS NULL
                                     AND ci.char_start IS NULL AND ci.char_end IS NULL
                                   ORDER BY ci.created_at DESC, ci.id DESC LIMIT 1)";

const PROFILE_COLUMNS: &str =
    "id, source_id, source_role, author_entity_id, created_at_claim, default_content_form,
     default_temporal_stance, default_perception_basis, clock_offset_ms,
     clock_offset_basis, review_state, created_by, supersedes_profile_id, created_at";

impl Store {
    /// Returns one source's passages in original order with effective readings.
    pub fn enrichment_passages(
        &self,
        case_id: &CaseId,
        source_id: &str,
    ) -> Result<Vec<EnrichmentPassage>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT c.id, src.id, src.logical_name, src.source_kind, seg.locator, c.text,
                    seg.page, seg.start_ms, seg.end_ms, seg.bbox_json,
                    c.machine_generated, c.extractor, c.review_state
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.case_id = ?1 AND src.id = ?2 AND c.review_state <> 'rejected'
             ORDER BY CASE WHEN seg.page IS NULL THEN 1 ELSE 0 END, seg.page,
                      CASE WHEN seg.start_ms IS NULL THEN 1 ELSE 0 END, seg.start_ms,
                      seg.locator, c.id",
        )?;
        let raw = statement
            .query_map(params![case_id.0, source_id], |row| {
                let bbox: Option<String> = row.get(9)?;
                let bounding_box = bbox
                    .map(|json| {
                        serde_json::from_str(&json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                9,
                                Type::Text,
                                Box::new(error),
                            )
                        })
                    })
                    .transpose()?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<u32>>(6)?,
                    sql_u64(row.get::<_, Option<i64>>(7)?, 7)?,
                    sql_u64(row.get::<_, Option<i64>>(8)?, 8)?,
                    bounding_box,
                    row.get::<_, i64>(10)? != 0,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, String>(12)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);

        raw.into_iter()
            .map(
                |(
                    content_id,
                    source_id,
                    source_name,
                    source_kind,
                    locator,
                    text,
                    page,
                    start_ms,
                    end_ms,
                    bounding_box,
                    machine_generated,
                    extractor,
                    content_review_state,
                )| {
                    let target = InterpretationTarget::Content {
                        id: content_id.clone(),
                    };
                    let current = self.current_interpretation(case_id, &target, None, None)?;
                    let effective = self.effective_interpretation(case_id, &target)?;
                    Ok(EnrichmentPassage {
                        content_id,
                        source_id,
                        source_name,
                        source_kind,
                        locator,
                        text,
                        page,
                        start_ms,
                        end_ms,
                        bounding_box,
                        machine_generated,
                        extractor,
                        content_review_state,
                        current,
                        effective,
                    })
                },
            )
            .collect()
    }

    /// Lists the case's originals with how much of each a person has read.
    ///
    /// The counts are deliberately three plain numbers rather than a
    /// completion percentage: a source is not "done", and the passages a
    /// person left alone are as often deliberate as unfinished. `decided`
    /// counts the passages carrying an explicit reading a person entered,
    /// `candidates` the ones where a deterministic rule is waiting for an
    /// answer, and the difference is what a sweep would still walk.
    pub fn enrichment_sources(&self, case_id: &CaseId) -> Result<Vec<EnrichmentSource>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT src.id, src.logical_name, src.source_kind, COUNT(*),
                    SUM(CASE WHEN {CURRENT_HEAD_STATE} IN ('unreviewed','reviewed','verified') THEN 1 ELSE 0 END),
                    SUM(CASE WHEN {CURRENT_HEAD_STATE} = 'suggested' THEN 1 ELSE 0 END)
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
             GROUP BY src.id, src.logical_name, src.source_kind
             ORDER BY src.logical_name, src.id"
        ))?;
        let rows = statement
            .query_map([&case_id.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);

        rows.into_iter()
            .map(|(source_id, name, kind, passages, decided, candidates)| {
                let profile = self.current_source_profile(case_id, &source_id)?;
                let profile_author = match profile
                    .as_ref()
                    .and_then(|item| item.author_entity_id.as_ref())
                {
                    Some(entity) => self.entity_name(case_id, entity)?,
                    None => None,
                };
                Ok(EnrichmentSource {
                    source_id,
                    name,
                    kind,
                    passages: u32::try_from(passages).unwrap_or(u32::MAX),
                    decided: u32::try_from(decided).unwrap_or(u32::MAX),
                    candidates: u32::try_from(candidates).unwrap_or(u32::MAX),
                    profile_role: profile
                        .as_ref()
                        .map(|item| item.source_role.as_str().to_owned()),
                    profile_author,
                    profile_state: profile
                        .as_ref()
                        .map(|item| item.review_state.as_str().to_owned()),
                })
            })
            .collect()
    }

    /// Offers the case's entities for `@` autocomplete, nearest match first.
    ///
    /// Matching is a plain case-insensitive substring, with names that begin
    /// with what was typed offered first. There is no fuzzy scoring: a person
    /// choosing whose words these are should be shown the case's own names,
    /// not a guess about which one was meant.
    pub fn entity_candidates(
        &self,
        case_id: &CaseId,
        prefix: &str,
        limit: u32,
    ) -> Result<Vec<EntityCandidate>> {
        self.require_case(case_id)?;
        let prefix = prefix.trim();
        let mut statement = self.connection.prepare_cached(
            "SELECT id, kind, display_name, notes FROM entities
             WHERE case_id = ?1
               AND (?2 = '' OR instr(lower(display_name), lower(?2)) > 0)
             ORDER BY CASE WHEN ?2 <> '' AND instr(lower(display_name), lower(?2)) = 1
                           THEN 0 ELSE 1 END,
                      display_name, id
             LIMIT ?3",
        )?;
        statement
            .query_map(params![case_id.0, prefix, limit], |row| {
                Ok(EntityCandidate {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    display_name: row.get(2)?,
                    notes: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Resolves one entity's display name inside the case.
    pub fn entity_name(&self, case_id: &CaseId, entity_id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT display_name FROM entities WHERE case_id = ?1 AND id = ?2",
                params![case_id.0, entity_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Rechecks an original's current bytes before returning its registered path.
    pub fn verified_source_location(&self, source_id: &str) -> Result<SourceLocation> {
        let location = self.source_location(source_id)?;
        let (hash, length): (String, i64) = self.connection.query_row(
            "SELECT sha256, byte_length FROM sources WHERE id = ?1",
            [source_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let length = u64::try_from(length).map_err(|_| {
            Error::InvalidIntake(format!("source `{source_id}` has an invalid stored length"))
        })?;
        super::verify_file_identity(&location.path, &hash, length)?;
        Ok(location)
    }

    /// Builds integrated document/audio/video context for one grid row.
    pub fn preview_descriptor(
        &self,
        case_id: &CaseId,
        passage: &EnrichmentPassage,
    ) -> Result<PreviewDescriptor> {
        if passage.source_id.is_empty() {
            return Err(Error::InvalidInterpretation(
                "preview passage has no source".to_owned(),
            ));
        }
        let original = match self.verified_source_location(&passage.source_id) {
            Ok(location) if location.case_id == *case_id => Some(location.path),
            Ok(_) => {
                return Err(Error::WrongCase {
                    kind: "source",
                    id: passage.source_id.clone(),
                });
            }
            Err(error) => {
                return Ok(PreviewDescriptor::Unavailable {
                    locator: passage.locator.clone(),
                    reason: error.to_string(),
                });
            }
        };
        let artifacts = self.preview_artifacts(&passage.source_id)?;
        match passage.source_kind.as_str() {
            "document" => Ok(PreviewDescriptor::Document {
                original_path: original,
                page: passage.page.unwrap_or(1),
                bounding_box: passage.bounding_box,
                page_image: select_page_preview(&artifacts, passage.page.unwrap_or(1)),
                verified_context: true,
            }),
            "audio" => Ok(PreviewDescriptor::Audio {
                original_path: original,
                start_ms: passage.start_ms.unwrap_or(0),
                end_ms: passage.end_ms.unwrap_or(passage.start_ms.unwrap_or(0)),
                waveform: artifact_paths(&artifacts, "waveform").next().cloned(),
                verified_context: true,
            }),
            "video" => Ok(PreviewDescriptor::Video {
                original_path: original,
                start_ms: passage.start_ms.unwrap_or(0),
                end_ms: passage.end_ms.unwrap_or(passage.start_ms.unwrap_or(0)),
                frames: artifact_paths(&artifacts, "still")
                    .take(12)
                    .cloned()
                    .collect(),
                verified_context: true,
            }),
            other => Ok(PreviewDescriptor::Unavailable {
                locator: passage.locator.clone(),
                reason: format!("integrated preview is not defined for source kind `{other}`"),
            }),
        }
    }

    fn preview_artifacts(&self, source_id: &str) -> Result<Vec<(String, PathBuf)>> {
        let mut statement = self.connection.prepare(
            "SELECT artifact.kind, artifact.path, artifact.sha256
             FROM intake_artifacts artifact
             JOIN intake_jobs job ON job.id = artifact.job_id
             WHERE job.source_id = ?1 AND job.state = 'completed'
             ORDER BY artifact.kind, artifact.path",
        )?;
        let candidates = statement
            .query_map([source_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    PathBuf::from(row.get::<_, String>(1)?),
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut verified = Vec::new();
        for (kind, path, hash) in candidates {
            let Some(hash) = hash else { continue };
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            if super::verify_file_identity(&path, &hash, metadata.len()).is_ok() {
                verified.push((kind, path));
            }
        }
        Ok(verified)
    }

    /// Atomically imports human-authored source profiles, groups, and interpretations.
    ///
    /// A batch is deliberately not a machine proposal channel. Suggested/verified
    /// state and `suggest:` authors are refused; candidates use the internal rule
    /// path and verification remains tied to the existing locator-aware review API.
    pub fn import_interpretations(&mut self, batch: &InterpretationBatch) -> Result<()> {
        self.require_case(&batch.case_id)?;
        for profile in &batch.source_profiles {
            require_human_batch(profile.review_state, &profile.created_by)?;
        }
        for group in &batch.content_groups {
            require_human_batch(group.review_state, &group.created_by)?;
        }
        for interpretation in &batch.interpretations {
            require_human_batch(interpretation.review_state, &interpretation.created_by)?;
        }

        let transaction = self.connection.transaction()?;
        for profile in &batch.source_profiles {
            insert_profile(&transaction, &batch.case_id, profile, false)?;
        }
        for group in &batch.content_groups {
            insert_group(&transaction, &batch.case_id, group)?;
        }
        for interpretation in &batch.interpretations {
            insert_interpretation(&transaction, &batch.case_id, interpretation, false)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Appends one human-authored source-profile version.
    pub fn append_source_profile(
        &mut self,
        case_id: &CaseId,
        profile: &ProposedSourceProfile,
    ) -> Result<SourceProfile> {
        self.require_case(case_id)?;
        require_human(profile.review_state, &profile.created_by)?;
        let transaction = self.connection.transaction()?;
        let id = insert_profile(&transaction, case_id, profile, false)?;
        transaction.commit()?;
        self.source_profile(case_id, &id)
    }

    /// Appends one deterministic source-profile candidate.
    #[allow(dead_code, reason = "used by the deterministic header-date rule")]
    pub(crate) fn append_suggested_source_profile(
        &mut self,
        case_id: &CaseId,
        profile: &ProposedSourceProfile,
    ) -> Result<SourceProfile> {
        self.require_case(case_id)?;
        let transaction = self.connection.transaction()?;
        let id = insert_profile(&transaction, case_id, profile, true)?;
        transaction.commit()?;
        self.source_profile(case_id, &id)
    }

    /// Appends one human-authored interpretation version.
    pub fn append_interpretation(
        &mut self,
        case_id: &CaseId,
        interpretation: &ProposedInterpretation,
    ) -> Result<ContentInterpretation> {
        self.require_case(case_id)?;
        require_human(interpretation.review_state, &interpretation.created_by)?;
        let transaction = self.connection.transaction()?;
        let id = insert_interpretation(&transaction, case_id, interpretation, false)?;
        transaction.commit()?;
        self.interpretation(case_id, &id)
    }

    /// Appends one versioned deterministic candidate.
    #[allow(
        dead_code,
        reason = "used by the deterministic enrichment-rule increment"
    )]
    pub(crate) fn append_suggested_interpretation(
        &mut self,
        case_id: &CaseId,
        interpretation: &ProposedInterpretation,
    ) -> Result<ContentInterpretation> {
        self.require_case(case_id)?;
        let transaction = self.connection.transaction()?;
        let id = insert_interpretation(&transaction, case_id, interpretation, true)?;
        transaction.commit()?;
        self.interpretation(case_id, &id)
    }

    /// Creates or revises one reviewer-defined ordered content group.
    pub fn append_content_group(
        &mut self,
        case_id: &CaseId,
        group: &ProposedContentGroup,
    ) -> Result<String> {
        self.require_case(case_id)?;
        require_human(group.review_state, &group.created_by)?;
        let transaction = self.connection.transaction()?;
        let id = insert_group(&transaction, case_id, group)?;
        transaction.commit()?;
        Ok(id)
    }

    /// Reads one source-profile version.
    pub fn source_profile(&self, case_id: &CaseId, id: &str) -> Result<SourceProfile> {
        self.require_case(case_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {PROFILE_COLUMNS} FROM source_profiles WHERE case_id = ?1 AND id = ?2"
                ),
                params![case_id.0, id],
                profile_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "source profile",
                id: id.to_owned(),
            })
    }

    /// Returns the current source profile, when one exists.
    pub fn current_source_profile(
        &self,
        case_id: &CaseId,
        source_id: &str,
    ) -> Result<Option<SourceProfile>> {
        self.require_case(case_id)?;
        self.connection
            .query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM current_source_profiles WHERE case_id = ?1 AND source_id = ?2 ORDER BY created_at DESC, id DESC LIMIT 1"),
                params![case_id.0, source_id],
                profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Reads one interpretation version.
    pub fn interpretation(&self, case_id: &CaseId, id: &str) -> Result<ContentInterpretation> {
        self.require_case(case_id)?;
        self.connection
            .query_row(
                &format!("SELECT {INTERPRETATION_COLUMNS} FROM content_interpretations WHERE case_id = ?1 AND id = ?2"),
                params![case_id.0, id],
                interpretation_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "interpretation",
                id: id.to_owned(),
            })
    }

    /// Returns all versions for a target/span in chronological order.
    pub fn interpretation_history(
        &self,
        case_id: &CaseId,
        target: &InterpretationTarget,
        char_start: Option<u32>,
        char_end: Option<u32>,
    ) -> Result<Vec<ContentInterpretation>> {
        self.require_case(case_id)?;
        let (content_id, group_id) = target_columns(target);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {INTERPRETATION_COLUMNS} FROM content_interpretations
             WHERE case_id = ?1 AND content_id IS ?2 AND content_group_id IS ?3
               AND char_start IS ?4 AND char_end IS ?5
             ORDER BY created_at, id"
        ))?;
        statement
            .query_map(
                params![case_id.0, content_id, group_id, char_start, char_end],
                interpretation_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Returns the current explicit interpretation for one target/span.
    pub fn current_interpretation(
        &self,
        case_id: &CaseId,
        target: &InterpretationTarget,
        char_start: Option<u32>,
        char_end: Option<u32>,
    ) -> Result<Option<ContentInterpretation>> {
        self.require_case(case_id)?;
        let (content_id, group_id) = target_columns(target);
        self.connection
            .query_row(
                &format!(
                    "SELECT {INTERPRETATION_COLUMNS} FROM current_content_interpretations
                     WHERE case_id = ?1 AND content_id IS ?2 AND content_group_id IS ?3
                       AND char_start IS ?4 AND char_end IS ?5
                     ORDER BY created_at DESC, id DESC LIMIT 1"
                ),
                params![case_id.0, content_id, group_id, char_start, char_end],
                interpretation_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Resolves an explicit whole-passage reading over reviewed source defaults.
    pub fn effective_interpretation(
        &self,
        case_id: &CaseId,
        target: &InterpretationTarget,
    ) -> Result<EffectiveInterpretation> {
        let explicit = self
            .current_interpretation(case_id, target, None, None)?
            .filter(|item| item.review_state != ReviewState::Rejected);
        let profile = match target {
            InterpretationTarget::Content { id } => {
                let source_id = self
                    .connection
                    .query_row(
                        "SELECT seg.source_id FROM content c
                         JOIN source_segments seg ON seg.id = c.segment_id
                         WHERE c.case_id = ?1 AND c.id = ?2",
                        params![case_id.0, id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                    .ok_or_else(|| Error::NotFound {
                        kind: "content",
                        id: id.clone(),
                    })?;
                self.current_source_profile(case_id, &source_id)?
                    .filter(|item| {
                        matches!(
                            item.review_state,
                            ReviewState::Reviewed | ReviewState::Verified
                        )
                    })
            }
            InterpretationTarget::ContentGroup { .. } => None,
        };

        let mut provenance = explicit
            .as_ref()
            .map_or_else(BTreeMap::new, |item| item.field_provenance.clone());
        let inherited = profile
            .as_ref()
            .map(|item| format!("inherited:{}", item.id));

        let content_form = explicit
            .as_ref()
            .and_then(|item| item.content_form)
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|item| item.default_content_form)
                    .inspect(|_| {
                        if let Some(value) = &inherited {
                            provenance.insert("content_form".to_owned(), value.clone());
                        }
                    })
            });
        let perception_basis = explicit
            .as_ref()
            .and_then(|item| item.perception_basis)
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|item| item.default_perception_basis)
                    .inspect(|_| {
                        if let Some(value) = &inherited {
                            provenance.insert("perception_basis".to_owned(), value.clone());
                        }
                    })
            });
        let temporal_stance = explicit
            .as_ref()
            .and_then(|item| item.temporal_stance)
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|item| item.default_temporal_stance)
                    .inspect(|_| {
                        if let Some(value) = &inherited {
                            provenance.insert("temporal_stance".to_owned(), value.clone());
                        }
                    })
            });
        let speaker_entity_id = explicit
            .as_ref()
            .and_then(|item| item.speaker_entity_id.clone())
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|item| item.author_entity_id.clone())
                    .inspect(|_| {
                        if let Some(value) = &inherited {
                            provenance.insert("speaker_entity_id".to_owned(), value.clone());
                        }
                    })
            });
        let content_created_at = explicit
            .as_ref()
            .and_then(|item| item.content_created_at.clone())
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|item| item.created_at_claim.clone())
                    .inspect(|_| {
                        if let Some(value) = &inherited {
                            provenance.insert("content_created_at".to_owned(), value.clone());
                        }
                    })
            });

        Ok(EffectiveInterpretation {
            explicit,
            content_form,
            perception_basis,
            temporal_stance,
            speaker_entity_id,
            content_created_at,
            field_provenance: provenance,
        })
    }
}

fn require_human(state: ReviewState, author: &str) -> Result<()> {
    if author.trim().is_empty() || author.trim().starts_with("suggest:") {
        return Err(Error::InvalidInterpretation(
            "human interpretation must name a person and may not use the suggest: namespace"
                .to_owned(),
        ));
    }
    if state == ReviewState::Suggested {
        return Err(Error::InvalidInterpretation(
            "a person cannot write suggested state".to_owned(),
        ));
    }
    Ok(())
}

fn require_human_batch(state: ReviewState, author: &str) -> Result<()> {
    require_human(state, author)?;
    if state == ReviewState::Verified {
        return Err(Error::InvalidInterpretation(
            "batch import cannot verify without the original-locator review path".to_owned(),
        ));
    }
    Ok(())
}

fn validate_rule(state: ReviewState, author: &str, allow_suggest: bool) -> Result<()> {
    if allow_suggest {
        if state != ReviewState::Suggested || !author.starts_with("suggest:") {
            return Err(Error::InvalidInterpretation(
                "a deterministic candidate must enter suggested under suggest:<rule>@<version>"
                    .to_owned(),
            ));
        }
        Ok(())
    } else {
        require_human(state, author)
    }
}

fn insert_profile(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    profile: &ProposedSourceProfile,
    allow_suggest: bool,
) -> Result<String> {
    validate_rule(
        profile.review_state,
        profile.created_by.trim(),
        allow_suggest,
    )?;
    require_node_case(
        transaction,
        "sources",
        &profile.source_id,
        case_id,
        "source",
    )?;
    validate_optional_entity(transaction, case_id, profile.author_entity_id.as_deref())?;
    if profile.clock_offset_ms.is_some() && !has_text(profile.clock_offset_basis.as_deref()) {
        return Err(Error::InvalidInterpretation(
            "a source clock offset requires a written basis".to_owned(),
        ));
    }
    if let Some(previous) = profile.supersedes_profile_id.as_deref() {
        require_current_profile(transaction, case_id, previous, &profile.source_id)?;
    } else if transaction
        .query_row(
            "SELECT 1 FROM current_source_profiles WHERE case_id = ?1 AND source_id = ?2 LIMIT 1",
            params![case_id.0, profile.source_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Err(Error::InvalidInterpretation(format!(
            "source `{}` already has a current profile; supersede it explicitly",
            profile.source_id
        )));
    }
    let id = supplied_or_uuid(
        transaction,
        "source_profiles",
        profile.id.as_deref(),
        "source profile",
    )?;
    transaction.execute(
        "INSERT INTO source_profiles
         (id, case_id, source_id, source_role, author_entity_id, created_at_claim,
          default_content_form, default_temporal_stance, default_perception_basis,
          clock_offset_ms, clock_offset_basis, review_state, created_by,
          supersedes_profile_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            case_id.0,
            profile.source_id,
            profile.source_role.as_str(),
            profile.author_entity_id,
            clean(profile.created_at_claim.as_deref()),
            profile.default_content_form.map(ContentForm::as_str),
            profile.default_temporal_stance.map(TemporalStance::as_str),
            profile
                .default_perception_basis
                .map(PerceptionBasis::as_str),
            profile.clock_offset_ms,
            clean(profile.clock_offset_basis.as_deref()),
            profile.review_state.as_str(),
            profile.created_by.trim(),
            profile.supersedes_profile_id,
        ],
    )?;
    Ok(id)
}

fn insert_group(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    group: &ProposedContentGroup,
) -> Result<String> {
    require_human(group.review_state, &group.created_by)?;
    if group.content_ids.len() < 2 {
        return Err(Error::InvalidInterpretation(
            "a content group needs at least two passages".to_owned(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for content_id in &group.content_ids {
        if !seen.insert(content_id) {
            return Err(Error::InvalidInterpretation(format!(
                "content `{content_id}` appears twice in one group"
            )));
        }
        require_node_case(transaction, "content", content_id, case_id, "content")?;
    }
    if let Some(previous) = group.supersedes_group_id.as_deref() {
        require_current_row(
            transaction,
            "content_groups",
            "supersedes_group_id",
            case_id,
            previous,
            "content group",
        )?;
    }
    let id = supplied_or_uuid(
        transaction,
        "content_groups",
        group.id.as_deref(),
        "content group",
    )?;
    transaction.execute(
        "INSERT INTO content_groups
         (id, case_id, label, review_state, created_by, supersedes_group_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            id,
            case_id.0,
            clean(group.label.as_deref()),
            group.review_state.as_str(),
            group.created_by.trim(),
            group.supersedes_group_id,
        ],
    )?;
    for (ordinal, content_id) in group.content_ids.iter().enumerate() {
        let ordinal = i64::try_from(ordinal)
            .map_err(|_| Error::InvalidInterpretation("content group is too large".to_owned()))?;
        transaction.execute(
            "INSERT INTO content_group_members (group_id, content_id, ordinal) VALUES (?1,?2,?3)",
            params![id, content_id, ordinal],
        )?;
    }
    Ok(id)
}

fn insert_interpretation(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    item: &ProposedInterpretation,
    allow_suggest: bool,
) -> Result<String> {
    validate_rule(item.review_state, item.created_by.trim(), allow_suggest)?;
    validate_target(transaction, case_id, &item.target)?;
    validate_span(transaction, &item.target, item.char_start, item.char_end)?;
    for entity in [
        item.speaker_entity_id.as_deref(),
        item.attributed_entity_id.as_deref(),
        item.location_entity_id.as_deref(),
    ] {
        validate_optional_entity(transaction, case_id, entity)?;
    }
    if let Some(parent) = item.reporting_parent_interpretation_id.as_deref() {
        require_node_case(
            transaction,
            "content_interpretations",
            parent,
            case_id,
            "interpretation",
        )?;
    }
    if item.normalized_start.is_some() && !has_text(item.time_alignment_basis.as_deref()) {
        return Err(Error::InvalidInterpretation(
            "normalized case time requires a written alignment basis".to_owned(),
        ));
    }
    if let (Some(start), Some(end)) = (item.asserted_start.as_deref(), item.asserted_end.as_deref())
        && end < start
    {
        return Err(Error::InvalidInterpretation(
            "asserted interval ends before it starts".to_owned(),
        ));
    }
    if let (Some(start), Some(end)) = (
        item.normalized_start.as_deref(),
        item.normalized_end.as_deref(),
    ) && end < start
    {
        return Err(Error::InvalidInterpretation(
            "normalized interval ends before it starts".to_owned(),
        ));
    }
    if let Some(previous) = item.supersedes_interpretation_id.as_deref() {
        require_current_interpretation(transaction, case_id, previous, item)?;
    } else {
        let (content_id, group_id) = target_columns(&item.target);
        if transaction
            .query_row(
                "SELECT 1 FROM current_content_interpretations
                 WHERE case_id = ?1 AND content_id IS ?2 AND content_group_id IS ?3
                   AND char_start IS ?4 AND char_end IS ?5 LIMIT 1",
                params![
                    case_id.0,
                    content_id,
                    group_id,
                    item.char_start,
                    item.char_end
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(Error::InvalidInterpretation(format!(
                "{} `{}` already has a current interpretation for this span; supersede it explicitly",
                match &item.target {
                    InterpretationTarget::Content { .. } => "content",
                    InterpretationTarget::ContentGroup { .. } => "content group",
                },
                item.target.id()
            )));
        }
    }
    let id = supplied_or_uuid(
        transaction,
        "content_interpretations",
        item.id.as_deref(),
        "interpretation",
    )?;
    let (content_id, group_id) = target_columns(&item.target);
    let provenance = serde_json::to_string(&item.field_provenance)?;
    transaction.execute(
        "INSERT INTO content_interpretations
         (id, case_id, content_id, content_group_id, char_start, char_end,
          content_form, perception_basis, temporal_stance, speaker_entity_id,
          attributed_entity_id, reporting_parent_interpretation_id,
          content_created_at, asserted_start, asserted_end, normalized_start,
          normalized_end, time_alignment_basis, location_text, location_entity_id,
          materiality, field_provenance_json, basis, review_state, created_by,
          supersedes_interpretation_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,
                 ?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
        params![
            id,
            case_id.0,
            content_id,
            group_id,
            item.char_start,
            item.char_end,
            item.content_form.map(ContentForm::as_str),
            item.perception_basis.map(PerceptionBasis::as_str),
            item.temporal_stance.map(TemporalStance::as_str),
            item.speaker_entity_id,
            item.attributed_entity_id,
            item.reporting_parent_interpretation_id,
            clean(item.content_created_at.as_deref()),
            clean(item.asserted_start.as_deref()),
            clean(item.asserted_end.as_deref()),
            clean(item.normalized_start.as_deref()),
            clean(item.normalized_end.as_deref()),
            clean(item.time_alignment_basis.as_deref()),
            clean(item.location_text.as_deref()),
            item.location_entity_id,
            item.materiality.as_str(),
            provenance,
            clean(item.basis.as_deref()),
            item.review_state.as_str(),
            item.created_by.trim(),
            item.supersedes_interpretation_id,
        ],
    )?;
    Ok(id)
}

fn validate_target(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    target: &InterpretationTarget,
) -> Result<()> {
    match target {
        InterpretationTarget::Content { id } => {
            require_node_case(transaction, "content", id, case_id, "content")
        }
        InterpretationTarget::ContentGroup { id } => {
            require_node_case(transaction, "content_groups", id, case_id, "content group")
        }
    }
}

fn validate_span(
    transaction: &Transaction<'_>,
    target: &InterpretationTarget,
    start: Option<u32>,
    end: Option<u32>,
) -> Result<()> {
    match (target, start, end) {
        (_, None, None) => Ok(()),
        (InterpretationTarget::Content { id }, Some(start), Some(end)) if end > start => {
            let length: i64 = transaction.query_row(
                "SELECT length(CAST(text AS BLOB)) FROM content WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            if i64::from(end) > length {
                return Err(Error::InvalidInterpretation(format!(
                    "span {start}..{end} exceeds content `{id}` byte length {length}"
                )));
            }
            Ok(())
        }
        (InterpretationTarget::ContentGroup { .. }, Some(_), Some(_)) => {
            Err(Error::InvalidInterpretation(
                "a content group cannot carry a character span".to_owned(),
            ))
        }
        _ => Err(Error::InvalidInterpretation(
            "character span requires both start and end, with end after start".to_owned(),
        )),
    }
}

fn require_current_profile(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    previous: &str,
    source_id: &str,
) -> Result<()> {
    let previous_source: String = transaction
        .query_row(
            "SELECT source_id FROM source_profiles WHERE case_id = ?1 AND id = ?2",
            params![case_id.0, previous],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound {
            kind: "source profile",
            id: previous.to_owned(),
        })?;
    if previous_source != source_id {
        return Err(Error::InvalidInterpretation(
            "a source-profile revision must describe the same source".to_owned(),
        ));
    }
    require_not_superseded(
        transaction,
        "source_profiles",
        "supersedes_profile_id",
        previous,
        "source profile",
    )
}

fn require_current_interpretation(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    previous: &str,
    item: &ProposedInterpretation,
) -> Result<()> {
    let previous_target: (Option<String>, Option<String>, Option<u32>, Option<u32>) = transaction
        .query_row(
            "SELECT content_id, content_group_id, char_start, char_end
             FROM content_interpretations WHERE case_id = ?1 AND id = ?2",
            params![case_id.0, previous],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound {
            kind: "interpretation",
            id: previous.to_owned(),
        })?;
    let (content_id, group_id) = target_columns(&item.target);
    if previous_target
        != (
            content_id.map(str::to_owned),
            group_id.map(str::to_owned),
            item.char_start,
            item.char_end,
        )
    {
        return Err(Error::InvalidInterpretation(
            "an interpretation revision must keep the same target and span".to_owned(),
        ));
    }
    require_not_superseded(
        transaction,
        "content_interpretations",
        "supersedes_interpretation_id",
        previous,
        "interpretation",
    )
}

fn require_current_row(
    transaction: &Transaction<'_>,
    table: &str,
    successor_column: &str,
    case_id: &CaseId,
    previous: &str,
    kind: &'static str,
) -> Result<()> {
    require_node_case(transaction, table, previous, case_id, kind)?;
    require_not_superseded(transaction, table, successor_column, previous, kind)
}

fn require_not_superseded(
    transaction: &Transaction<'_>,
    table: &str,
    successor_column: &str,
    previous: &str,
    kind: &'static str,
) -> Result<()> {
    let successor = transaction
        .query_row(
            &format!("SELECT id FROM {table} WHERE {successor_column} = ?1"),
            [previous],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if successor.is_some() {
        return Err(Error::Superseded {
            kind,
            id: previous.to_owned(),
            by: successor,
        });
    }
    Ok(())
}

fn validate_optional_entity(
    transaction: &Transaction<'_>,
    case_id: &CaseId,
    entity_id: Option<&str>,
) -> Result<()> {
    if let Some(entity_id) = entity_id {
        require_node_case(transaction, "entities", entity_id, case_id, "entity")?;
    }
    Ok(())
}

fn require_node_case(
    transaction: &Transaction<'_>,
    table: &str,
    id: &str,
    case_id: &CaseId,
    kind: &'static str,
) -> Result<()> {
    let owner = transaction
        .query_row(
            &format!("SELECT case_id FROM {table} WHERE id = ?1"),
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    match owner {
        Some(owner) if owner == case_id.0 => Ok(()),
        Some(_) => Err(Error::WrongCase {
            kind,
            id: id.to_owned(),
        }),
        None => Err(Error::NotFound {
            kind,
            id: id.to_owned(),
        }),
    }
}

fn supplied_or_uuid(
    transaction: &Transaction<'_>,
    table: &str,
    supplied: Option<&str>,
    kind: &'static str,
) -> Result<String> {
    let id = supplied
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
    let exists = transaction
        .query_row(
            &format!("SELECT 1 FROM {table} WHERE id = ?1"),
            [&id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Err(Error::AlreadyExists { kind, id });
    }
    Ok(id)
}

fn target_columns(target: &InterpretationTarget) -> (Option<&str>, Option<&str>) {
    match target {
        InterpretationTarget::Content { id } => (Some(id), None),
        InterpretationTarget::ContentGroup { id } => (None, Some(id)),
    }
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn has_text(value: Option<&str>) -> bool {
    clean(value).is_some()
}

fn profile_from_row(row: &Row<'_>) -> rusqlite::Result<SourceProfile> {
    Ok(SourceProfile {
        id: row.get(0)?,
        source_id: row.get(1)?,
        source_role: enum_value(row, 2, SourceRole::from_db, "source role")?,
        author_entity_id: row.get(3)?,
        created_at_claim: row.get(4)?,
        default_content_form: optional_enum(row, 5, ContentForm::from_db, "content form")?,
        default_temporal_stance: optional_enum(row, 6, TemporalStance::from_db, "temporal stance")?,
        default_perception_basis: optional_enum(
            row,
            7,
            PerceptionBasis::from_db,
            "perception basis",
        )?,
        clock_offset_ms: row.get(8)?,
        clock_offset_basis: row.get(9)?,
        review_state: enum_value(row, 10, ReviewState::from_db, "review state")?,
        created_by: row.get(11)?,
        supersedes_profile_id: row.get(12)?,
        created_at: row.get(13)?,
    })
}

fn interpretation_from_row(row: &Row<'_>) -> rusqlite::Result<ContentInterpretation> {
    let content_id: Option<String> = row.get(1)?;
    let group_id: Option<String> = row.get(2)?;
    let target = match (content_id, group_id) {
        (Some(id), None) => InterpretationTarget::Content { id },
        (None, Some(id)) => InterpretationTarget::ContentGroup { id },
        _ => return Err(conversion_error(1, "interpretation target")),
    };
    let provenance_json: String = row.get(20)?;
    let field_provenance = serde_json::from_str(&provenance_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(20, Type::Text, Box::new(error))
    })?;
    Ok(ContentInterpretation {
        id: row.get(0)?,
        target,
        char_start: row.get(3)?,
        char_end: row.get(4)?,
        content_form: optional_enum(row, 5, ContentForm::from_db, "content form")?,
        perception_basis: optional_enum(row, 6, PerceptionBasis::from_db, "perception basis")?,
        temporal_stance: optional_enum(row, 7, TemporalStance::from_db, "temporal stance")?,
        speaker_entity_id: row.get(8)?,
        attributed_entity_id: row.get(9)?,
        reporting_parent_interpretation_id: row.get(10)?,
        content_created_at: row.get(11)?,
        asserted_start: row.get(12)?,
        asserted_end: row.get(13)?,
        normalized_start: row.get(14)?,
        normalized_end: row.get(15)?,
        time_alignment_basis: row.get(16)?,
        location_text: row.get(17)?,
        location_entity_id: row.get(18)?,
        materiality: enum_value(row, 19, Materiality::from_db, "materiality")?,
        field_provenance,
        basis: row.get(21)?,
        review_state: enum_value(row, 22, ReviewState::from_db, "review state")?,
        created_by: row.get(23)?,
        supersedes_interpretation_id: row.get(24)?,
        created_at: row.get(25)?,
    })
}

fn enum_value<T>(
    row: &Row<'_>,
    index: usize,
    parse: impl FnOnce(&str) -> Option<T>,
    label: &'static str,
) -> rusqlite::Result<T> {
    let value: String = row.get(index)?;
    parse(&value).ok_or_else(|| conversion_error(index, label))
}

fn optional_enum<T>(
    row: &Row<'_>,
    index: usize,
    parse: impl FnOnce(&str) -> Option<T>,
    label: &'static str,
) -> rusqlite::Result<Option<T>> {
    let value: Option<String> = row.get(index)?;
    value
        .map(|value| parse(&value).ok_or_else(|| conversion_error(index, label)))
        .transpose()
}

fn conversion_error(index: usize, label: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown {label}"),
        )),
    )
}

fn sql_u64(value: Option<i64>, index: usize) -> rusqlite::Result<Option<u64>> {
    value
        .map(|value| u64::try_from(value).map_err(|_| conversion_error(index, "time offset")))
        .transpose()
}

fn artifact_paths<'a>(
    artifacts: &'a [(String, PathBuf)],
    kind: &'a str,
) -> impl Iterator<Item = &'a PathBuf> {
    artifacts
        .iter()
        .filter(move |(candidate, _)| candidate == kind)
        .map(|(_, path)| path)
}

fn select_page_preview(artifacts: &[(String, PathBuf)], page: u32) -> Option<PathBuf> {
    let needles = [
        format!("page-{page:04}"),
        format!("page_{page:04}"),
        format!("page-{page}"),
    ];
    artifact_paths(artifacts, "page_preview")
        .find(|path| {
            let text = path.to_string_lossy().to_ascii_lowercase();
            needles.iter().any(|needle| text.contains(needle))
        })
        .cloned()
        .or_else(|| artifact_paths(artifacts, "page_preview").next().cloned())
}

#[allow(dead_code)]
fn _node_kind_contract(target: &InterpretationTarget) -> NodeKind {
    match target {
        InterpretationTarget::Content { .. } => NodeKind::Content,
        InterpretationTarget::ContentGroup { .. } => NodeKind::ContentGroup,
    }
}
