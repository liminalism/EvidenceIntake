//! Speech recognition through the shared Evidence inference broker.
//!
//! The broker serves whichever `transcribe_audio` pack the caller names. The
//! default is Phonon-2 (`FermionResearch/Phonon-2`), a 164 MB English-only
//! CPU model served by `evidence-trt-phonon-worker`; a Whisper pack with the
//! same typed contract still works when installed. The adapter sends 16 kHz
//! mono `f32` PCM and receives timed segments, so nothing here depends on
//! which engine answered beyond the pack id and revision stamped on each
//! statement.

use std::path::Path;

use evidence_trt::{Client, InputMetadata, Operation, Request, ResultBody, TranscriptSegment};

use crate::decode::decode_wav;
use crate::whisperx::{WhisperxSegment, WhisperxTranscript, WhisperxWord};
use crate::{Error, Result, TranscriptBackend};

/// Extractor name stamped on broker-produced speech, whichever pack served it.
pub const EXTRACTOR_BROKER_ASR: &str = "broker_asr";
/// Extractor name older databases carry from the Whisper-only broker.
pub const EXTRACTOR_TRT_WHISPER: &str = "tensorrt_whisper";
/// Sample rate every broker speech pack receives.
pub const ASR_RATE: u32 = 16_000;

/// Speech backend served by the Evidence inference broker.
#[derive(Debug, Clone)]
pub struct TrtSpeechBackend {
    /// Local broker endpoint.
    pub endpoint: String,
    /// Checksum-pinned model-pack identifier, normally `phonon-2`.
    pub model: String,
    /// Immutable model/export revision.
    pub revision: String,
    /// Optional language hint. Phonon-2 refuses anything but English.
    pub language: Option<String>,
}

impl TranscriptBackend for TrtSpeechBackend {
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript> {
        let decoded = decode_wav(wav)?;
        let mono = decoded.mix_mono();
        let pcm = if mono.sample_rate == ASR_RATE {
            mono
        } else {
            crate::resample_to(&mono, ASR_RATE)?
        };
        let mut payload = Vec::with_capacity(pcm.samples.len() * size_of::<f32>());
        for sample in pcm.samples {
            payload.extend_from_slice(&sample.to_le_bytes());
        }
        let mut client = Client::connect(&self.endpoint)
            .map_err(|error| Error::Backend(format!("inference broker: {error}")))?;
        let result = client
            .request(
                Request::Infer {
                    model: self.model.clone(),
                    revision: self.revision.clone(),
                    operation: Operation::TranscribeAudio,
                    input: InputMetadata {
                        media_type: Some("audio/x-f32le".to_owned()),
                        sample_rate: Some(ASR_RATE),
                        channels: Some(1),
                        language: self.language.clone(),
                        ..InputMetadata::default()
                    },
                },
                &payload,
            )
            .map_err(|error| Error::Backend(format!("inference broker: {error}")))?;
        let ResultBody::Transcript { segments } = result else {
            return Err(Error::Backend(
                "inference broker returned a non-transcript result".to_owned(),
            ));
        };
        transcript_from_segments(segments, self.language.clone())
    }

    fn version(&self) -> String {
        format!("{EXTRACTOR_BROKER_ASR}@{}/{}", self.model, self.revision)
    }

    fn extractor(&self) -> &'static str {
        EXTRACTOR_BROKER_ASR
    }
}

/// Fold typed broker segments into the adapter's transcript shape.
///
/// Speaker labels, when a pack produces any, must stay anonymous
/// (`SPEAKER_nn`); a pack that names people is refused rather than mapped.
/// Confidence is carried through as a log probability only when the pack
/// reported one, so a pack that reports none (Phonon-2) yields `None`.
pub fn transcript_from_segments(
    segments: Vec<TranscriptSegment>,
    language: Option<String>,
) -> Result<WhisperxTranscript> {
    Ok(WhisperxTranscript {
        segments: segments
            .into_iter()
            .map(|segment| {
                if let Some(speaker) = &segment.speaker
                    && (!speaker.starts_with("SPEAKER_")
                        || !speaker[8..].bytes().all(|byte| byte.is_ascii_digit()))
                {
                    return Err(Error::Backend(format!(
                        "inference broker returned non-anonymous speaker label `{speaker}`"
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
        language,
    })
}

#[cfg(test)]
mod tests {
    use evidence_trt::{TranscriptSegment, TranscriptWord};

    use super::{EXTRACTOR_BROKER_ASR, TrtSpeechBackend, transcript_from_segments};
    use crate::TranscriptBackend;

    fn segment(speaker: Option<&str>, confidence: Option<f32>) -> TranscriptSegment {
        TranscriptSegment {
            start_ms: 8_200,
            end_ms: 9_000,
            text: "It just happened.".to_owned(),
            confidence,
            words: vec![TranscriptWord {
                start_ms: 8_200,
                end_ms: 8_400,
                text: "It".to_owned(),
                confidence: None,
            }],
            speaker: speaker.map(str::to_owned),
        }
    }

    #[test]
    fn a_pack_that_reports_no_confidence_yields_none_rather_than_a_number() {
        let transcript =
            transcript_from_segments(vec![segment(None, None)], Some("en".to_owned())).unwrap();
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(transcript.segments[0].avg_logprob, None);
        assert_eq!(transcript.segments[0].words[0].score, None);
        assert_eq!(transcript.segments[0].speaker, None);
        assert!((transcript.segments[0].start - 8.2).abs() < 1e-9);
    }

    #[test]
    fn only_anonymous_speaker_labels_cross_the_broker_boundary() {
        assert!(transcript_from_segments(vec![segment(Some("SPEAKER_01"), None)], None).is_ok());
        assert!(transcript_from_segments(vec![segment(Some("Officer Lee"), None)], None).is_err());
    }

    #[test]
    fn the_version_names_the_pack_and_its_revision() {
        let backend = TrtSpeechBackend {
            endpoint: "evidence-trt".to_owned(),
            model: "phonon-2".to_owned(),
            revision: "98125795".to_owned(),
            language: Some("en".to_owned()),
        };
        assert_eq!(
            backend.version(),
            format!("{EXTRACTOR_BROKER_ASR}@phonon-2/98125795")
        );
    }
}
