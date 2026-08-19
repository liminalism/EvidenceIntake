//! Video intake adapter for the evidence collation kernel.
//!
//! Slice 1 cuts scenes. Slice 2 attaches boxed detections. Slice 3 attaches
//! one VLM description per scene. Slice 4 hands the original container's audio
//! to the audio adapter and merges the spoken statements into the same batch,
//! under the same source. The kernel crate never depends on this one.

mod caption;
mod clock;
mod embed;
mod error;
mod map;
mod scene;
mod soundtrack;
mod sync;
mod vision;

pub use caption::{
    CaptionBackend, CaptionDocument, DEFAULT_PROMPT, EXTRACTOR_CAPTION, JsonCaptionBackend,
    SceneCaption, VlmCliBackend, caption_scene, caption_scenes,
};
pub use clock::{
    ClockBackend, ClockDocument, ClockReading, EXTRACTOR_CLOCK, JsonClockBackend, OverlayBand,
    RawClockText, TesseractCliBackend, attach_clock_readings, parse_clock_text,
    read_clock_overlays, read_clocks, read_clocks_from_json,
};
pub use embed::{
    CliEmbeddingBackend, EXTRACTOR_EMBED, EmbeddingBackend, EmbeddingDocument,
    JsonEmbeddingBackend, QueryEmbedding, StillEmbedding, embed_from_document, embed_keyframes,
};
pub use error::{Error, Result};
pub use map::{
    ANALYSIS_VERSION, EXTRACTOR_GAP, EXTRACTOR_KEYFRAME, EXTRACTOR_SCENE, VideoIdentity,
    analyze_to_batch, attach_captions, attach_detections, scenes_and_captions_to_batch,
    scenes_and_detections_to_batch, scenes_to_batch,
};
pub use scene::{
    DEFAULT_GAP_MS, DEFAULT_THRESHOLD, Keyframe, Scene, SceneAnalysis, detect_scenes, dropout_span,
    ffprobe_available, jpeg_dimensions, parse_scene_report,
};
pub use soundtrack::{
    DEFAULT_SPEECH_GAP_MS, SoundtrackInput, SoundtrackOptions, merge_soundtrack, soundtrack_batch,
    soundtrack_request,
};
pub use sync::{
    EXTRACTOR_SYNC, SYNC_VERSION, SyncMeasurement, SyncOptions, SyncPair, SyncSide, measure_offset,
    measurement_to_batch, sync_pair,
};
pub use vision::{
    Detection, DetectionDocument, EXTRACTOR_DETECT, JsonVisionBackend, RawDetection, VisionBackend,
    YoloCliBackend, coco_label, detect_on_scenes, parse_yolo_txt, place_on_scene,
};

use std::path::Path;

use evidence_audio::{MediaClass, classify, media_type};
use evidence_intake::{CaseId, KeyframeIndex, NormalizedBatch, TemporalRelation};
use sha2::{Digest, Sha256};

/// Inputs for one video → one [`NormalizedBatch`].
#[derive(Debug, Clone)]
pub struct SceneRequest {
    /// Existing case.
    pub case_id: CaseId,
    /// Existing production that will own the sources.
    pub production_id: String,
    /// Adapter-assigned source identifier of the original video.
    pub source_id: String,
    /// Path to the untouched original.
    pub path: std::path::PathBuf,
    /// Display name; the file name when omitted by the caller.
    pub logical_name: Option<String>,
    /// Whether the recording is contemporaneous. Default is `unknown`.
    pub temporal_relation: TemporalRelation,
    /// ffmpeg `scene` score that counts as a cut.
    pub threshold: f64,
    /// Minimum missing tail, in milliseconds, that becomes a `recording_gap`.
    pub gap_ms: u64,
    /// Where to keep derived stills. A temp directory when omitted.
    pub stills_dir: Option<std::path::PathBuf>,
}

/// Hash the original, cut scenes, extract stills, and map a batch.
pub fn cut_scenes(request: &SceneRequest) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    scenes_to_batch(&identity, &analysis)
}

/// Cut scenes, run `backend` on each still, and emit scenes plus detections.
pub fn detect_objects(
    request: &SceneRequest,
    backend: &dyn VisionBackend,
) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let detections = detect_on_scenes(&analysis, backend)?;
    scenes_and_detections_to_batch(&identity, &analysis, &detections, &backend.version())
}

/// Hash the original, cut scenes, and attach a JSON detection document.
pub fn detect_from_json(request: &SceneRequest, json_path: &Path) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let document = JsonVisionBackend {
        path: json_path.to_path_buf(),
    }
    .load()?;
    let version = document
        .version
        .clone()
        .unwrap_or_else(|| "from-json".to_owned());
    scenes_and_detections_to_batch(&identity, &analysis, &document.detections, &version)
}

/// Cut scenes, run `backend` on each still, and emit scenes plus descriptions.
pub fn describe_scenes(
    request: &SceneRequest,
    backend: &dyn CaptionBackend,
) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let captions = caption_scenes(&analysis, backend)?;
    scenes_and_captions_to_batch(&identity, &analysis, &captions, &backend.version())
}

/// Where detections come from during [`analyze`].
#[derive(Clone, Copy)]
pub enum DetectionInput<'a> {
    /// Skip the detector.
    None,
    /// Load a prior detection document.
    Json(&'a Path),
    /// Run a live backend on each still.
    Live(&'a dyn VisionBackend),
}

/// Where scene descriptions come from during [`analyze`].
#[derive(Clone, Copy)]
pub enum CaptionInput<'a> {
    /// Skip the VLM.
    None,
    /// Load a prior caption document.
    Json(&'a Path),
    /// Run a live backend on each still.
    Live(&'a dyn CaptionBackend),
}

/// Where clock-overlay readings come from during [`analyze`].
#[derive(Clone, Copy)]
pub enum ClockInput<'a> {
    /// Skip the overlay OCR.
    None,
    /// Load a prior clock document.
    Json(&'a Path),
    /// Run a live OCR backend on each still.
    Live(&'a dyn ClockBackend),
}

/// Cut scenes once, then optionally attach boxes, captions, clock readings, and speech.
///
/// The soundtrack is transcribed from the same original container and merged
/// onto the same source, so a scene and the statements spoken inside it are
/// siblings.
pub fn analyze(
    request: &SceneRequest,
    detections: DetectionInput<'_>,
    captions: CaptionInput<'_>,
    clocks: ClockInput<'_>,
    soundtrack: SoundtrackInput<'_>,
) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let (boxes, box_version) = match detections {
        DetectionInput::None => (Vec::new(), String::new()),
        DetectionInput::Json(path) => {
            let document = JsonVisionBackend {
                path: path.to_path_buf(),
            }
            .load()?;
            let version = document
                .version
                .clone()
                .unwrap_or_else(|| "from-json".to_owned());
            (document.detections, version)
        }
        DetectionInput::Live(backend) => {
            let hits = detect_on_scenes(&analysis, backend)?;
            (hits, backend.version())
        }
    };
    let (texts, text_version) = match captions {
        CaptionInput::None => (Vec::new(), String::new()),
        CaptionInput::Json(path) => {
            let document = JsonCaptionBackend {
                path: path.to_path_buf(),
            }
            .load()?;
            let version = document
                .version
                .clone()
                .unwrap_or_else(|| "from-json".to_owned());
            (document.captions, version)
        }
        CaptionInput::Live(backend) => {
            let hits = caption_scenes(&analysis, backend)?;
            (hits, backend.version())
        }
    };
    let mut batch = analyze_to_batch(
        &identity,
        &analysis,
        &boxes,
        &box_version,
        &texts,
        &text_version,
    )?;
    match clocks {
        ClockInput::None => {}
        ClockInput::Json(path) => {
            let document = JsonClockBackend {
                path: path.to_path_buf(),
            }
            .load()?;
            let version = document
                .version
                .clone()
                .unwrap_or_else(|| "from-json".to_owned());
            attach_clock_readings(&mut batch, &identity, &document.readings, &version)?;
        }
        ClockInput::Live(backend) => {
            let readings = read_clocks(&analysis, backend)?;
            attach_clock_readings(&mut batch, &identity, &readings, &backend.version())?;
        }
    }
    if let Some(spoken) = soundtrack_batch(request, soundtrack, SoundtrackOptions::default())? {
        merge_soundtrack(&mut batch, &identity, spoken)?;
    }
    Ok(batch)
}

/// Cut scenes and embed each keyframe with `backend`.
pub fn embed_scenes(
    request: &SceneRequest,
    backend: &dyn EmbeddingBackend,
) -> Result<KeyframeIndex> {
    let (identity, analysis) = open_and_cut(request)?;
    embed_keyframes(&identity, &analysis, backend)
}

/// Hash the original, cut scenes, and attach a JSON embedding document.
pub fn embed_from_json(request: &SceneRequest, json_path: &Path) -> Result<KeyframeIndex> {
    let (identity, analysis) = open_and_cut(request)?;
    let document = JsonEmbeddingBackend {
        path: json_path.to_path_buf(),
    }
    .load()?;
    embed_from_document(&identity, &analysis, &document)
}

/// Cut scenes and transcribe the soundtrack into one batch, nothing else.
pub fn transcribe_soundtrack(
    request: &SceneRequest,
    backend: &dyn evidence_audio::TranscriptBackend,
) -> Result<NormalizedBatch> {
    analyze(
        request,
        DetectionInput::None,
        CaptionInput::None,
        ClockInput::None,
        SoundtrackInput::Live(backend),
    )
}

/// Hash the original, cut scenes, and attach a JSON caption document.
pub fn describe_from_json(request: &SceneRequest, json_path: &Path) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let document = JsonCaptionBackend {
        path: json_path.to_path_buf(),
    }
    .load()?;
    let version = document
        .version
        .clone()
        .unwrap_or_else(|| "from-json".to_owned());
    scenes_and_captions_to_batch(&identity, &analysis, &document.captions, &version)
}

pub(crate) fn open_and_cut(request: &SceneRequest) -> Result<(VideoIdentity, SceneAnalysis)> {
    match classify(&request.path) {
        MediaClass::Video => {}
        other => {
            return Err(Error::Open {
                path: request.path.clone(),
                message: format!("expected a video container, classified as {other:?}"),
            });
        }
    }
    let identity = hash_original(request)?;
    let analysis = detect_scenes(
        &request.path,
        request.threshold,
        request.gap_ms,
        request.stills_dir.as_deref(),
    )?;
    Ok((identity, analysis))
}

fn hash_original(request: &SceneRequest) -> Result<VideoIdentity> {
    let bytes = std::fs::read(&request.path).map_err(|error| Error::Open {
        path: request.path.clone(),
        message: error.to_string(),
    })?;
    if bytes.is_empty() {
        return Err(Error::Open {
            path: request.path.clone(),
            message: "file is empty".to_owned(),
        });
    }
    let logical_name = request.logical_name.clone().unwrap_or_else(|| {
        request
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("video.mp4")
            .to_owned()
    });
    Ok(VideoIdentity {
        case_id: request.case_id.clone(),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        logical_name,
        media_type: media_type(Path::new(&request.path)).to_owned(),
        temporal_relation: request.temporal_relation,
        sha256: hex::encode(Sha256::digest(&bytes)),
        byte_length: bytes.len() as u64,
    })
}
