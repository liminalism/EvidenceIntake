//! Versioned typed broker protocol.

use serde::{Deserialize, Serialize};

/// Wire protocol discriminator.
pub const PROTOCOL: &str = "evidence.trt";
/// Current wire protocol version.
pub const PROTOCOL_VERSION: u32 = 2;

/// One request sent to the broker or a supervised worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestEnvelope {
    /// Protocol discriminator.
    pub protocol: String,
    /// Protocol version.
    pub version: u32,
    /// Caller-generated request identifier.
    pub id: u64,
    /// Typed operation.
    pub request: Request,
}

impl RequestEnvelope {
    /// Build a current-protocol request.
    pub fn new(id: u64, request: Request) -> Self {
        Self {
            protocol: PROTOCOL.to_owned(),
            version: PROTOCOL_VERSION,
            id,
            request,
        }
    }

    /// Reject an incompatible protocol before dispatch.
    pub fn validate(&self) -> crate::Result<()> {
        if self.protocol != PROTOCOL || self.version != PROTOCOL_VERSION {
            return Err(crate::Error::IncompatibleProtocol(format!(
                "expected {PROTOCOL}/{PROTOCOL_VERSION}, got {}/{}",
                self.protocol, self.version
            )));
        }
        Ok(())
    }
}

/// Broker control or inference request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Return runtime and GPU health.
    Health,
    /// List installed model identities and operations.
    Models,
    /// Load and probe a model.
    Load {
        /// Installed model identifier.
        model: String,
    },
    /// Release a resident model.
    Unload {
        /// Installed model identifier.
        model: String,
    },
    /// Run one typed inference. Input bytes are the frame payload.
    Infer {
        /// Installed model identifier.
        model: String,
        /// Immutable export revision the caller requires.
        revision: String,
        /// Typed inference operation.
        operation: Operation,
        /// Input metadata required by preprocessing.
        input: InputMetadata,
    },
}

/// Supported model operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// OCR one raster page.
    PageOcr,
    /// Embed one image.
    EmbedImage,
    /// Embed UTF-8 text.
    EmbedText,
    /// Detect objects in one image.
    DetectImage,
    /// Transcribe mono PCM audio.
    TranscribeAudio,
    /// Generate a bounded caption for one image.
    CaptionImage,
}

impl std::fmt::Display for Operation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PageOcr => "page_ocr",
            Self::EmbedImage => "embed_image",
            Self::EmbedText => "embed_text",
            Self::DetectImage => "detect_image",
            Self::TranscribeAudio => "transcribe_audio",
            Self::CaptionImage => "caption_image",
        })
    }
}

/// Metadata describing an inference payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct InputMetadata {
    /// Media type of encoded image/document bytes.
    pub media_type: Option<String>,
    /// Raster width when payload is an image.
    pub width: Option<u32>,
    /// Raster height when payload is an image.
    pub height: Option<u32>,
    /// Interleaved image channel count.
    pub channels: Option<u8>,
    /// Audio sample rate when payload is PCM.
    pub sample_rate: Option<u32>,
    /// Language hint for OCR or transcription.
    pub language: Option<String>,
    /// Immutable prompt for caption generation.
    pub prompt: Option<String>,
}

/// One response from the broker or worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseEnvelope {
    /// Protocol discriminator.
    pub protocol: String,
    /// Protocol version.
    pub version: u32,
    /// Matching request identifier.
    pub id: u64,
    /// Success or failure.
    pub response: Response,
}

impl ResponseEnvelope {
    /// Build a current-protocol response.
    pub fn new(id: u64, response: Response) -> Self {
        Self {
            protocol: PROTOCOL.to_owned(),
            version: PROTOCOL_VERSION,
            id,
            response,
        }
    }

    /// Validate protocol and request correlation.
    pub fn validate(&self, request_id: u64) -> crate::Result<()> {
        if self.protocol != PROTOCOL || self.version != PROTOCOL_VERSION {
            return Err(crate::Error::IncompatibleProtocol(format!(
                "expected {PROTOCOL}/{PROTOCOL_VERSION}, got {}/{}",
                self.protocol, self.version
            )));
        }
        if self.id != request_id {
            return Err(crate::Error::IncompatibleProtocol(format!(
                "response id {} did not match request {request_id}",
                self.id
            )));
        }
        Ok(())
    }
}

/// Successful result or explicit fail-closed error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    /// Request completed successfully.
    Ok {
        /// Typed response body.
        result: ResultBody,
    },
    /// Request failed; the broker never retries it through another backend.
    Error {
        /// Stable error category.
        code: String,
        /// Human-readable diagnostic.
        message: String,
    },
}

/// Typed successful response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultBody {
    /// Broker or worker health.
    Health(Health),
    /// Installed models.
    Models {
        /// Installed model summaries.
        models: Vec<ModelSummary>,
    },
    /// A model was loaded and probed.
    Loaded {
        /// Model identifier.
        model: String,
    },
    /// A model was unloaded.
    Unloaded {
        /// Model identifier.
        model: String,
    },
    /// Page OCR lines.
    PageOcr {
        /// Detected and recognized lines.
        lines: Vec<OcrLine>,
    },
    /// One normalized embedding.
    Embedding {
        /// Embedding coordinates.
        vector: Vec<f32>,
    },
    /// Object detections.
    Detections {
        /// Detected boxes.
        detections: Vec<Detection>,
    },
    /// Speech transcript segments.
    Transcript {
        /// Timed segments.
        segments: Vec<TranscriptSegment>,
    },
    /// Bounded caption or explicit abstention.
    Caption {
        /// `None` means the model abstained.
        text: Option<String>,
    },
}

/// Broker health and runtime identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Health {
    /// Whether the broker can accept requests.
    pub ready: bool,
    /// Selected GPU name.
    pub gpu: String,
    /// `TensorRT` runtime version.
    pub tensorrt_version: String,
    /// `TensorRT-LLM` runtime version, when installed.
    pub tensorrt_llm_version: Option<String>,
}

/// Installed model information returned without filesystem details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSummary {
    /// Stable model identifier.
    pub id: String,
    /// Immutable model revision.
    pub revision: String,
    /// Supported typed operations.
    pub operations: Vec<Operation>,
    /// Whether the model is currently resident.
    pub resident: bool,
}

/// OCR line in source raster coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrLine {
    /// Raw recognized text.
    pub text: String,
    /// Recognition confidence.
    pub confidence: Option<f32>,
    /// `[x, y, width, height]` in input raster space.
    pub bounding_box: [u32; 4],
}

/// One object detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    /// Model label.
    pub label: String,
    /// Detection confidence.
    pub confidence: f32,
    /// `[x, y, width, height]` in input raster space.
    pub bounding_box: [f32; 4],
}

/// One timed speech segment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    /// Start on the original PCM timeline.
    pub start_ms: u64,
    /// End on the original PCM timeline.
    pub end_ms: u64,
    /// Raw transcript.
    pub text: String,
    /// Segment confidence.
    pub confidence: Option<f32>,
    /// Word-aligned output on the original PCM timeline.
    pub words: Vec<TranscriptWord>,
    /// Anonymous diarization label (`SPEAKER_nn`), when available.
    pub speaker: Option<String>,
}

/// One word-aligned speech token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptWord {
    /// Start on the original PCM timeline.
    pub start_ms: u64,
    /// End on the original PCM timeline.
    pub end_ms: u64,
    /// Raw token text.
    pub text: String,
    /// Alignment confidence.
    pub confidence: Option<f32>,
}
