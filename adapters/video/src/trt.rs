//! Typed video-model adapters for the Evidence TensorRT broker.

use std::path::{Path, PathBuf};
use std::process::Command;

use evidence_trt::{Client, InputMetadata, Operation, Request, ResultBody};

use crate::caption::{CaptionBackend, EXTRACTOR_CAPTION};
use crate::clock::{ClockBackend, OverlayBand, RawClockText};
use crate::embed::{EXTRACTOR_EMBED, EmbeddingBackend};
use crate::vision::{EXTRACTOR_DETECT, RawDetection, VisionBackend};
use crate::{Error, Result};

/// Scene-caption backend served by the local `TensorRT-LLM` worker.
#[derive(Debug, Clone)]
pub struct TrtCaptionBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned model-pack identifier.
    pub model: String,
    /// Immutable model/export revision stamped on suggestions.
    pub revision: String,
    /// Bounded scene-caption prompt.
    pub prompt: String,
}

impl CaptionBackend for TrtCaptionBackend {
    fn describe_still(&self, still: &Path) -> Result<Option<String>> {
        let mut client = connect(&self.endpoint)?;
        caption(
            &mut client,
            &self.model,
            &self.revision,
            &self.prompt,
            still,
        )
    }

    fn describe_stills(&self, stills: &[PathBuf]) -> Result<Vec<Option<String>>> {
        let mut client = connect(&self.endpoint)?;
        stills
            .iter()
            .map(|still| {
                caption(
                    &mut client,
                    &self.model,
                    &self.revision,
                    &self.prompt,
                    still,
                )
            })
            .collect()
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_CAPTION
    }

    fn version(&self) -> String {
        format!("{}@{}", self.model, self.revision)
    }
}

/// Image/text embedding backend served by core `TensorRT`.
#[derive(Debug, Clone)]
pub struct TrtEmbeddingBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned model-pack identifier and embedding-space name.
    pub model: String,
    /// Immutable model/export revision.
    pub revision: String,
}

impl EmbeddingBackend for TrtEmbeddingBackend {
    fn embed_still(&self, still: &Path) -> Result<Vec<f32>> {
        let mut client = connect(&self.endpoint)?;
        embed_image(&mut client, &self.model, &self.revision, still)
    }

    fn embed_stills(&self, stills: &[PathBuf]) -> Result<Vec<Vec<f32>>> {
        let mut client = connect(&self.endpoint)?;
        stills
            .iter()
            .map(|still| embed_image(&mut client, &self.model, &self.revision, still))
            .collect()
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Err(Error::Backend("embedding query is empty".to_owned()));
        }
        let mut client = connect(&self.endpoint)?;
        let result = client
            .request(
                Request::Infer {
                    model: self.model.clone(),
                    revision: self.revision.clone(),
                    operation: Operation::EmbedText,
                    input: InputMetadata {
                        media_type: Some("text/plain; charset=utf-8".to_owned()),
                        ..InputMetadata::default()
                    },
                },
                text.trim().as_bytes(),
            )
            .map_err(|error| trt_error(&error))?;
        embedding(result)
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_EMBED
    }

    fn version(&self) -> String {
        self.revision.clone()
    }

    fn model(&self) -> String {
        self.model.clone()
    }
}

/// Object detector served by core `TensorRT`.
#[derive(Debug, Clone)]
pub struct TrtVisionBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned detector model-pack identifier.
    pub model: String,
    /// Immutable model/export revision.
    pub revision: String,
    /// Minimum confidence retained from the worker.
    pub confidence: f64,
}

impl VisionBackend for TrtVisionBackend {
    fn detect_still(&self, still: &Path) -> Result<Vec<RawDetection>> {
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(Error::Backend(
                "TensorRT detector confidence is outside 0..=1".to_owned(),
            ));
        }
        let mut client = connect(&self.endpoint)?;
        let bytes = std::fs::read(still)?;
        let result = client
            .request(
                Request::Infer {
                    model: self.model.clone(),
                    revision: self.revision.clone(),
                    operation: Operation::DetectImage,
                    input: image_metadata(still),
                },
                &bytes,
            )
            .map_err(|error| trt_error(&error))?;
        let ResultBody::Detections { detections } = result else {
            return Err(unexpected("detections"));
        };
        Ok(detections
            .into_iter()
            .filter(|hit| f64::from(hit.confidence) >= self.confidence)
            .map(|hit| RawDetection {
                label: hit.label,
                bbox: hit.bounding_box.map(f64::from),
                normalized: false,
                confidence: Some(f64::from(hit.confidence)),
            })
            .collect())
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_DETECT
    }

    fn version(&self) -> String {
        format!("{}@{}", self.model, self.revision)
    }
}

/// Burned-in clock OCR served by the shared page-OCR model.
#[derive(Debug, Clone)]
pub struct TrtClockBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned OCR model-pack identifier.
    pub model: String,
    /// Immutable model/export revision.
    pub revision: String,
    /// OCR language.
    pub language: String,
}

impl ClockBackend for TrtClockBackend {
    fn read(&self, still: &Path) -> Result<Vec<RawClockText>> {
        let work = tempfile::tempdir()
            .map_err(|error| Error::Backend(format!("could not create clock crop dir: {error}")))?;
        let mut client = connect(&self.endpoint)?;
        let mut readings = Vec::new();
        for band in OverlayBand::all() {
            let crop = work.path().join(format!("{}.png", band.as_str()));
            crop_band(still, band, &crop)?;
            let bytes = std::fs::read(&crop)?;
            let result = client
                .request(
                    Request::Infer {
                        model: self.model.clone(),
                        revision: self.revision.clone(),
                        operation: Operation::PageOcr,
                        input: InputMetadata {
                            media_type: Some("image/png".to_owned()),
                            language: Some(self.language.clone()),
                            ..InputMetadata::default()
                        },
                    },
                    &bytes,
                )
                .map_err(|error| trt_error(&error))?;
            let ResultBody::PageOcr { lines } = result else {
                return Err(unexpected("page OCR"));
            };
            let text = lines
                .into_iter()
                .map(|line| line.text.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() {
                readings.push(RawClockText { band, text });
            }
        }
        Ok(readings)
    }

    fn version(&self) -> String {
        format!("{}@{}", self.model, self.revision)
    }
}

fn connect(endpoint: &str) -> Result<Client> {
    Client::connect(endpoint).map_err(|error| trt_error(&error))
}

fn caption(
    client: &mut Client,
    model: &str,
    revision: &str,
    prompt: &str,
    still: &Path,
) -> Result<Option<String>> {
    let bytes = std::fs::read(still)?;
    let result = client
        .request(
            Request::Infer {
                model: model.to_owned(),
                revision: revision.to_owned(),
                operation: Operation::CaptionImage,
                input: InputMetadata {
                    prompt: Some(prompt.to_owned()),
                    ..image_metadata(still)
                },
            },
            &bytes,
        )
        .map_err(|error| trt_error(&error))?;
    let ResultBody::Caption { text } = result else {
        return Err(unexpected("caption"));
    };
    match text {
        Some(text) if text.trim().is_empty() => Err(Error::Backend(
            "TensorRT caption worker returned empty text instead of abstaining".to_owned(),
        )),
        Some(text) => Ok(Some(text.trim().to_owned())),
        None => Ok(None),
    }
}

fn embed_image(client: &mut Client, model: &str, revision: &str, still: &Path) -> Result<Vec<f32>> {
    let bytes = std::fs::read(still)?;
    let result = client
        .request(
            Request::Infer {
                model: model.to_owned(),
                revision: revision.to_owned(),
                operation: Operation::EmbedImage,
                input: image_metadata(still),
            },
            &bytes,
        )
        .map_err(|error| trt_error(&error))?;
    embedding(result)
}

fn embedding(result: ResultBody) -> Result<Vec<f32>> {
    let ResultBody::Embedding { vector } = result else {
        return Err(unexpected("embedding"));
    };
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(Error::Backend(
            "TensorRT embedding worker returned an empty or non-finite vector".to_owned(),
        ));
    }
    Ok(vector)
}

fn image_metadata(path: &Path) -> InputMetadata {
    let media_type = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        _ => "image/jpeg",
    };
    InputMetadata {
        media_type: Some(media_type.to_owned()),
        ..InputMetadata::default()
    }
}

fn crop_band(still: &Path, band: OverlayBand, output: &Path) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(still)
        .args(["-vf", band.crop_expression(), "-frames:v", "1"])
        .arg(output)
        .status()
        .map_err(|error| Error::Backend(format!("could not crop clock overlay: {error}")))?;
    if !status.success() || !output.is_file() {
        return Err(Error::Backend(format!(
            "ffmpeg did not write the {} clock crop",
            band.as_str()
        )));
    }
    Ok(())
}

fn unexpected(expected: &str) -> Error {
    Error::Backend(format!(
        "TensorRT broker returned a result other than {expected}"
    ))
}

fn trt_error(error: &evidence_trt::Error) -> Error {
    Error::Backend(format!("TensorRT broker: {error}"))
}
