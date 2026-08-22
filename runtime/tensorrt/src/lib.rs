//! Evidence-owned local inference broker contract.
//!
//! Application adapters use typed operations while specialized native workers
//! own `TensorRT` or `TensorRT-LLM` details. Native failures are isolated behind a
//! supervised process boundary and are never retried through another backend.

pub mod broker;
pub mod client;
pub mod frame;
pub mod manifest;
pub mod protocol;
pub mod server;
pub mod worker;

pub use broker::Broker;
pub use client::Client;
pub use error::{Error, Result};
pub use manifest::{Artifact, ModelManifest, RuntimeKind};
pub use protocol::{
    Detection, Health, InputMetadata, ModelSummary, OcrLine, Operation, Request, RequestEnvelope,
    Response, ResponseEnvelope, ResultBody, TranscriptSegment, TranscriptWord,
};

mod error;
