//! Vision backends. Detectors propose boxes; they do not identify people.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::scene::{Keyframe, Scene, SceneAnalysis};
use crate::{Error, Result};

/// Extractor name written on a boxed detector observation.
pub const EXTRACTOR_DETECT: &str = "vision_detect";

/// One detector hit, already placed on the original timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    /// One-based scene index, when the hit came from a scene still.
    #[serde(default)]
    pub scene_index: Option<u32>,
    /// Inclusive start on the original timeline.
    pub start_ms: u64,
    /// Exclusive end on the original timeline.
    pub end_ms: u64,
    /// Detector class or open-vocabulary phrase. Not a person name.
    pub label: String,
    /// `[x, y, width, height]` in original-frame pixels (after scaling).
    pub bbox: [f64; 4],
    /// Detector confidence in `0..=1`, when reported.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// A JSON document of detections, as produced by a prior run or a test fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DetectionDocument {
    /// Ordered hits.
    #[serde(default)]
    pub detections: Vec<Detection>,
    /// Backend name stored on each observation.
    #[serde(default)]
    pub extractor: Option<String>,
    /// Backend version stored on each observation.
    #[serde(default)]
    pub version: Option<String>,
}

/// A box in the space the detector used, before scaling.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDetection {
    /// Detector class or phrase.
    pub label: String,
    /// `[x, y, width, height]`.
    pub bbox: [f64; 4],
    /// Whether `bbox` is normalized to `0..=1`.
    pub normalized: bool,
    /// Detector confidence in `0..=1`.
    pub confidence: Option<f64>,
}

/// Something that can propose boxes on a still.
pub trait VisionBackend {
    /// Detect objects on one working-copy still.
    fn detect_still(&self, still: &Path) -> Result<Vec<RawDetection>>;

    /// Extractor name stored on each observation.
    fn extractor(&self) -> &str;

    /// Version string stored on each observation.
    fn version(&self) -> String;
}

/// Reads a [`DetectionDocument`] from disk. Used in tests and when the
/// operator already ran a detector.
#[derive(Debug, Clone)]
pub struct JsonVisionBackend {
    /// Path to the JSON document.
    pub path: PathBuf,
}

impl JsonVisionBackend {
    /// Load the document. Times are already on the original timeline.
    pub fn load(&self) -> Result<DetectionDocument> {
        let json = std::fs::read_to_string(&self.path).map_err(|error| {
            Error::Backend(format!("could not read {}: {error}", self.path.display()))
        })?;
        serde_json::from_str(&json).map_err(|error| {
            Error::Backend(format!(
                "{} is not a detection document: {error}",
                self.path.display()
            ))
        })
    }
}

/// Runs the Ultralytics `yolo` CLI on a still and reads YOLO-txt labels.
///
/// Overnight, not realtime. Missing binary is a hard error with an install hint.
#[derive(Debug, Clone)]
pub struct YoloCliBackend {
    /// Binary name or path. Default `yolo`.
    pub bin: PathBuf,
    /// Model argument (`yolov8n.pt`, `yolov8s-world.pt`, a local path).
    pub model: String,
    /// Minimum confidence to keep.
    pub confidence: f64,
}

impl YoloCliBackend {
    /// `yolo` on PATH, nano COCO weights.
    pub fn default_local() -> Self {
        Self {
            bin: PathBuf::from("yolo"),
            model: "yolov8n.pt".to_owned(),
            confidence: 0.25,
        }
    }
}

impl VisionBackend for YoloCliBackend {
    fn detect_still(&self, still: &Path) -> Result<Vec<RawDetection>> {
        let work = tempfile::tempdir().map_err(|error| {
            Error::Backend(format!("could not create yolo output dir: {error}"))
        })?;
        let status = Command::new(&self.bin)
            .current_dir(work.path())
            .args([
                "detect",
                "predict",
                &format!("model={}", self.model),
                &format!("source={}", still.display()),
                &format!("conf={}", self.confidence),
                "save=False",
                "save_txt=True",
                "save_conf=True",
                "project=.",
                "name=run",
                "exist_ok=True",
            ])
            .status()
            .map_err(|error| {
                Error::Backend(format!(
                    "could not run `{}`: {error}. Install Ultralytics (`pip install ultralytics`) or pass --from-json.",
                    self.bin.display()
                ))
            })?;
        if !status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {status}",
                self.bin.display()
            )));
        }
        let stem = still
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| Error::Backend("still path has no file stem".to_owned()))?;
        let label_path = work
            .path()
            .join("run")
            .join("labels")
            .join(format!("{stem}.txt"));
        if !label_path.is_file() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&label_path)?;
        parse_yolo_txt(&text)
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_DETECT
    }

    fn version(&self) -> String {
        format!("yolo@{}", self.model)
    }
}

/// Parses Ultralytics/YOLO txt: `class_id x_center y_center width height [conf]`.
pub fn parse_yolo_txt(text: &str) -> Result<Vec<RawDetection>> {
    let mut hits = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            return Err(Error::Backend(format!(
                "yolo txt line {} is not class xc yc w h [conf]",
                line_no + 1
            )));
        }
        let class_id: usize = parts[0].parse().map_err(|_| {
            Error::Backend(format!(
                "yolo txt line {} has a non-integer class",
                line_no + 1
            ))
        })?;
        let xc: f64 = parse_f64(parts[1], line_no)?;
        let yc: f64 = parse_f64(parts[2], line_no)?;
        let width: f64 = parse_f64(parts[3], line_no)?;
        let height: f64 = parse_f64(parts[4], line_no)?;
        let confidence = parts
            .get(5)
            .map(|value| parse_f64(value, line_no))
            .transpose()?;
        let x = xc - width / 2.0;
        let y = yc - height / 2.0;
        hits.push(RawDetection {
            label: coco_label(class_id).to_owned(),
            bbox: [x, y, width, height],
            normalized: true,
            confidence,
        });
    }
    Ok(hits)
}

fn parse_f64(value: &str, line_no: usize) -> Result<f64> {
    value.parse().map_err(|_| {
        Error::Backend(format!(
            "yolo txt line {} has a non-numeric field",
            line_no + 1
        ))
    })
}

/// Places raw boxes onto a scene, scaling normalized coords by the still size.
pub fn place_on_scene(
    scene: &Scene,
    still: Option<&Keyframe>,
    raw: &[RawDetection],
) -> Result<Vec<Detection>> {
    let mut placed = Vec::new();
    for hit in raw {
        let bbox = scale_bbox(hit, still)?;
        let label = hit.label.trim();
        if label.is_empty() {
            return Err(Error::Mapping("detection has an empty label".to_owned()));
        }
        if let Some(confidence) = hit.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            return Err(Error::Mapping(format!(
                "detection `{label}` confidence is outside 0..=1"
            )));
        }
        placed.push(Detection {
            scene_index: Some(scene.index),
            start_ms: scene.start_ms,
            end_ms: scene.end_ms,
            label: label.to_owned(),
            bbox,
            confidence: hit.confidence,
        });
    }
    Ok(placed)
}

fn scale_bbox(hit: &RawDetection, still: Option<&Keyframe>) -> Result<[f64; 4]> {
    if !hit.normalized {
        return Ok(hit.bbox);
    }
    let (width, height) = still
        .and_then(|frame| Some((f64::from(frame.width?), f64::from(frame.height?))))
        .ok_or_else(|| {
            Error::Mapping(
                "normalized box needs the still's pixel size to reach original-frame space"
                    .to_owned(),
            )
        })?;
    Ok([
        hit.bbox[0] * width,
        hit.bbox[1] * height,
        hit.bbox[2] * width,
        hit.bbox[3] * height,
    ])
}

/// Runs `backend` on every scene still.
pub fn detect_on_scenes(
    analysis: &SceneAnalysis,
    backend: &dyn VisionBackend,
) -> Result<Vec<Detection>> {
    let mut all = Vec::new();
    for scene in &analysis.scenes {
        let Some(still) = scene.keyframe.as_ref() else {
            continue;
        };
        let raw = backend.detect_still(&still.path)?;
        all.extend(place_on_scene(scene, Some(still), &raw)?);
    }
    Ok(all)
}

/// COCO-80 names used by the default Ultralytics detect weights.
pub fn coco_label(class_id: usize) -> &'static str {
    COCO80.get(class_id).copied().unwrap_or("object")
}

const COCO80: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];
