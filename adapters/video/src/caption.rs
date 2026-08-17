//! Scene descriptions. A VLM proposes text; it does not narrate a verdict.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::scene::{Scene, SceneAnalysis};
use crate::{Error, Result};

/// Extractor name written on a scene-description observation.
pub const EXTRACTOR_CAPTION: &str = "vlm_scene";

/// Prompt used when the operator does not supply one.
pub const DEFAULT_PROMPT: &str = "Describe only what is visible in this single video frame. \
Do not infer what happened before or after. Do not identify anyone by name. \
One or two sentences.";

/// One VLM description, already placed on the original timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneCaption {
    /// One-based scene index, when the text came from a scene still.
    #[serde(default)]
    pub scene_index: Option<u32>,
    /// Inclusive start on the original timeline.
    pub start_ms: u64,
    /// Exclusive end on the original timeline.
    pub end_ms: u64,
    /// The model's description of the still. Not a finding of fact.
    pub text: String,
    /// Model confidence in `0..=1`, when reported.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// A JSON document of captions, as produced by a prior run or a test fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CaptionDocument {
    /// Ordered scene descriptions.
    #[serde(default)]
    pub captions: Vec<SceneCaption>,
    /// Backend name stored on each observation.
    #[serde(default)]
    pub extractor: Option<String>,
    /// Backend version stored on each observation.
    #[serde(default)]
    pub version: Option<String>,
}

/// Something that can describe one working-copy still.
pub trait CaptionBackend {
    /// Describe `still`. Return one or two sentences about the pixels only.
    fn describe_still(&self, still: &Path) -> Result<String>;

    /// Extractor name stored on each observation.
    fn extractor(&self) -> &str;

    /// Version string stored on each observation.
    fn version(&self) -> String;
}

/// Reads a [`CaptionDocument`] from disk.
#[derive(Debug, Clone)]
pub struct JsonCaptionBackend {
    /// Path to the JSON document.
    pub path: PathBuf,
}

impl JsonCaptionBackend {
    /// Load timed captions. The stills are not consulted.
    pub fn load(&self) -> Result<CaptionDocument> {
        let json = std::fs::read_to_string(&self.path).map_err(|error| {
            Error::Backend(format!("could not read {}: {error}", self.path.display()))
        })?;
        serde_json::from_str(&json).map_err(|error| {
            Error::Backend(format!(
                "{} is not a caption document: {error}",
                self.path.display()
            ))
        })
    }
}

/// Shells out to a local VLM CLI. Default shape is `ollama run <model> <prompt> <image>`.
#[derive(Debug, Clone)]
pub struct VlmCliBackend {
    /// Binary name or path. Default `ollama`.
    pub bin: PathBuf,
    /// Model name (`llava`, `qwen2.5vl`, …).
    pub model: String,
    /// Instruction given with the still.
    pub prompt: String,
}

impl VlmCliBackend {
    /// `ollama` on PATH, `llava`, bounded prompt.
    pub fn default_local() -> Self {
        Self {
            bin: PathBuf::from("ollama"),
            model: "llava".to_owned(),
            prompt: DEFAULT_PROMPT.to_owned(),
        }
    }
}

impl CaptionBackend for VlmCliBackend {
    fn describe_still(&self, still: &Path) -> Result<String> {
        let output = Command::new(&self.bin)
            .arg("run")
            .arg(&self.model)
            .arg(&self.prompt)
            .arg(still)
            .output()
            .map_err(|error| {
                Error::Backend(format!(
                    "could not run `{}`: {error}. Install a local VLM CLI (ollama) or pass --from-json.",
                    self.bin.display()
                ))
            })?;
        if !output.status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {}",
                self.bin.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if text.is_empty() {
            return Err(Error::Backend(format!(
                "`{}` wrote no description for {}",
                self.bin.display(),
                still.display()
            )));
        }
        Ok(text)
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_CAPTION
    }

    fn version(&self) -> String {
        format!("vlm@{}", self.model)
    }
}

/// Places a raw description onto a scene.
pub fn caption_scene(scene: &Scene, text: &str, confidence: Option<f64>) -> Result<SceneCaption> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Mapping("scene description is empty".to_owned()));
    }
    if let Some(value) = confidence
        && !(0.0..=1.0).contains(&value)
    {
        return Err(Error::Mapping(
            "scene description confidence is outside 0..=1".to_owned(),
        ));
    }
    if scene.end_ms < scene.start_ms {
        return Err(Error::Mapping(format!(
            "scene {} ends before it starts",
            scene.index
        )));
    }
    Ok(SceneCaption {
        scene_index: Some(scene.index),
        start_ms: scene.start_ms,
        end_ms: scene.end_ms,
        text: text.to_owned(),
        confidence,
    })
}

/// Runs `backend` on every scene still.
pub fn caption_scenes(
    analysis: &SceneAnalysis,
    backend: &dyn CaptionBackend,
) -> Result<Vec<SceneCaption>> {
    let mut captions = Vec::new();
    for scene in &analysis.scenes {
        let Some(still) = scene.keyframe.as_ref() else {
            continue;
        };
        let text = backend.describe_still(&still.path)?;
        captions.push(caption_scene(scene, &text, None)?);
    }
    Ok(captions)
}
