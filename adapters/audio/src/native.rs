//! In-process Whisper via the `franken_whisper` native engine.
//!
//! Pure Rust: no C++ toolchain, no subprocess, no Python. The engine reads the
//! same ggml / gguf model files whisper.cpp does, so an operator's existing
//! `ggml-*.bin` keeps working. Only the engine is compiled in — the crate is
//! pulled with `default-features = false`, which leaves its CLI, orchestrator,
//! model downloader, and second SQLite out of this adapter.

use std::path::{Path, PathBuf};

use franken_whisper::native_engine::decode::{DecodeOutput, DecodeParams};
use franken_whisper::native_engine::{NativeWhisperModel, default_threads};

use crate::decode::decode_wav;
use crate::whisperx::{WhisperxSegment, WhisperxTranscript, WhisperxWord};
use crate::{EXTRACTOR_NATIVE_WHISPER, Error, Result, TranscriptBackend};

/// 16 kHz mono f32 is what the engine's mel front end expects.
const WHISPER_RATE: u32 = 16_000;

/// In-process Whisper. The model path is operator-supplied; we never download
/// weights, and the engine's own package resolver is never consulted.
#[derive(Debug, Clone)]
pub struct NativeWhisperBackend {
    /// Path to a ggml / gguf Whisper model (`ggml-base.bin`, …).
    pub model_path: PathBuf,
    /// Language hint. `None` lets a multilingual model detect.
    pub language: Option<String>,
}

impl NativeWhisperBackend {
    /// A backend pointed at a local model file, defaulting to English.
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            language: Some("en".to_owned()),
        }
    }

    /// `ggml-base.en.bin` → `base.en`. Stamped on every statement and handed to
    /// the engine so it can pick the right word-alignment heads for the model.
    fn model_name(&self) -> String {
        let stem = self
            .model_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("ggml");
        stem.strip_prefix("ggml-").unwrap_or(stem).to_owned()
    }
}

impl TranscriptBackend for NativeWhisperBackend {
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript> {
        let pcm = decode_wav(wav)?;
        let samples = pcm_16k_mono(&pcm)?;
        let model = NativeWhisperModel::load(&self.model_path).map_err(|error| {
            Error::Backend(format!(
                "could not load {}: {error}",
                self.model_path.display()
            ))
        })?;
        let params = DecodeParams {
            language: self.language.clone(),
            timestamps: true,
            word_timestamps: true,
            model_hint: Some(self.model_name()),
            n_threads: default_threads(),
            ..DecodeParams::default()
        };
        let output = model
            .transcribe(&samples, &params, &|| Ok(()))
            .map_err(|error| Error::Backend(format!("franken_whisper: {error}")))?;
        Ok(collect_transcript(output))
    }

    fn version(&self) -> String {
        format!(
            "{EXTRACTOR_NATIVE_WHISPER}@{}/{}",
            self.model_name(),
            franken_whisper::VERSION
        )
    }

    fn extractor(&self) -> &'static str {
        EXTRACTOR_NATIVE_WHISPER
    }
}

fn pcm_16k_mono(audio: &crate::DecodedAudio) -> Result<Vec<f32>> {
    let mono = audio.mix_mono();
    if mono.sample_rate == WHISPER_RATE {
        return Ok(mono.samples);
    }
    let resampled = crate::resample_to(&mono, WHISPER_RATE)?;
    Ok(resampled.samples)
}

/// Engine output → the WhisperX shape the mapper consumes.
///
/// The engine reports one confidence per segment as `exp(mean token logprob)`;
/// it is carried as `avg_logprob` (the inverse) so the mapper's segment-level
/// confidence path handles it. Word timings have no per-word score.
fn collect_transcript(output: DecodeOutput) -> WhisperxTranscript {
    let DecodeOutput {
        segments,
        language,
        word_timings,
        ..
    } = output;
    let mut timings = word_timings.map(Vec::into_iter);
    let mut mapped = Vec::with_capacity(segments.len());
    for segment in segments {
        let words = timings
            .as_mut()
            .and_then(Iterator::next)
            .unwrap_or_default();
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        let (Some(start), Some(end)) = (segment.start_sec, segment.end_sec) else {
            continue;
        };
        let words = words
            .into_iter()
            .filter_map(|word| {
                let text = word.text.trim();
                (!text.is_empty()).then(|| WhisperxWord {
                    word: text.to_owned(),
                    start: Some(word.start_sec),
                    end: Some(word.end_sec),
                    score: None,
                })
            })
            .collect();
        mapped.push(WhisperxSegment {
            start,
            end,
            text: text.to_owned(),
            words,
            speaker: None,
            avg_logprob: segment
                .confidence
                .filter(|confidence| *confidence > 0.0)
                .map(f64::ln),
        });
    }
    WhisperxTranscript {
        segments: mapped,
        language,
    }
}

#[cfg(test)]
mod tests {
    use franken_whisper::TranscriptionSegment;
    use franken_whisper::native_engine::decode::{DecodeOutput, DecodeWorkStats};
    use franken_whisper::native_engine::dtw::WordTiming;

    use super::collect_transcript;

    fn segment(start: f64, end: f64, text: &str, confidence: Option<f64>) -> TranscriptionSegment {
        TranscriptionSegment {
            start_sec: Some(start),
            end_sec: Some(end),
            text: text.to_owned(),
            speaker: None,
            confidence,
        }
    }

    fn word(text: &str, start: f64, end: f64) -> WordTiming {
        WordTiming {
            text: text.to_owned(),
            start_sec: start,
            end_sec: end,
        }
    }

    #[test]
    fn engine_output_keeps_segment_words_aligned_and_confidence_round_trips() {
        let output = DecodeOutput {
            segments: vec![
                segment(0.0, 1.5, " Ask not ", Some(0.5)),
                segment(1.5, 1.6, "   ", Some(0.9)),
                segment(1.6, 3.0, "what your country", None),
            ],
            language: Some("en".to_owned()),
            windows: Vec::new(),
            dropped_windows: Vec::new(),
            work: DecodeWorkStats::default(),
            word_timings: Some(vec![
                vec![word(" Ask", 0.0, 0.7), word(" not", 0.7, 1.5)],
                Vec::new(),
                vec![word(" what", 1.6, 2.0), word("", 2.0, 2.0)],
            ]),
        };

        let transcript = collect_transcript(output);

        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert_eq!(transcript.segments.len(), 2, "blank segments are dropped");
        let first = &transcript.segments[0];
        assert_eq!(first.text, "Ask not");
        assert_eq!(
            first
                .words
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>(),
            ["Ask", "not"]
        );
        assert_eq!(first.words[1].start, Some(0.7));
        assert!(first.words.iter().all(|w| w.score.is_none()));
        let logprob = first.avg_logprob.expect("confidence carried as logprob");
        assert!((logprob.exp() - 0.5).abs() < 1e-12);

        let third = &transcript.segments[1];
        assert_eq!(third.text, "what your country");
        assert_eq!(third.words.len(), 1, "empty word timings are dropped");
        assert_eq!(third.words[0].word, "what");
        assert!(third.avg_logprob.is_none());
    }
}
