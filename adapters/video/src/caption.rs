//! Scene descriptions. A VLM proposes text; it does not narrate a verdict.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scene::{Scene, SceneAnalysis};
use crate::{Error, Result};

/// Extractor name written on a scene-description observation.
pub const EXTRACTOR_CAPTION: &str = "vlm_scene";

/// Prompt used when the operator does not supply one.
pub const DEFAULT_PROMPT: &str = "Write exactly one short plain-text sentence describing this single video frame as a navigation hint for a lawyer. \
State only coarse features directly visible in the frame: clothing, objects, setting, and spatial relationships. \
Prefer visible appearance over assigning a role. Do not identify a person, organization, unit, location, insignia, \
vehicle owner, or event. Do not infer intent, cause, sequence, speed, authenticity, or anything before or after the frame. \
Return one sentence only, with no JSON, list, explanation, or timestamp. If the scene cannot be described conservatively, \
answer exactly ABSTAIN.";

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
    /// Describe `still`, or abstain when no conservative description is available.
    fn describe_still(&self, still: &Path) -> Result<Option<String>>;

    /// Describe several stills while allowing a backend to load once.
    fn describe_stills(&self, stills: &[PathBuf]) -> Result<Vec<Option<String>>> {
        stills
            .iter()
            .map(|still| self.describe_still(still))
            .collect()
    }

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
    let keyed: Vec<_> = analysis
        .scenes
        .iter()
        .filter_map(|scene| scene.keyframe.as_ref().map(|still| (scene, still)))
        .collect();
    let paths: Vec<_> = keyed.iter().map(|(_, still)| still.path.clone()).collect();
    let descriptions = backend.describe_stills(&paths)?;
    if descriptions.len() != keyed.len() {
        return Err(Error::Backend(format!(
            "caption backend returned {} results for {} keyframes",
            descriptions.len(),
            keyed.len()
        )));
    }
    let mut captions = Vec::new();
    for ((scene, _), description) in keyed.into_iter().zip(descriptions) {
        if let Some(text) = description {
            captions.push(caption_scene(scene, &text, None)?);
        }
    }
    Ok(captions)
}
