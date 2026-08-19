//! Keyframe embeddings. A visual finder, not a finding.

use std::path::{Path, PathBuf};
use std::process::Command;

use evidence_intake::{IndexedKeyframe, KeyframeIndex};
use serde::{Deserialize, Serialize};

use crate::map::VideoIdentity;
use crate::scene::SceneAnalysis;
use crate::{Error, Result};

/// Extractor name stored with each indexed vector.
pub const EXTRACTOR_EMBED: &str = "keyframe_embed";

/// Something that can embed a still and a text query into one space.
pub trait EmbeddingBackend {
    /// Embed one working-copy still.
    fn embed_still(&self, still: &Path) -> Result<Vec<f32>>;

    /// Embed a text query into the same space as [`Self::embed_still`].
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;

    /// Encoder name stored with each vector.
    fn extractor(&self) -> &str;

    /// Encoder version stored with each vector.
    fn version(&self) -> String;

    /// Embedding space these vectors belong to.
    fn model(&self) -> String;
}

/// A JSON document of still and query vectors, as produced by a prior run or a
/// test fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EmbeddingDocument {
    /// Embedding space name stored with each vector.
    #[serde(default)]
    pub model: Option<String>,
    /// Backend name stored with each vector.
    #[serde(default)]
    pub extractor: Option<String>,
    /// Backend version stored with each vector.
    #[serde(default)]
    pub version: Option<String>,
    /// Per-still vectors.
    #[serde(default)]
    pub stills: Vec<StillEmbedding>,
    /// Per-query vectors, for tests and overnight query scripts.
    #[serde(default)]
    pub queries: Vec<QueryEmbedding>,
}

/// One still's vector, addressed by scene index, path, or already-known id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StillEmbedding {
    /// One-based scene index, matching [`crate::Scene::index`].
    #[serde(default)]
    pub scene_index: Option<u32>,
    /// Working-copy jpeg path, matched by file name.
    #[serde(default)]
    pub path: Option<String>,
    /// Derived still source id, when the caller already knows it.
    #[serde(default)]
    pub source_id: Option<String>,
    /// Coordinates in the document's model space.
    pub vector: Vec<f32>,
}

/// One text query's vector in the same space as the stills.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryEmbedding {
    /// Query text, matched exactly after trimming.
    pub text: String,
    /// Coordinates in the document's model space.
    pub vector: Vec<f32>,
}

/// Reads an [`EmbeddingDocument`] from disk.
#[derive(Debug, Clone)]
pub struct JsonEmbeddingBackend {
    /// Path to the JSON document.
    pub path: PathBuf,
}

impl JsonEmbeddingBackend {
    /// Load still and query vectors. The stills on disk are not consulted.
    pub fn load(&self) -> Result<EmbeddingDocument> {
        let json = std::fs::read_to_string(&self.path).map_err(|error| {
            Error::Backend(format!("could not read {}: {error}", self.path.display()))
        })?;
        serde_json::from_str(&json).map_err(|error| {
            Error::Backend(format!(
                "{} is not an embedding document: {error}",
                self.path.display()
            ))
        })
    }
}

impl EmbeddingBackend for JsonEmbeddingBackend {
    fn embed_still(&self, still: &Path) -> Result<Vec<f32>> {
        let document = self.load()?;
        let file_name = still
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        document
            .stills
            .iter()
            .find(|entry| {
                entry.path.as_deref().is_some_and(|path| {
                    Path::new(path).file_name().and_then(|n| n.to_str()) == Some(file_name)
                })
            })
            .map(|entry| entry.vector.clone())
            .ok_or_else(|| {
                Error::Backend(format!(
                    "{} has no vector for still `{}`",
                    self.path.display(),
                    still.display()
                ))
            })
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let document = self.load()?;
        let wanted = text.trim();
        document
            .queries
            .iter()
            .find(|entry| entry.text.trim() == wanted)
            .map(|entry| entry.vector.clone())
            .ok_or_else(|| {
                Error::Backend(format!(
                    "{} has no vector for query `{wanted}`",
                    self.path.display()
                ))
            })
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_EMBED
    }

    fn version(&self) -> String {
        "from-json".to_owned()
    }

    fn model(&self) -> String {
        self.load()
            .ok()
            .and_then(|document| document.model)
            .unwrap_or_else(|| "from-json".to_owned())
    }
}

/// Shells out to a local embedding CLI.
///
/// Expected interface:
/// `embed-cli still <path>` and `embed-cli query <text>`, each printing
/// `{"vector":[...]}` on stdout. Missing binary is a hard error with an
/// install hint. Overnight, not realtime.
#[derive(Debug, Clone)]
pub struct CliEmbeddingBackend {
    /// Binary name or path. Default `embed-cli`.
    pub bin: PathBuf,
    /// Embedding space name stored with each vector.
    pub model: String,
}

impl CliEmbeddingBackend {
    /// `embed-cli` on PATH, generic CLIP space name.
    pub fn default_local() -> Self {
        Self {
            bin: PathBuf::from("embed-cli"),
            model: "clip".to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct CliVector {
    vector: Vec<f32>,
}

impl EmbeddingBackend for CliEmbeddingBackend {
    fn embed_still(&self, still: &Path) -> Result<Vec<f32>> {
        self.run(["still", still.to_str().unwrap_or_default()])
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.run(["query", text])
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_EMBED
    }

    fn version(&self) -> String {
        self.model.clone()
    }

    fn model(&self) -> String {
        self.model.clone()
    }
}

impl CliEmbeddingBackend {
    fn run<const N: usize>(&self, args: [&str; N]) -> Result<Vec<f32>> {
        let output = Command::new(&self.bin).args(args).output().map_err(|error| {
            Error::Backend(format!(
                "could not run `{}`: {error}. Install a local CLIP/SigLIP CLI or pass --from-json.",
                self.bin.display()
            ))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {stderr}",
                self.bin.display(),
                output.status
            )));
        }
        let parsed: CliVector = serde_json::from_slice(&output.stdout).map_err(|error| {
            Error::Backend(format!(
                "`{}` did not print {{\"vector\":[...]}}: {error}",
                self.bin.display()
            ))
        })?;
        if parsed.vector.is_empty() {
            return Err(Error::Backend(format!(
                "`{}` printed an empty vector",
                self.bin.display()
            )));
        }
        Ok(parsed.vector)
    }
}

/// Embed each scene keyframe and address the vectors to the derived still ids
/// [`crate::scenes_to_batch`] will have used.
pub fn embed_keyframes(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
    backend: &dyn EmbeddingBackend,
) -> Result<KeyframeIndex> {
    let mut embeddings = Vec::new();
    for scene in &analysis.scenes {
        let Some(still) = &scene.keyframe else {
            continue;
        };
        let vector = backend.embed_still(&still.path)?;
        embeddings.push(IndexedKeyframe {
            source_id: format!("{}-still-{:04}", identity.source_id, scene.index),
            model: backend.model(),
            extractor: backend.extractor().to_owned(),
            version: backend.version(),
            vector,
        });
    }
    if embeddings.is_empty() {
        return Err(Error::Mapping("no keyframes to embed".to_owned()));
    }
    Ok(KeyframeIndex {
        case_id: identity.case_id.clone(),
        embeddings,
    })
}

/// Map a JSON embedding document onto the still ids of `analysis`.
pub fn embed_from_document(
    identity: &VideoIdentity,
    analysis: &SceneAnalysis,
    document: &EmbeddingDocument,
) -> Result<KeyframeIndex> {
    let model = document
        .model
        .clone()
        .unwrap_or_else(|| "from-json".to_owned());
    let extractor = document
        .extractor
        .clone()
        .unwrap_or_else(|| EXTRACTOR_EMBED.to_owned());
    let version = document
        .version
        .clone()
        .unwrap_or_else(|| "from-json".to_owned());
    let mut embeddings = Vec::new();
    for scene in &analysis.scenes {
        let Some(still) = &scene.keyframe else {
            continue;
        };
        let source_id = format!("{}-still-{:04}", identity.source_id, scene.index);
        let file_name = still
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let entry = document.stills.iter().find(|candidate| {
            candidate.source_id.as_deref() == Some(source_id.as_str())
                || candidate.scene_index == Some(scene.index)
                || candidate.path.as_deref().is_some_and(|path| {
                    Path::new(path).file_name().and_then(|name| name.to_str()) == Some(file_name)
                })
        });
        let Some(entry) = entry else {
            return Err(Error::Mapping(format!(
                "embedding document has no vector for still `{source_id}` (scene {})",
                scene.index
            )));
        };
        embeddings.push(IndexedKeyframe {
            source_id,
            model: model.clone(),
            extractor: extractor.clone(),
            version: version.clone(),
            vector: entry.vector.clone(),
        });
    }
    if embeddings.is_empty() {
        return Err(Error::Mapping("no keyframes to embed".to_owned()));
    }
    Ok(KeyframeIndex {
        case_id: identity.case_id.clone(),
        embeddings,
    })
}
