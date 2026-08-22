//! Model inventory, admission, residency, and fail-closed dispatch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::manifest::ModelManifest;
use crate::protocol::{
    Health, ModelSummary, Request, RequestEnvelope, Response, ResponseEnvelope, ResultBody,
};
use crate::worker::{ProcessWorker, Worker};
use crate::{Error, Result};

#[derive(Debug, Clone)]
struct InstalledModel {
    root: PathBuf,
    manifest: ModelManifest,
}

struct ResidentModel {
    worker: Box<dyn Worker>,
    last_used: u64,
}

/// Single-GPU broker with bounded model residency.
pub struct Broker {
    health: Health,
    vram_budget: u64,
    installed: HashMap<String, InstalledModel>,
    resident: HashMap<String, ResidentModel>,
    clock: u64,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Broker")
            .field("health", &self.health)
            .field("vram_budget", &self.vram_budget)
            .field("installed", &self.installed.keys())
            .field("resident", &self.resident.keys())
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl Broker {
    /// Build a broker from verified model-pack directories.
    pub fn discover(
        model_root: &Path,
        vram_budget: u64,
        gpu: impl Into<String>,
        tensorrt_version: impl Into<String>,
        tensorrt_llm_version: Option<String>,
    ) -> Result<Self> {
        let mut installed = HashMap::new();
        if model_root.is_dir() {
            for entry in std::fs::read_dir(model_root)? {
                let root = entry?.path();
                if !root.is_dir() || !root.join("manifest.json").is_file() {
                    continue;
                }
                let manifest = ModelManifest::read(&root)?;
                if installed.contains_key(&manifest.id) {
                    return Err(Error::InvalidManifest(format!(
                        "duplicate installed model id {}",
                        manifest.id
                    )));
                }
                installed.insert(manifest.id.clone(), InstalledModel { root, manifest });
            }
        }
        Ok(Self {
            health: Health {
                ready: true,
                gpu: gpu.into(),
                tensorrt_version: tensorrt_version.into(),
                tensorrt_llm_version,
            },
            vram_budget,
            installed,
            resident: HashMap::new(),
            clock: 0,
        })
    }

    /// Dispatch one validated request. Errors become explicit responses and are never retried.
    pub fn handle(&mut self, envelope: &RequestEnvelope, payload: &[u8]) -> ResponseEnvelope {
        let id = envelope.id;
        let response = envelope
            .validate()
            .and_then(|()| self.dispatch(envelope, payload));
        match response {
            Ok(result) => ResponseEnvelope::new(id, Response::Ok { result }),
            Err(error) => ResponseEnvelope::new(
                id,
                Response::Error {
                    code: error_code(&error).to_owned(),
                    message: error.to_string(),
                },
            ),
        }
    }

    fn dispatch(&mut self, envelope: &RequestEnvelope, payload: &[u8]) -> Result<ResultBody> {
        match &envelope.request {
            Request::Health => Ok(ResultBody::Health(self.health.clone())),
            Request::Models => Ok(ResultBody::Models {
                models: self.model_summaries(),
            }),
            Request::Load { model } => {
                self.ensure_resident(model, envelope.id)?;
                Ok(ResultBody::Loaded {
                    model: model.clone(),
                })
            }
            Request::Unload { model } => {
                self.resident.remove(model);
                Ok(ResultBody::Unloaded {
                    model: model.clone(),
                })
            }
            Request::Infer {
                model,
                revision,
                operation,
                ..
            } => {
                let installed = self
                    .installed
                    .get(model)
                    .ok_or_else(|| Error::ModelNotFound(model.clone()))?;
                if installed.manifest.revision != *revision {
                    return Err(Error::RevisionMismatch {
                        model: model.clone(),
                        requested: revision.clone(),
                        installed: installed.manifest.revision.clone(),
                    });
                }
                if !installed.manifest.operations.contains(operation) {
                    return Err(Error::UnsupportedOperation {
                        model: model.clone(),
                        operation: operation.to_string(),
                    });
                }
                self.ensure_resident(model, envelope.id)?;
                self.clock = self.clock.wrapping_add(1);
                let resident = self
                    .resident
                    .get_mut(model)
                    .ok_or_else(|| Error::Worker("model disappeared after load".to_owned()))?;
                resident.last_used = self.clock;
                let response = resident.worker.request(envelope, payload)?;
                match response.response {
                    Response::Ok { result } => Ok(result),
                    Response::Error { code, message } => {
                        Err(Error::Worker(format!("{code}: {message}")))
                    }
                }
            }
        }
    }

    fn ensure_resident(&mut self, model: &str, request_id: u64) -> Result<()> {
        if self.resident.contains_key(model) {
            return Ok(());
        }
        let installed = self
            .installed
            .get(model)
            .cloned()
            .ok_or_else(|| Error::ModelNotFound(model.to_owned()))?;
        let required = installed.manifest.estimated_vram_bytes;
        if required > self.vram_budget {
            return Err(Error::VramBudget {
                model: model.to_owned(),
                required,
                budget: self.vram_budget,
            });
        }
        while self.resident_vram() + required > self.vram_budget {
            let candidate = self
                .resident
                .iter()
                .min_by_key(|(_, resident)| resident.last_used)
                .map(|(id, _)| id.clone())
                .ok_or_else(|| Error::VramBudget {
                    model: model.to_owned(),
                    required,
                    budget: self.vram_budget,
                })?;
            self.resident.remove(&candidate);
        }
        let worker = ProcessWorker::start(&installed.root, &installed.manifest, request_id)?;
        self.clock = self.clock.wrapping_add(1);
        self.resident.insert(
            model.to_owned(),
            ResidentModel {
                worker: Box::new(worker),
                last_used: self.clock,
            },
        );
        Ok(())
    }

    fn resident_vram(&self) -> u64 {
        self.resident
            .keys()
            .filter_map(|id| self.installed.get(id))
            .map(|model| model.manifest.estimated_vram_bytes)
            .sum()
    }

    fn model_summaries(&self) -> Vec<ModelSummary> {
        let mut models = self
            .installed
            .values()
            .map(|model| ModelSummary {
                id: model.manifest.id.clone(),
                revision: model.manifest.revision.clone(),
                operations: model.manifest.operations.clone(),
                resident: self.resident.contains_key(&model.manifest.id),
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.id.cmp(&right.id));
        models
    }
}

fn error_code(error: &Error) -> &'static str {
    match error {
        Error::IncompatibleProtocol(_) => "incompatible_protocol",
        Error::ModelNotFound(_) => "model_not_found",
        Error::RevisionMismatch { .. } => "revision_mismatch",
        Error::UnsupportedOperation { .. } => "unsupported_operation",
        Error::VramBudget { .. } => "vram_budget",
        Error::InvalidManifest(_) => "invalid_manifest",
        Error::Worker(_) => "worker_failure",
        Error::Request(_) => "request_failure",
        Error::Io(_) | Error::Json(_) | Error::FrameTooLarge => "broker_failure",
    }
}
