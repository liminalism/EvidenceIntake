//! Deterministic, typed-template case digest.

use rusqlite::OptionalExtension;

use super::Store;
use crate::{
    CaseDigest, CaseId, DigestLocator, DigestSection, DigestSentence, Error, ExportAudience,
    OmittedDigestTemplate, PacketItem, PropositionPacket, Result,
};

#[derive(Debug)]
enum Template<'a> {
    RecordedCapture {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    ContemporaneousAccount {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    LaterAccount {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    OfficialAssertion {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    PropositionLink {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    DependencyOrConflict {
        proposition: &'a str,
        item: &'a PacketItem,
    },
    MissingOrUnreviewed {
        proposition: &'a str,
        item: &'a PacketItem,
    },
}

impl Template<'_> {
    fn id(&self) -> &'static str {
        match self {
            Self::RecordedCapture { .. } => "recorded-capture@1",
            Self::ContemporaneousAccount { .. } => "contemporaneous-account@1",
            Self::LaterAccount { .. } => "later-account@1",
            Self::OfficialAssertion { .. } => "official-assertion@1",
            Self::PropositionLink { .. } => "proposition-link@1",
            Self::DependencyOrConflict { .. } => "dependency-or-conflict@1",
            Self::MissingOrUnreviewed { .. } => "missing-or-unreviewed@1",
        }
    }

    fn render(&self) -> std::result::Result<DigestSentence, String> {
        let (proposition, item) = match self {
            Self::RecordedCapture { proposition, item }
            | Self::ContemporaneousAccount { proposition, item }
            | Self::LaterAccount { proposition, item }
            | Self::OfficialAssertion { proposition, item }
            | Self::PropositionLink { proposition, item }
            | Self::DependencyOrConflict { proposition, item }
            | Self::MissingOrUnreviewed { proposition, item } => (*proposition, *item),
        };
        if item.original.trim().is_empty() || item.locator.trim().is_empty() {
            return Err("the linked passage has no exact original locator".to_owned());
        }

        let time_labels = time_labels(item);
        let text = match self {
            Self::RecordedCapture { .. }
                if item.content_form.as_deref() == Some("no_speech_aligned") =>
            {
                format!(
                    "{} at {} records an ASR-aligned no-speech interval. This does not establish recording loss. {time_labels}",
                    item.original, item.locator
                )
            }
            Self::RecordedCapture { .. }
                if item.content_form.as_deref() == Some("visual_observation") =>
            {
                format!(
                    "Within the camera's field of view, {} at {} contains: {} {time_labels}",
                    item.original, item.locator, item.text
                )
            }
            Self::RecordedCapture { .. } => format!(
                "{} at {} directly captures material linked to “{}”: {} {time_labels}",
                item.original, item.locator, proposition, item.text
            ),
            Self::ContemporaneousAccount { .. } => format!(
                "{} at {} contains a contemporaneous account linked to “{}”: {} {time_labels}",
                item.original, item.locator, proposition, item.text
            ),
            Self::LaterAccount { .. } => format!(
                "{} at {} contains a later account linked to “{}”: {} {time_labels}",
                item.original, item.locator, proposition, item.text
            ),
            Self::OfficialAssertion { .. } => format!(
                "{} at {} characterizes or asserts, without adoption by this digest: {} {time_labels}",
                item.original, item.locator, item.text
            ),
            Self::PropositionLink { .. } => format!(
                "{} at {} is linked as `{}` to the contested proposition “{}”; the link is {}.",
                item.original, item.locator, item.relation, proposition, item.relation_state
            ),
            Self::DependencyOrConflict { .. } => {
                let roots = item
                    .lineage_roots
                    .iter()
                    .map(|root| format!("{}:{}", root.kind.as_str(), root.id))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} at {} carries a reviewed reporting/derivation root [{}] while bearing on “{}”. {time_labels}",
                    item.original, item.locator, roots, proposition
                )
            }
            Self::MissingOrUnreviewed { .. } => format!(
                "{} at {} remains in interpretation state `{}` with proposition-link state `{}` for “{}”. {time_labels}",
                item.original,
                item.locator,
                item.interpretation_state,
                item.relation_state,
                proposition
            ),
        };
        Ok(DigestSentence {
            template_id: self.id().to_owned(),
            text,
            locators: vec![DigestLocator {
                original: item.original.clone(),
                locator: item.locator.clone(),
                content_id: item.content_id.clone(),
            }],
        })
    }
}

impl Store {
    /// Renders the nine fixed digest layers from reviewed packet records.
    ///
    /// The disclosable path never calls a privileged-table reader. Every
    /// factual sentence is rendered through [`Template`] and the result is
    /// validated before it leaves this method.
    pub fn case_digest(&self, case_id: &CaseId, audience: ExportAudience) -> Result<CaseDigest> {
        let case_name = self.case_name(case_id)?;
        let mut sections = vec![
            DigestSection {
                key: "posture_elements".to_owned(),
                title: "Case posture and elements".to_owned(),
                sentences: Vec::new(),
                analysis: vec![format!("Case: {case_name}")],
                omitted: Vec::new(),
            },
            blank_section("recorded_sequence", "Recorded sequence"),
            blank_section("contemporaneous_accounts", "Contemporaneous accounts"),
            blank_section("later_accounts", "Later accounts"),
            blank_section("official_narrative", "Official documentary narrative"),
            blank_section("contested_propositions", "Contested proposition packets"),
            blank_section(
                "dependencies_conflicts",
                "Source dependencies and unresolved conflicts",
            ),
            blank_section("missing_unreviewed", "Missing and unreviewed material"),
            blank_section("privileged_issues", "Privileged issues and decisions"),
        ];

        let proposition_ids = self.digest_proposition_ids(case_id)?;
        for proposition_id in proposition_ids {
            let packet = self.proposition_packet(case_id, &proposition_id)?;
            populate_from_packet(&packet, &mut sections);
        }

        let template_sections = [
            (1, "recorded-capture@1"),
            (2, "contemporaneous-account@1"),
            (3, "later-account@1"),
            (4, "official-assertion@1"),
            (5, "proposition-link@1"),
            (6, "dependency-or-conflict@1"),
            (7, "missing-or-unreviewed@1"),
        ];
        for (index, template_id) in template_sections {
            if sections[index].sentences.is_empty() {
                sections[index].omitted.push(OmittedDigestTemplate {
                    template_id: template_id.to_owned(),
                    reason: "no source-linked record satisfied every reviewed template slot"
                        .to_owned(),
                });
            }
        }

        if audience.includes_privileged() {
            sections[8].analysis = self
                .privileged_work_product(case_id)?
                .into_iter()
                .map(|item| {
                    format!(
                        "{} — {} (version {}, author {}): {}",
                        item.kind, item.title, item.version, item.author, item.body
                    )
                })
                .collect();
        } else {
            sections[8].analysis.clear();
            sections[8].omitted.push(OmittedDigestTemplate {
                template_id: "privileged-work-product@1".to_owned(),
                reason: "privileged tables are structurally excluded for the disclosable audience"
                    .to_owned(),
            });
        }

        let omitted_sentences = sections
            .iter()
            .map(|section| u32::try_from(section.omitted.len()).unwrap_or(u32::MAX))
            .fold(0_u32, u32::saturating_add);
        let digest = CaseDigest {
            case_id: case_id.0.clone(),
            case_name,
            audience: audience.as_str().to_owned(),
            includes_privileged: audience.includes_privileged(),
            sections,
            omitted_sentences,
        };
        self.validate_digest(&digest)?;
        Ok(digest)
    }

    /// Refuses any generated factual sentence that lost its exact source link.
    pub fn validate_digest(&self, digest: &CaseDigest) -> Result<()> {
        for section in &digest.sections {
            for sentence in &section.sentences {
                if sentence.locators.is_empty() {
                    return Err(Error::UnsupportedSentence(format!(
                        "template `{}` emitted no locator",
                        sentence.template_id
                    )));
                }
                for locator in &sentence.locators {
                    if locator.original.trim().is_empty()
                        || locator.locator.trim().is_empty()
                        || locator.content_id.trim().is_empty()
                    {
                        return Err(Error::UnsupportedSentence(format!(
                            "template `{}` emitted an incomplete locator",
                            sentence.template_id
                        )));
                    }
                    let actual = self
                        .connection
                        .query_row(
                            "SELECT src.logical_name, seg.locator
                             FROM content c
                             JOIN source_segments seg ON seg.id = c.segment_id
                             JOIN sources src ON src.id = seg.source_id
                             WHERE c.id = ?1",
                            [&locator.content_id],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                        )
                        .optional()?;
                    if actual.as_ref() != Some(&(locator.original.clone(), locator.locator.clone()))
                    {
                        return Err(Error::UnsupportedSentence(format!(
                            "template `{}` cites a locator that does not match content `{}`",
                            sentence.template_id, locator.content_id
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn digest_proposition_ids(&self, case_id: &CaseId) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM propositions
             WHERE case_id = ?1 AND status = 'contested' AND review_state <> 'rejected'
             ORDER BY text, id",
        )?;
        statement
            .query_map([&case_id.0], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn blank_section(key: &str, title: &str) -> DigestSection {
    DigestSection {
        key: key.to_owned(),
        title: title.to_owned(),
        sentences: Vec::new(),
        analysis: Vec::new(),
        omitted: Vec::new(),
    }
}

fn populate_from_packet(packet: &PropositionPacket, sections: &mut [DigestSection]) {
    let mappings = [
        ("direct_capture", 1_usize),
        ("contemporaneous_accounts", 2),
        ("later_accounts", 3),
        ("official_assertions", 4),
        ("reporting_dependencies", 6),
        ("raw_conflicts", 6),
        ("missing_unreviewed", 7),
    ];
    for (packet_key, digest_index) in mappings {
        let Some(packet_section) = packet
            .sections
            .iter()
            .find(|section| section.key == packet_key)
        else {
            continue;
        };
        sections[digest_index]
            .analysis
            .extend(packet_section.notes.iter().cloned());
        for item in &packet_section.items {
            let template = match packet_key {
                "direct_capture" => Template::RecordedCapture {
                    proposition: &packet.proposition,
                    item,
                },
                "contemporaneous_accounts" => Template::ContemporaneousAccount {
                    proposition: &packet.proposition,
                    item,
                },
                "later_accounts" => Template::LaterAccount {
                    proposition: &packet.proposition,
                    item,
                },
                "official_assertions" => Template::OfficialAssertion {
                    proposition: &packet.proposition,
                    item,
                },
                "reporting_dependencies" | "raw_conflicts" => Template::DependencyOrConflict {
                    proposition: &packet.proposition,
                    item,
                },
                _ => Template::MissingOrUnreviewed {
                    proposition: &packet.proposition,
                    item,
                },
            };
            push_template(&mut sections[digest_index], &template);
        }
    }

    if let Some(evaluative) = packet
        .sections
        .iter()
        .find(|section| section.key == "evaluative_relations")
    {
        for item in &evaluative.items {
            push_template(
                &mut sections[5],
                &Template::PropositionLink {
                    proposition: &packet.proposition,
                    item,
                },
            );
        }
    }
}

fn push_template(section: &mut DigestSection, template: &Template<'_>) {
    match template.render() {
        Ok(sentence) => {
            let identity = sentence
                .locators
                .first()
                .map(|locator| (sentence.template_id.clone(), locator.content_id.clone()));
            let held = identity.is_some_and(|identity| {
                section.sentences.iter().any(|sentence| {
                    sentence.locators.first().is_some_and(|locator| {
                        sentence.template_id == identity.0 && locator.content_id == identity.1
                    })
                })
            });
            if !held {
                section.sentences.push(sentence);
            }
        }
        Err(reason) => section.omitted.push(OmittedDigestTemplate {
            template_id: template.id().to_owned(),
            reason,
        }),
    }
}

fn time_labels(item: &PacketItem) -> String {
    let mut labels = Vec::new();
    if let Some(value) = &item.normalized_start {
        labels.push(format!("Normalized case time: {value}."));
    } else {
        labels.push("Alignment: not set.".to_owned());
    }
    if let Some(value) = &item.asserted_start {
        labels.push(format!("Alleged event time: {value}."));
    }
    if let Some(value) = &item.content_created_at {
        labels.push(format!("Report created: {value}."));
    }
    labels.join(" ")
}
