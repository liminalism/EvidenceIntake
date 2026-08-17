//! Decode an original wav and hash the untouched bytes.

use std::path::Path;

use hound::{SampleFormat, WavReader};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Interleaved PCM in `[-1.0, 1.0]`, plus the original file's identity.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// SHA-256 of the untouched file bytes, lowercase hex.
    pub sha256: String,
    /// Length of the untouched file.
    pub byte_length: u64,
    /// MIME type. Wav decode always reports `audio/wav`.
    pub media_type: String,
    /// Sample rate of the original.
    pub sample_rate: u32,
    /// Channel count of the original.
    pub channels: u16,
    /// Interleaved samples, one frame at a time.
    pub samples: Vec<f32>,
}

impl DecodedAudio {
    /// Number of frames (not samples).
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        }
    }

    /// Duration of the original in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.frames() as u64 * 1_000) / u64::from(self.sample_rate)
    }

    /// Samples of one channel, in `[-1.0, 1.0]`.
    pub fn channel(&self, index: usize) -> Result<Vec<f32>> {
        let channels = usize::from(self.channels);
        if index >= channels {
            return Err(Error::Empty(format!(
                "channel {index} does not exist on a {channels}-channel file"
            )));
        }
        Ok(self
            .samples
            .iter()
            .skip(index)
            .step_by(channels)
            .copied()
            .collect())
    }

    /// Interleaved samples covering `[start_ms, end_ms)` on the original timeline.
    pub fn window(&self, start_ms: u64, end_ms: u64) -> Self {
        let start = self.frame_at(start_ms);
        let end = self.frame_at(end_ms).max(start);
        let channels = usize::from(self.channels);
        let from = start.saturating_mul(channels);
        let to = end.saturating_mul(channels).min(self.samples.len());
        Self {
            sha256: self.sha256.clone(),
            byte_length: self.byte_length,
            media_type: self.media_type.clone(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            samples: self.samples.get(from..to).unwrap_or(&[]).to_vec(),
        }
    }

    /// Mixes every channel into a single mono stream. Stereo analysis is done
    /// on the original; the mix is only a working copy for cleanup / ASR.
    pub fn mix_mono(&self) -> Self {
        if self.channels <= 1 {
            return self.clone();
        }
        let channels = usize::from(self.channels);
        let samples = self
            .samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
        Self {
            sha256: self.sha256.clone(),
            byte_length: self.byte_length,
            media_type: self.media_type.clone(),
            sample_rate: self.sample_rate,
            channels: 1,
            samples,
        }
    }

    fn frame_at(&self, ms: u64) -> usize {
        let frame = (ms * u64::from(self.sample_rate)) / 1_000;
        usize::try_from(frame)
            .unwrap_or(usize::MAX)
            .min(self.frames())
    }
}

/// Reads a wav, hashes the original bytes, and returns PCM in `[-1.0, 1.0]`.
pub fn decode_wav(path: &Path) -> Result<DecodedAudio> {
    let bytes = std::fs::read(path).map_err(|error| Error::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if bytes.is_empty() {
        return Err(Error::Empty(format!("`{}` is empty", path.display())));
    }
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let byte_length = bytes.len() as u64;

    let cursor = std::io::Cursor::new(bytes);
    let mut reader = WavReader::new(cursor).map_err(|error| Error::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err(Error::Decode {
            path: path.to_path_buf(),
            message: "wav header has zero channels or sample rate".to_owned(),
        });
    }

    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => read_int_samples(&mut reader, 127.0)?,
        (SampleFormat::Int, 16) => read_int_samples(&mut reader, f32::from(i16::MAX))?,
        (SampleFormat::Int, 24 | 32) => read_int_samples(&mut reader, i32::MAX as f32)?,
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?,
        (format, bits) => {
            return Err(Error::Decode {
                path: path.to_path_buf(),
                message: format!("unsupported wav format {format:?} / {bits}-bit"),
            });
        }
    };

    if samples.is_empty() {
        return Err(Error::Empty(format!(
            "`{}` contains no samples",
            path.display()
        )));
    }

    Ok(DecodedAudio {
        sha256,
        byte_length,
        media_type: "audio/wav".to_owned(),
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

fn read_int_samples<R: std::io::Read>(reader: &mut WavReader<R>, scale: f32) -> Result<Vec<f32>> {
    let path_hint = "wav";
    reader
        .samples::<i32>()
        .map(|sample| {
            sample
                .map(|value| value as f32 / scale)
                .map_err(|error| Error::Decode {
                    path: Path::new(path_hint).to_path_buf(),
                    message: error.to_string(),
                })
        })
        .collect()
}

/// Writes PCM as a 16-bit wav. Used only for the working copy handed to WhisperX.
pub fn write_wav(path: &Path, audio: &DecodedAudio) -> Result<()> {
    let spec = hound::WavSpec {
        channels: audio.channels,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in &audio.samples {
        let clipped = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        writer.write_sample(clipped)?;
    }
    writer.finalize()?;
    Ok(())
}
