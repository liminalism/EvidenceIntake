//! Reviewed provenance traversal and structural counts.

use std::collections::{HashSet, VecDeque};

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::Store;
use crate::{
    AuthoredLink, AuthoredOccurrence, CaseId, EdgeKind, Error, Lineage, NodeKind, NodeRef,
    ProposedOccurrence, Result, StructuralCount,
};

impl Store {
    /// Follows reviewed descriptive dependency edges to their roots.
    pub fn lineage(&self, case_id: &CaseId, node: &NodeRef) -> Result<Lineage> {
        self.require_case(case_id)?;
        self.require_node(case_id, node)?;
        let mut queue = VecDeque::from([node.clone()]);
        let mut seen: HashSet<(NodeKind, String)> = HashSet::new();
        let mut members = Vec::new();
        let mut roots = Vec::new();

        while let Some(current) = queue.pop_front() {
            if !seen.insert((current.kind, current.id.clone())) {
                continue;
            }
            members.push(current.clone());
            let parents = self.lineage_parents(case_id, &current)?;
            if parents.is_empty() {
                roots.push(current);
            } else {
                queue.extend(parents);
            }
        }
        members.sort_by(node_order);
        roots.sort_by(node_order);
        roots.dedup();
        Ok(Lineage {
            node: node.clone(),
            members,
            roots,
        })
    }

    /// Computes non-evaluative original/lineage/dependency counts for nodes.
    pub fn structural_count(&self, case_id: &CaseId, nodes: &[NodeRef]) -> Result<StructuralCount> {
        let mut originals = HashSet::new();
        let mut roots = HashSet::new();
        let mut without = 0_u32;
        for node in nodes {
            let lineage = self.lineage(case_id, node)?;
            if !self.has_reviewed_dependency(case_id, node)? {
                without = without.saturating_add(1);
            }
            originals.extend(self.immediate_originals(case_id, node)?);
            for root in lineage.roots {
                roots.insert((root.kind, root.id));
            }
        }
        Ok(StructuralCount {
            original_files: u32::try_from(originals.len()).unwrap_or(u32::MAX),
            reporting_lineages: u32::try_from(roots.len()).unwrap_or(u32::MAX),
            without_reviewed_dependency: without,
        })
    }

    fn lineage_parents(&self, case_id: &CaseId, node: &NodeRef) -> Result<Vec<NodeRef>> {
        let mut parents = Vec::new();
        if node.kind == NodeKind::ContentGroup {
            let mut statement = self.connection.prepare(
                "SELECT member.content_id FROM content_group_members member
                 JOIN content_groups grouped ON grouped.id = member.group_id
                 WHERE grouped.case_id = ?1 AND grouped.id = ?2
                 ORDER BY member.ordinal",
            )?;
            parents.extend(
                statement
                    .query_map(params![case_id.0, node.id], |row| {
                        Ok(NodeRef::new(NodeKind::Content, row.get::<_, String>(0)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }

        let mut statement = self.connection.prepare(
            "SELECT target_kind, target_id FROM edges
             WHERE case_id = ?1 AND source_kind = ?2 AND source_id = ?3
               AND review_state IN ('reviewed','verified')
               AND relation IN ('quotes','reports','summarizes','transcribes','based_on',
                                'derived_from','records_utterance')
             ORDER BY target_kind, target_id",
        )?;
        let edge_parents = statement
            .query_map(params![case_id.0, node.kind.as_str(), node.id], |row| {
                let kind: String = row.get(0)?;
                let kind = NodeKind::from_db(&kind).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "unknown edge node kind",
                        )),
                    )
                })?;
                Ok(NodeRef::new(kind, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        parents.extend(edge_parents);
        // The immutable original is the root only when no reviewed reporting
        // or derivation dependency says this passage rests on something else.
        if node.kind == NodeKind::Content && parents.is_empty() {
            let source_id = self
                .connection
                .query_row(
                    "SELECT seg.source_id FROM content c
                     JOIN source_segments seg ON seg.id = c.segment_id
                     WHERE c.case_id = ?1 AND c.id = ?2",
                    params![case_id.0, node.id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(source_id) = source_id {
                parents.push(NodeRef::new(NodeKind::Source, source_id));
            }
        }
        parents.sort_by(node_order);
        parents.dedup();
        Ok(parents)
    }

    fn has_reviewed_dependency(&self, case_id: &CaseId, node: &NodeRef) -> Result<bool> {
        if node.kind == NodeKind::ContentGroup {
            return Ok(true);
        }
        self.connection
            .query_row(
                "SELECT 1 FROM edges
                 WHERE case_id = ?1 AND source_kind = ?2 AND source_id = ?3
                   AND review_state IN ('reviewed','verified')
                   AND relation IN ('quotes','reports','summarizes','transcribes','based_on',
                                    'derived_from','records_utterance')
                 LIMIT 1",
                params![case_id.0, node.kind.as_str(), node.id],
                |_| Ok(true),
            )
            .optional()
            .map(|found| found.unwrap_or(false))
            .map_err(Into::into)
    }

    fn immediate_originals(&self, case_id: &CaseId, node: &NodeRef) -> Result<Vec<String>> {
        match node.kind {
            NodeKind::Source => Ok(vec![node.id.clone()]),
            NodeKind::Content => self
                .connection
                .query_row(
                    "SELECT seg.source_id FROM content c
                     JOIN source_segments seg ON seg.id = c.segment_id
                     WHERE c.case_id = ?1 AND c.id = ?2",
                    params![case_id.0, node.id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map(|source| source.into_iter().collect())
                .map_err(Into::into),
            NodeKind::ContentGroup => {
                let mut statement = self.connection.prepare(
                    "SELECT DISTINCT seg.source_id
                     FROM content_group_members member
                     JOIN content_groups grouped ON grouped.id = member.group_id
                     JOIN content c ON c.id = member.content_id
                     JOIN source_segments seg ON seg.id = c.segment_id
                     WHERE grouped.case_id = ?1 AND grouped.id = ?2
                     ORDER BY seg.source_id",
                )?;
                statement
                    .query_map(params![case_id.0, node.id], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Assembles reviewed accounts of one occurrence into a single event.
    ///
    /// This is the step no adapter can take. Two passages describing the same
    /// twenty seconds are not the same record, and nothing in the extraction
    /// says they belong together: a person decides that, in writing. So the
    /// event and every `account_of` edge enter `unreviewed` like any other
    /// authored claim, the accounts stay separate rows in their own lanes, and
    /// the timeline still shows them as competing accounts rather than one
    /// settled sequence.
    ///
    /// A same-occurrence candidate a reviewer has already rejected is refused:
    /// grouping two passages a person has just said do not belong together
    /// would overwrite that decision with silence.
    pub fn author_occurrence(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedOccurrence,
    ) -> Result<AuthoredOccurrence> {
        self.require_case(case_id)?;
        let author = super::require_named_person(&proposal.author)?.to_owned();
        let label = super::require_text(&proposal.label, "an occurrence must have a label")?;
        let rationale = super::require_text(
            &proposal.rationale,
            "grouping accounts into one occurrence requires a written reason",
        )?;
        if proposal.passage_ids.len() < 2 {
            return Err(Error::InvalidAuthoring(
                "an occurrence assembles at least two accounts; one passage is already itself"
                    .to_owned(),
            ));
        }
        let mut unique = proposal.passage_ids.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != proposal.passage_ids.len() {
            return Err(Error::InvalidAuthoring(
                "the same passage was listed twice as an account of the occurrence".to_owned(),
            ));
        }
        if (proposal.normalized_start.is_some() || proposal.normalized_end.is_some())
            && super::trimmed(proposal.time_basis.as_deref()).is_none()
        {
            return Err(Error::InvalidAuthoring(
                "a normalized occurrence time is a reviewable hypothesis and requires a written \
                 alignment basis"
                    .to_owned(),
            ));
        }
        for passage in &proposal.passage_ids {
            self.require_node(
                case_id,
                &NodeRef {
                    kind: NodeKind::Content,
                    id: passage.clone(),
                },
            )?;
        }
        if let Some(proposition) = proposal.proposition_id.as_deref() {
            self.require_node(
                case_id,
                &NodeRef {
                    kind: NodeKind::Proposition,
                    id: proposition.to_owned(),
                },
            )?;
        }
        self.refuse_rejected_same_occurrence(case_id, &proposal.passage_ids)?;

        let event_id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_id(NodeKind::Event, supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };
        let edge_ids = proposal
            .passage_ids
            .iter()
            .map(|_| Uuid::now_v7().to_string())
            .collect::<Vec<_>>();

        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO events
               (id, case_id, label, lane, raw_time, normalized_start, normalized_end,
                time_basis, location_text, proposition_id, review_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'unreviewed')",
            params![
                event_id,
                case_id.0,
                label,
                proposal.lane.as_str(),
                super::trimmed(proposal.raw_time.as_deref()),
                super::trimmed(proposal.normalized_start.as_deref()),
                super::trimmed(proposal.normalized_end.as_deref()),
                super::trimmed(proposal.time_basis.as_deref()),
                super::trimmed(proposal.location_text.as_deref()),
                proposal.proposition_id.as_deref(),
            ],
        )?;
        for (edge_id, passage) in edge_ids.iter().zip(&proposal.passage_ids) {
            let written = transaction.execute(
                "INSERT INTO edges
                   (id, case_id, source_kind, source_id, relation, target_kind, target_id,
                    rationale, review_state, created_by)
                 VALUES (?1, ?2, 'content', ?3, 'account_of', 'event', ?4, ?5, 'unreviewed', ?6)
                 ON CONFLICT (case_id, source_kind, source_id, relation, target_kind, target_id)
                   DO NOTHING",
                params![edge_id, case_id.0, passage, event_id, rationale, author],
            )?;
            if written == 0 {
                return Err(Error::AlreadyExists {
                    kind: "relationship",
                    id: format!("{passage} account_of {event_id}"),
                });
            }
        }
        transaction.commit()?;

        Ok(AuthoredOccurrence {
            event_id: event_id.clone(),
            account_links: edge_ids
                .into_iter()
                .zip(&proposal.passage_ids)
                .map(|(id, passage)| AuthoredLink {
                    id,
                    from_kind: NodeKind::Content.as_str().to_owned(),
                    from_id: passage.clone(),
                    relation: EdgeKind::AccountOf.as_str().to_owned(),
                    to_kind: NodeKind::Event.as_str().to_owned(),
                    to_id: event_id.clone(),
                    rationale: rationale.clone(),
                    review_state: "unreviewed".to_owned(),
                    created_by: author.clone(),
                })
                .collect(),
        })
    }

    /// Refuses an occurrence over a pair a reviewer has already separated.
    fn refuse_rejected_same_occurrence(&self, case_id: &CaseId, passages: &[String]) -> Result<()> {
        let mut statement = self.connection.prepare_cached(
            "SELECT 1 FROM edges
             WHERE case_id = ?1 AND relation = 'candidate_same_occurrence'
               AND review_state = 'rejected'
               AND source_kind = 'content' AND target_kind = 'content'
               AND source_id = ?2 AND target_id = ?3
             LIMIT 1",
        )?;
        for (index, left) in passages.iter().enumerate() {
            for right in &passages[index + 1..] {
                for (from, to) in [(left, right), (right, left)] {
                    if statement
                        .query_row(params![case_id.0, from, to], |_| Ok(()))
                        .optional()?
                        .is_some()
                    {
                        return Err(Error::InvalidAuthoring(format!(
                            "a reviewer rejected the same-occurrence candidate between `{left}` \
                             and `{right}`; reopen that decision before grouping them"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn node_order(left: &NodeRef, right: &NodeRef) -> std::cmp::Ordering {
    left.kind
        .as_str()
        .cmp(right.kind.as_str())
        .then_with(|| left.id.cmp(&right.id))
}
