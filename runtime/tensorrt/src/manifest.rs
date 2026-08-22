//! Checksum-pinned model-pack manifests.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::protocol::Operation;
use crate::{Error, Result};

/// Current model-pack manifest version.
pub const MANIFEST_VERSION: u32 = 1;

/// Runtime implementation required by a model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Fixed-output core `TensorRT` worker.
    TensorRt,
    /// Autoregressive `TensorRT-LLM` worker.
    TensorRtLlm,
}

/// One installed model pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Stable local model identifier.
    pub id: String,
    /// Immutable upstream/export revision.
    pub revision: String,
    /// Model provider.
    pub provider: String,
    /// Model licence identifier.
    pub license: String,
    /// Native worker family.
    pub runtime: RuntimeKind,
    /// Worker executable relative to the pack root or absolute.
    pub worker: PathBuf,
    /// Worker arguments; no shell expansion occurs.
    #[serde(default)]
    pub worker_args: Vec<String>,
    /// Typed operations implemented by this pack.
    pub operations: Vec<Operation>,
    /// Conservative resident-memory estimate used for admission.
    pub estimated_vram_bytes: u64,
    /// Checksum-pinned artifacts.
    pub artifacts: Vec<Artifact>,
}

impl ModelManifest {
    /// Read and verify a `manifest.json` model pack.
    pub fn read(pack_root: &Path) -> Result<Self> {
        let bytes = std::fs::read(pack_root.join("manifest.json"))?;
        let manifest: Self = serde_json::from_slice(&bytes)?;
        manifest.verify(pack_root)?;
        Ok(manifest)
    }

    /// Validate identity, operations, paths, and every artifact checksum.
    pub fn verify(&self, pack_root: &Path) -> Result<()> {
        if self.schema_version != MANIFEST_VERSION {
            return Err(Error::InvalidManifest(format!(
                "model {} uses schema {}, expected {MANIFEST_VERSION}",
                self.id, self.schema_version
            )));
        }
        if self.id.trim().is_empty()
            || self.revision.trim().is_empty()
            || self.provider.trim().is_empty()
            || self.license.trim().is_empty()
        {
            return Err(Error::InvalidManifest(
                "model id, revision, provider and license are required".to_owned(),
            ));
        }
        if self.operations.is_empty() || self.artifacts.is_empty() || self.estimated_vram_bytes == 0
        {
            return Err(Error::InvalidManifest(format!(
                "model {} needs operations, artifacts and a nonzero VRAM estimate",
                self.id
            )));
        }
        let unique = self.operations.iter().copied().collect::<HashSet<_>>();
        if unique.len() != self.operations.len() {
            return Err(Error::InvalidManifest(format!(
                "model {} repeats an operation",
                self.id
            )));
        }
        let artifact_paths = self
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_path())
            .collect::<HashSet<_>>();
        if artifact_paths.len() != self.artifacts.len() {
            return Err(Error::InvalidManifest(format!(
                "model {} repeats an artifact path",
                self.id
            )));
        }
        if !artifact_paths.contains(self.worker.as_path()) {
            return Err(Error::InvalidManifest(format!(
                "model {} worker must be a checksummed pack artifact",
                self.id
            )));
        }
        for artifact in &self.artifacts {
            artifact.verify(pack_root)?;
        }
        let worker = resolve_pack_path(pack_root, &self.worker)?;
        if !worker.is_file() {
            return Err(Error::InvalidManifest(format!(
                "worker does not exist: {}",
                worker.display()
            )));
        }
        Ok(())
    }

    /// Resolve the native worker without allowing a relative path to escape its pack.
    pub fn worker_path(&self, pack_root: &Path) -> Result<PathBuf> {
        resolve_pack_path(pack_root, &self.worker)
    }
}

/// One checksummed model or tokenizer asset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    /// Artifact role such as `detector`, `tokenizer`, or `engine`.
    pub role: String,
    /// Path relative to the model pack.
    pub path: PathBuf,
    /// Lowercase SHA-256.
    pub sha256: String,
}

impl Artifact {
    fn verify(&self, pack_root: &Path) -> Result<()> {
        if self.role.trim().is_empty() || self.sha256.len() != 64 {
            return Err(Error::InvalidManifest(format!(
                "invalid artifact metadata for {}",
                self.path.display()
            )));
        }
        let path = resolve_pack_path(pack_root, &self.path)?;
        let bytes = std::fs::read(&path).map_err(|error| {
            Error::InvalidManifest(format!("could not read {}: {error}", path.display()))
        })?;
        let actual = format!("{:x}", Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(&self.sha256) {
            return Err(Error::InvalidManifest(format!(
                "checksum mismatch for {}: expected {}, got {actual}",
                path.display(),
                self.sha256
            )));
        }
        Ok(())
    }
}

fn resolve_pack_path(root: &Path, path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Err(Error::InvalidManifest(format!(
            "model-pack path must be relative: {}",
            path.display()
        )));
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    }) {
        return Err(Error::InvalidManifest(format!(
            "relative path escapes model pack: {}",
            path.display()
        )));
    }
    let canonical_root = root.canonicalize().map_err(|error| {
        Error::InvalidManifest(format!(
            "could not resolve model-pack root {}: {error}",
            root.display()
        ))
    })?;
    let candidate = root.join(path).canonicalize().map_err(|error| {
        Error::InvalidManifest(format!(
            "could not resolve model-pack path {}: {error}",
            path.display()
        ))
    })?;
    if !candidate.starts_with(&canonical_root) {
        return Err(Error::InvalidManifest(format!(
            "model-pack path resolves outside its pack: {}",
            path.display()
        )));
    }
    Ok(candidate)
}
