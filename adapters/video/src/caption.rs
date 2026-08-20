//! Scene descriptions. A VLM proposes text; it does not narrate a verdict.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    fn describe_still(&self, still: &Path) -> Result<Option<String>> {
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
        Ok(Some(text))
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_CAPTION
    }

    fn version(&self) -> String {
        format!("vlm@{}", self.model)
    }
}

/// Shells out once to a local caption CLI for a batch of still paths.
///
/// The CLI reads `{"paths":[...]}` from standard input for its `batch`
/// command and writes `{"captions":["...",null]}`. A null value is an
/// explicit abstention, not a failed job.
#[derive(Debug, Clone)]
pub struct BatchCaptionCliBackend {
    /// Caption CLI executable or script.
    pub bin: PathBuf,
    /// Python interpreter when `bin` is a Python script.
    pub python: Option<PathBuf>,
    /// Fully local model directory.
    pub model_dir: PathBuf,
    /// Immutable model identifier stamped on generated content.
    pub model: String,
    /// Instruction applied independently to each still.
    pub prompt: String,
    /// Torch device passed to the caption CLI.
    pub device: String,
    /// Torch dtype passed to the caption CLI.
    pub torch_dtype: String,
    /// Maximum newly generated tokens for one still.
    pub max_new_tokens: u32,
}

#[derive(Serialize)]
struct CaptionBatchRequest<'a> {
    paths: &'a [PathBuf],
}

#[derive(Deserialize)]
struct CaptionBatchResponse {
    captions: Vec<Option<String>>,
}

impl CaptionBackend for BatchCaptionCliBackend {
    fn describe_still(&self, still: &Path) -> Result<Option<String>> {
        let mut captions = self.describe_stills(&[still.to_path_buf()])?;
        Ok(captions.pop().flatten())
    }

    fn describe_stills(&self, stills: &[PathBuf]) -> Result<Vec<Option<String>>> {
        if stills.is_empty() {
            return Ok(Vec::new());
        }
        let mut command = self.command();
        command
            .arg("batch")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| self.spawn_error(&error))?;
        let request = serde_json::to_vec(&CaptionBatchRequest { paths: stills })?;
        child
            .stdin
            .take()
            .ok_or_else(|| Error::Backend("caption CLI stdin was not piped".to_owned()))?
            .write_all(&request)?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {}",
                self.bin.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let parsed: CaptionBatchResponse =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                Error::Backend(format!(
                    "`{}` did not print {{\"captions\":[\"...\",null]}}: {error}",
                    self.bin.display()
                ))
            })?;
        if parsed.captions.len() != stills.len() {
            return Err(Error::Backend(format!(
                "`{}` returned {} captions for {} stills",
                self.bin.display(),
                parsed.captions.len(),
                stills.len()
            )));
        }
        parsed
            .captions
            .into_iter()
            .map(|caption| match caption {
                Some(text) if text.trim().is_empty() => Err(Error::Backend(format!(
                    "`{}` returned an empty caption instead of null",
                    self.bin.display()
                ))),
                Some(text) => Ok(Some(text.trim().to_owned())),
                None => Ok(None),
            })
            .collect()
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_CAPTION
    }

    fn version(&self) -> String {
        self.model.clone()
    }
}

impl BatchCaptionCliBackend {
    fn command(&self) -> Command {
        let mut command = if let Some(python) = &self.python {
            let mut command = Command::new(python);
            command.arg(&self.bin);
            command
        } else {
            Command::new(&self.bin)
        };
        command
            .arg("--model")
            .arg(&self.model_dir)
            .arg("--device")
            .arg(&self.device)
            .arg("--torch-dtype")
            .arg(&self.torch_dtype)
            .arg("--max-new-tokens")
            .arg(self.max_new_tokens.to_string())
            .arg("--prompt")
            .arg(&self.prompt);
        command
    }

    fn spawn_error(&self, error: &std::io::Error) -> Error {
        Error::Backend(format!(
            "could not run `{}`: {error}. Install the local caption CLI or pass --from-json.",
            self.bin.display()
        ))
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
