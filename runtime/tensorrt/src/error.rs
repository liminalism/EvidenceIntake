//! Broker errors.

/// Result returned by the `TensorRT` broker and client.
pub type Result<T> = std::result::Result<T, Error>;

/// Protocol, manifest, worker, or scheduling failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem, stream, or process I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A framed JSON header could not be encoded or decoded.
    #[error("protocol JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A frame was larger than the broker permits.
    #[error("protocol frame exceeds the configured limit")]
    FrameTooLarge,
    /// The peer speaks an incompatible protocol.
    #[error("incompatible protocol: {0}")]
    IncompatibleProtocol(String),
    /// Model-pack contents are invalid or fail verification.
    #[error("invalid model pack: {0}")]
    InvalidManifest(String),
    /// The requested model is not installed.
    #[error("model `{0}` is not installed")]
    ModelNotFound(String),
    /// The caller requested a different export revision than the installed pack.
    #[error("model `{model}` revision mismatch: requested `{requested}`, installed `{installed}`")]
    RevisionMismatch {
        /// Model identifier.
        model: String,
        /// Revision required by the caller.
        requested: String,
        /// Revision verified from the installed manifest.
        installed: String,
    },
    /// The model does not implement the requested typed operation.
    #[error("model `{model}` does not support {operation}")]
    UnsupportedOperation {
        /// Requested model identifier.
        model: String,
        /// Requested operation.
        operation: String,
    },
    /// VRAM policy cannot admit the requested model.
    #[error("model `{model}` needs {required} bytes but broker budget is {budget} bytes")]
    VramBudget {
        /// Model identifier.
        model: String,
        /// Estimated bytes required.
        required: u64,
        /// Configured broker budget.
        budget: u64,
    },
    /// A native worker failed or returned an invalid response.
    #[error("worker failure: {0}")]
    Worker(String),
    /// The broker returned an explicit inference failure.
    #[error("broker request failed: {0}")]
    Request(String),
}
