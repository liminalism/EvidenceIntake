//! Map scene cuts onto a kernel `NormalizedBatch`.

use evidence_audio::{format_clock, format_locator};
use evidence_intake::{
    CaseId, ContentKind, EdgeKind, ExtractionProvenance, NodeKind, NormalizedBatch,
    NormalizedContent, NormalizedEdge, NormalizedSegment, NormalizedSource, ReviewState,
    SourceKind, TemporalRelation,
};

use crate::caption::{EXTRACTOR_CAPTION, SceneCaption};
use crate::scene::{Keyframe, Scene, SceneAnalysis};
use crate::vision::{Detection, EXTRACTOR_DETECT};
use crate::{Error, Result};

/// Extractor name written on every scene observation.
pub const EXTRACTOR_SCENE: &str = "ffmpeg_scene";
/// Extractor name written on a derived keyframe observation.
pub const EXTRACTOR_KEYFRAME: &str = "ffmpeg_keyframe";
/// Extractor name written on a claimed-duration dropout.
pub const EXTRACTOR_GAP: &str = "video_gap";
/// Version stamped on scene, still, and gap rows.
pub const ANALYSIS_VERSION: &str = "0.1.0";

/// Identity of the original video the scenes were cut from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoIdentity {
    /// Case that already owns the production.
    pub case_id: CaseId,
    /// Existing production ledger identifier.
    pub production_id: String,
    /// Adapter-assigned source identifier of the original video.
    pub source_id: String,
    /// Filename or other display label.
    pub logical_name: String,
    /// MIME type of the untouched original.
    pub media_type: String,
    /// Whether the recording is contemporaneous with the event.
    pub temporal_relation: TemporalRelation,
    /// SHA-256 of the untouched original bytes.
    pub sha256: String,
    /// Length of the untouched original.
    pub byte_length: u64,
}

/// Maps scenes, then attaches boxed detections to the original video source.
pub fn scenes_and_detections_to_batch(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
    detections: &[Detection],
    detector_version: &str,
) -> Result<NormalizedBatch> {
    let mut batch = scenes_to_batch(identity, analysis)?;
    attach_detections(&mut batch, identity, detections, detector_version)?;
    Ok(batch)
}

/// Maps scenes, then attaches boxed detections and scene descriptions.
pub fn analyze_to_batch(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
    detections: &[Detection],
    detector_version: &str,
    captions: &[SceneCaption],
    caption_version: &str,
) -> Result<NormalizedBatch> {
    let mut batch = scenes_to_batch(identity, analysis)?;
    attach_detections(&mut batch, identity, detections, detector_version)?;
    attach_captions(&mut batch, identity, captions, caption_version)?;
    Ok(batch)
}

/// Maps scenes, then attaches scene descriptions to the original video source.
pub fn scenes_and_captions_to_batch(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
    captions: &[SceneCaption],
    caption_version: &str,
) -> Result<NormalizedBatch> {
    let mut batch = scenes_to_batch(identity, analysis)?;
    attach_captions(&mut batch, identity, captions, caption_version)?;
    Ok(batch)
}

/// Appends one suggested observation per scene description onto the original video.
pub fn attach_captions(
    batch: &mut NormalizedBatch,
    identity: &VideoIdentity,
    captions: &[SceneCaption],
    caption_version: &str,
) -> Result<()> {
    let video = batch
        .sources
        .iter_mut()
        .find(|source| source.id == identity.source_id)
        .ok_or_else(|| Error::Mapping("batch has no original video source".to_owned()))?;
    for (ordinal, caption) in captions.iter().enumerate() {
        if caption.end_ms < caption.start_ms {
            return Err(Error::Mapping(
                "scene description ends before it starts".to_owned(),
            ));
        }
        if caption.text.trim().is_empty() {
            return Err(Error::Mapping("scene description is empty".to_owned()));
        }
        if let Some(confidence) = caption.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            return Err(Error::Mapping(
                "scene description confidence is outside 0..=1".to_owned(),
            ));
        }
        video
            .segments
            .push(caption_segment(identity, caption, ordinal, caption_version));
    }
    Ok(())
}

/// Appends one suggested observation per detection onto the original video.
pub fn attach_detections(
    batch: &mut NormalizedBatch,
    identity: &VideoIdentity,
    detections: &[Detection],
    detector_version: &str,
) -> Result<()> {
    let video = batch
        .sources
        .iter_mut()
        .find(|source| source.id == identity.source_id)
        .ok_or_else(|| Error::Mapping("batch has no original video source".to_owned()))?;
    for (ordinal, hit) in detections.iter().enumerate() {
        if hit.end_ms < hit.start_ms {
            return Err(Error::Mapping(format!(
                "detection `{}` ends before it starts",
                hit.label
            )));
        }
        if let Some(confidence) = hit.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            return Err(Error::Mapping(format!(
                "detection `{}` confidence is outside 0..=1",
                hit.label
            )));
        }
        if hit
            .bbox
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(Error::Mapping(format!(
                "detection `{}` box is not a finite non-negative [x,y,w,h]",
                hit.label
            )));
        }
        video
            .segments
            .push(detection_segment(identity, hit, ordinal, detector_version));
    }
    Ok(())
}

/// Maps a [`SceneAnalysis`] onto a video source plus derived still sources.
pub fn scenes_to_batch(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
) -> Result<NormalizedBatch> {
    if analysis.scenes.is_empty() {
        return Err(Error::Mapping(
            "scene analysis produced no scenes".to_owned(),
        ));
    }

    let mut video_segments = Vec::new();
    for scene in &analysis.scenes {
        if scene.end_ms <= scene.start_ms {
            return Err(Error::Mapping(format!(
                "scene {} ends before it starts",
                scene.index
            )));
        }
        video_segments.push(scene_segment(identity, scene));
    }
    if let Some((start_ms, end_ms)) = analysis.dropout {
        video_segments.push(gap_segment(&identity.source_id, start_ms, end_ms));
    }

    let mut sources = vec![NormalizedSource {
        id: identity.source_id.clone(),
        production_id: identity.production_id.clone(),
        logical_name: identity.logical_name.clone(),
        media_type: identity.media_type.clone(),
        source_kind: SourceKind::Video,
        temporal_relation: identity.temporal_relation,
        sha256: identity.sha256.clone(),
        byte_length: identity.byte_length,
        segments: video_segments,
    }];

    let mut edges = Vec::new();
    for scene in &analysis.scenes {
        if let Some(still) = &scene.keyframe {
            let source = still_source(identity, scene, still);
            edges.push(derived_from_edge(identity, scene, &source.id));
            sources.push(source);
        }
    }

    Ok(NormalizedBatch {
        case_id: identity.case_id.clone(),
        sources,
        edges,
    })
}

/// The still is a working copy cut from the video: say so structurally.
fn derived_from_edge(identity: &VideoIdentity, scene: &Scene, still_id: &str) -> NormalizedEdge {
    NormalizedEdge {
        id: format!("{}-stilledge-{:04}", identity.source_id, scene.index),
        from_kind: NodeKind::Source,
        from_id: still_id.to_owned(),
        relation: EdgeKind::DerivedFrom,
        to_kind: NodeKind::Source,
        to_id: identity.source_id.clone(),
        rationale: format!(
            "Keyframe extracted by ffmpeg from `{}` at {} (scene {}); a derived working copy,              not the original.",
            identity.logical_name,
            format_clock(scene.start_ms),
            scene.index
        ),
        extraction: machine(EXTRACTOR_KEYFRAME, None),
    }
}

fn scene_locator(scene: &Scene) -> String {
    format!(
        "scene {}, {}",
        scene.index,
        format_locator(scene.start_ms, scene.end_ms)
    )
}

fn scene_segment(identity: &VideoIdentity, scene: &Scene) -> NormalizedSegment {
    let locator = scene_locator(scene);
    let still_note = if scene.keyframe.is_some() {
        " A working-copy keyframe was extracted; verify against the original frame."
    } else {
        ""
    };
    NormalizedSegment {
        id: format!("{}-scene-{:04}", identity.source_id, scene.index),
        locator: locator.clone(),
        page: None,
        start_ms: Some(scene.start_ms),
        end_ms: Some(scene.end_ms),
        bounding_box: None,
        content: vec![NormalizedContent {
            id: format!("{}-scenec-{:04}", identity.source_id, scene.index),
            kind: ContentKind::Observation,
            text: format!(
                "Scene {} on the original timeline.{still_note}",
                scene.index
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: machine(EXTRACTOR_SCENE, None),
        }],
    }
}

fn still_source(identity: &VideoIdentity, scene: &Scene, still: &Keyframe) -> NormalizedSource {
    let locator = scene_locator(scene);
    NormalizedSource {
        id: format!("{}-still-{:04}", identity.source_id, scene.index),
        production_id: identity.production_id.clone(),
        logical_name: format!(
            "{} @ {}",
            identity.logical_name,
            format_clock(scene.start_ms)
        ),
        media_type: "image/jpeg".to_owned(),
        source_kind: SourceKind::Other,
        temporal_relation: identity.temporal_relation,
        sha256: still.sha256.clone(),
        byte_length: still.byte_length,
        segments: vec![NormalizedSegment {
            id: format!("{}-stillseg-{:04}", identity.source_id, scene.index),
            locator: locator.clone(),
            page: None,
            start_ms: Some(scene.start_ms),
            end_ms: Some(scene.start_ms),
            bounding_box: None,
            content: vec![NormalizedContent {
                id: format!("{}-stillc-{:04}", identity.source_id, scene.index),
                kind: ContentKind::Observation,
                text: format!(
                    "Keyframe from `{}` at {}. Derived working copy — not the original.",
                    identity.logical_name,
                    format_clock(scene.start_ms)
                ),
                speaker_entity_id: None,
                attributed_to_entity_id: None,
                parent_content_id: None,
                raw_time: None,
                content_created_at: None,
                asserted_time: None,
                normalized_start: None,
                normalized_end: None,
                time_basis: None,
                location_text: None,
                extraction: machine(EXTRACTOR_KEYFRAME, None),
            }],
        }],
    }
}

fn detection_segment(
    identity: &VideoIdentity,
    hit: &Detection,
    ordinal: usize,
    version: &str,
) -> NormalizedSegment {
    let scene = hit
        .scene_index
        .map(|index| format!("scene {index}, "))
        .unwrap_or_default();
    let locator = format!(
        "{scene}{}; detector proposed {} (#{})",
        format_locator(hit.start_ms, hit.end_ms),
        hit.label,
        ordinal + 1
    );
    let confidence = hit
        .confidence
        .map(|value| format!(" (confidence {value:.2})"))
        .unwrap_or_default();
    NormalizedSegment {
        id: format!("{}-det-{ordinal:04}", identity.source_id),
        locator,
        page: None,
        start_ms: Some(hit.start_ms),
        end_ms: Some(hit.end_ms),
        bounding_box: Some(hit.bbox),
        content: vec![NormalizedContent {
            id: format!("{}-detc-{ordinal:04}", identity.source_id),
            kind: ContentKind::Observation,
            text: format!(
                "Detector proposed `{}`{confidence}. Suggested; open the original at this locator.",
                hit.label
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: ExtractionProvenance {
                extractor: EXTRACTOR_DETECT.to_owned(),
                version: version.to_owned(),
                machine_generated: true,
                confidence: hit.confidence,
                review_state: ReviewState::Suggested,
            },
        }],
    }
}

fn caption_segment(
    identity: &VideoIdentity,
    caption: &SceneCaption,
    ordinal: usize,
    version: &str,
) -> NormalizedSegment {
    let scene = caption
        .scene_index
        .map(|index| format!("scene {index}, "))
        .unwrap_or_default();
    let locator = format!(
        "{scene}{}; scene description",
        format_locator(caption.start_ms, caption.end_ms)
    );
    NormalizedSegment {
        id: format!("{}-cap-{ordinal:04}", identity.source_id),
        locator,
        page: None,
        start_ms: Some(caption.start_ms),
        end_ms: Some(caption.end_ms),
        bounding_box: None,
        content: vec![NormalizedContent {
            id: format!("{}-capc-{ordinal:04}", identity.source_id),
            kind: ContentKind::Observation,
            text: format!(
                "Scene description (machine): {}. Suggested; open the original at this locator.",
                caption.text.trim()
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: ExtractionProvenance {
                extractor: EXTRACTOR_CAPTION.to_owned(),
                version: version.to_owned(),
                machine_generated: true,
                confidence: caption.confidence,
                review_state: ReviewState::Suggested,
            },
        }],
    }
}

fn gap_segment(source_id: &str, start_ms: u64, end_ms: u64) -> NormalizedSegment {
    NormalizedSegment {
        id: format!("{source_id}-vgap"),
        locator: format_locator(start_ms, end_ms),
        page: None,
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        bounding_box: None,
        content: vec![NormalizedContent {
            id: format!("{source_id}-vgapc"),
            kind: ContentKind::RecordingGap,
            text: format!(
                "The video stream ends at {} but the container claims {}.",
                format_clock(start_ms),
                format_clock(end_ms)
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: machine(EXTRACTOR_GAP, None),
        }],
    }
}

fn machine(extractor: &str, confidence: Option<f64>) -> ExtractionProvenance {
    ExtractionProvenance {
        extractor: extractor.to_owned(),
        version: ANALYSIS_VERSION.to_owned(),
        machine_generated: true,
        confidence,
        review_state: ReviewState::Suggested,
    }
}
