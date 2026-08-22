//! Packaged Lege OCR process boundary.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::docir::Document;
use crate::{Error, Result};

/// Something that turns an untouched PDF into canonical DocIR.
pub trait DocumentBackend {
    /// Process `original`, retaining all artifacts in `artifacts`.
    fn process(&self, original: &Path, artifacts: &Path) -> Result<(PathBuf, Document)>;
}

/// Runs the separately packaged `lege-ocr` product.
#[derive(Debug, Clone)]
pub struct LegeOcrCliBackend {
    /// Executable name or path.
    pub bin: PathBuf,
    /// Optional packaged TensorRT OCR root.
    pub tensorrt_root: Option<PathBuf>,
}

/// Runs Lege with its explicit Evidence-broker OCR provider.
#[derive(Debug, Clone)]
pub struct BrokeredLegeOcrCliBackend {
    /// Packaged Lege CLI.
    pub bin: PathBuf,
    /// Lege-protocol-to-Evidence bridge executable.
    pub bridge: PathBuf,
    /// Current-user broker endpoint.
    pub endpoint: String,
    /// Installed OCR model identifier.
    pub model: String,
    /// Immutable model revision.
    pub revision: String,
}

impl Default for LegeOcrCliBackend {
    fn default() -> Self {
        Self {
            bin: PathBuf::from("lege-ocr"),
            tensorrt_root: None,
        }
    }
}

impl DocumentBackend for LegeOcrCliBackend {
    fn process(&self, original: &Path, artifacts: &Path) -> Result<(PathBuf, Document)> {
        std::fs::create_dir_all(artifacts)?;
        let mut command = Command::new(&self.bin);
        command
            .arg("batch")
            .arg(original)
            .arg("--output")
            .arg(artifacts)
            .arg("--profile")
            .arg("search")
            .arg("--backend")
            .arg("tensorrt-paddle")
            .arg("--format")
            .arg("json")
            .arg("--workers")
            .arg("1")
            .arg("--on-error")
            .arg("stop");
        if let Some(root) = &self.tensorrt_root {
            command.arg("--tensorrt-ocr-root").arg(root);
        }
        let output = command.output().map_err(|error| {
            Error::Backend(format!("could not run `{}`: {error}", self.bin.display()))
        })?;
        if !output.status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {}",
                self.bin.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let candidates = docir_files(artifacts)?;
        if candidates.len() != 1 {
            return Err(Error::Backend(format!(
                "expected one .lege.json artifact below {}, found {}",
                artifacts.display(),
                candidates.len()
            )));
        }
        let path = candidates.into_iter().next().expect("length checked");
        let document = Document::read(&path)?;
        Ok((path, document))
    }
}

impl DocumentBackend for BrokeredLegeOcrCliBackend {
    fn process(&self, original: &Path, artifacts: &Path) -> Result<(PathBuf, Document)> {
        std::fs::create_dir_all(artifacts)?;
        let output = Command::new(&self.bin)
            .arg("batch")
            .arg(original)
            .arg("--output")
            .arg(artifacts)
            .arg("--profile")
            .arg("search")
            .arg("--backend")
            .arg("brokered-tensorrt")
            .arg("--broker-bridge")
            .arg(&self.bridge)
            .arg("--broker-endpoint")
            .arg(&self.endpoint)
            .arg("--broker-model")
            .arg(&self.model)
            .arg("--broker-revision")
            .arg(&self.revision)
            .arg("--format")
            .arg("json")
            .arg("--resume")
            .arg("--workers")
            .arg("1")
            .arg("--on-error")
            .arg("stop")
            .output()
            .map_err(|error| {
                Error::Backend(format!("could not run `{}`: {error}", self.bin.display()))
            })?;
        if !output.status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {}",
                self.bin.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        read_single_docir(artifacts)
    }
}

fn read_single_docir(artifacts: &Path) -> Result<(PathBuf, Document)> {
    let candidates = docir_files(artifacts)?;
    if candidates.len() != 1 {
        return Err(Error::Backend(format!(
            "expected one .lege.json artifact below {}, found {}",
            artifacts.display(),
            candidates.len()
        )));
    }
    let path = candidates.into_iter().next().expect("length checked");
    let document = Document::read(&path)?;
    Ok((path, document))
}

fn docir_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".lege.json"))
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    Ok(matches)
}
