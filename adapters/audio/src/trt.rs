//! Whisper transcription through the shared TensorRT broker.

use std::path::Path;

use evidence_trt::{Client, InputMetadata, Operation, Request, ResultBody};

use crate::decode::decode_wav;
use crate::whisperx::{WhisperxSegment, WhisperxTranscript, WhisperxWord};
use crate::{Error, Result, TranscriptBackend};

/// Extractor name stamped on broker-produced speech.
pub const EXTRACTOR_TRT_WHISPER: &str = "tensorrt_whisper";
const WHISPER_RATE: u32 = 16_000;

/// Whisper backend served by core `TensorRT`.
#[derive(Debug, Clone)]
pub struct TrtWhisperBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned model-pack identifier.
    pub model: String,
    /// Immutable model/export revision.
    pub revision: String,
    /// Optional language hint.
    pub language: Option<String>,
}

impl TranscriptBackend for TrtWhisperBackend {
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript> {
        let decoded = decode_wav(wav)?;
        let mono = decoded.mix_mono();
        let pcm = if mono.sample_rate == WHISPER_RATE {
            mono
        } else {
            crate::resample_to(&mono, WHISPER_RATE)?
        };
        let mut payload = Vec::with_capacity(pcm.samples.len() * size_of::<f32>());
        for sample in pcm.samples {
            payload.extend_from_slice(&sample.to_le_bytes());
        }
        let mut client = Client::connect(&self.endpoint)
            .map_err(|error| Error::Backend(format!("TensorRT broker: {error}")))?;
        let result = client
            .request(
                Request::Infer {
                    model: self.model.clone(),
                    revision: self.revision.clone(),
                    operation: Operation::TranscribeAudio,
                    input: InputMetadata {
                        media_type: Some("audio/x-f32le".to_owned()),
                        sample_rate: Some(WHISPER_RATE),
                        channels: Some(1),
                        language: self.language.clone(),
                        ..InputMetadata::default()
                    },
                },
                &payload,
            )
            .map_err(|error| Error::Backend(format!("TensorRT broker: {error}")))?;
        let ResultBody::Transcript { segments } = result else {
            return Err(Error::Backend(
                "TensorRT broker returned a non-transcript result".to_owned(),
            ));
        };
        Ok(WhisperxTranscript {
            segments: segments
                .into_iter()
                .map(|segment| {
                    if let Some(speaker) = &segment.speaker
                        && (!speaker.starts_with("SPEAKER_")
                            || !speaker[8..].bytes().all(|byte| byte.is_ascii_digit()))
                    {
                        return Err(Error::Backend(format!(
                            "TensorRT broker returned non-anonymous speaker label `{speaker}`"
                        )));
                    }
                    Ok(WhisperxSegment {
                        start: segment.start_ms as f64 / 1_000.0,
                        end: segment.end_ms as f64 / 1_000.0,
                        text: segment.text,
                        words: segment
                            .words
                            .into_iter()
                            .map(|word| WhisperxWord {
                                word: word.text,
                                start: Some(word.start_ms as f64 / 1_000.0),
                                end: Some(word.end_ms as f64 / 1_000.0),
                                score: word.confidence.map(f64::from),
                            })
                            .collect(),
                        speaker: segment.speaker,
                        avg_logprob: segment
                            .confidence
                            .filter(|confidence| *confidence > 0.0)
                            .map(|confidence| f64::from(confidence).ln()),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            language: self.language.clone(),
        })
    }

    fn version(&self) -> String {
        format!("{EXTRACTOR_TRT_WHISPER}@{}/{}", self.model, self.revision)
    }

    fn extractor(&self) -> &'static str {
        EXTRACTOR_TRT_WHISPER
    }
}
