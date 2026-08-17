//! Channel and level hypotheses. These are observations, not people.

use crate::decode::DecodedAudio;

/// Which side carried most of the energy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSide {
    /// Left / channel 0.
    Left,
    /// Right / channel 1.
    Right,
}

/// A reviewable stereo-channel split over one millisecond span.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSplit {
    /// Span start on the original timeline.
    pub start_ms: u64,
    /// Span end on the original timeline.
    pub end_ms: u64,
    /// The louder side.
    pub side: ChannelSide,
    /// RMS of the left channel in the span.
    pub left_rms: f64,
    /// RMS of the right channel in the span.
    pub right_rms: f64,
    /// How clean the split is (`0..=1`), never who is speaking.
    pub confidence: f64,
}

/// A reviewable near-field / far-field level split over one millisecond span.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelSplit {
    /// Span start on the original timeline.
    pub start_ms: u64,
    /// Span end on the original timeline.
    pub end_ms: u64,
    /// `A` when the window is louder than the file, `B` when quieter.
    pub label: char,
    /// RMS of the window.
    pub window_rms: f64,
    /// RMS of the whole file (mono mix).
    pub file_rms: f64,
    /// How clean the split is (`0..=1`).
    pub confidence: f64,
}

/// Conservative L/R RMS ratio before a channel split is emitted.
pub const CHANNEL_RATIO: f64 = 4.0;
/// Balance `|L-R|/(L+R)` that still counts as dual-mono.
pub const DUAL_MONO_BALANCE: f64 = 0.5;
/// RMS below which a window is treated as silence.
pub const SILENCE_RMS: f64 = 1.0e-4;
/// Opt-in level split: window must be this many times louder or quieter.
pub const LEVEL_RATIO: f64 = 4.0;

/// Channel energy on `[start_ms, end_ms)` of the **original** PCM.
///
/// Returns `None` for mono, dual-mono, silence, or a ratio below [`CHANNEL_RATIO`].
pub fn channel_split(original: &DecodedAudio, start_ms: u64, end_ms: u64) -> Option<ChannelSplit> {
    if original.channels < 2 {
        return None;
    }
    let window = original.window(start_ms, end_ms);
    if window.frames() == 0 {
        return None;
    }
    let (left, right) = stereo_rms(&window)?;
    if left < SILENCE_RMS && right < SILENCE_RMS {
        return None;
    }
    let louder = left.max(right);
    let quieter = left.min(right).max(f64::EPSILON);
    let ratio = louder / quieter;
    let balance = (left - right).abs() / (left + right + f64::EPSILON);
    if ratio < CHANNEL_RATIO || balance < DUAL_MONO_BALANCE {
        return None;
    }
    let side = if left > right {
        ChannelSide::Left
    } else {
        ChannelSide::Right
    };
    // Confidence describes the split, not a speaker. Clamp to (0, 1].
    let confidence = (1.0 - CHANNEL_RATIO / ratio).clamp(0.05, 0.95);
    Some(ChannelSplit {
        start_ms,
        end_ms,
        side,
        left_rms: left,
        right_rms: right,
        confidence,
    })
}

/// Opt-in near/far split. Off unless the caller asks, because AGC 911 audio
/// flattens levels and a volume cluster is easy to over-read.
pub fn level_split(
    original: &DecodedAudio,
    start_ms: u64,
    end_ms: u64,
    file_rms: f64,
) -> Option<LevelSplit> {
    if file_rms < SILENCE_RMS {
        return None;
    }
    let window = original.window(start_ms, end_ms);
    let window_rms = mono_rms(&window)?;
    if window_rms < SILENCE_RMS {
        return None;
    }
    let (label, ratio) = if window_rms >= file_rms * LEVEL_RATIO {
        ('A', window_rms / file_rms)
    } else if file_rms >= window_rms * LEVEL_RATIO {
        ('B', file_rms / window_rms)
    } else {
        return None;
    };
    Some(LevelSplit {
        start_ms,
        end_ms,
        label,
        window_rms,
        file_rms,
        confidence: (1.0 - LEVEL_RATIO / ratio).clamp(0.05, 0.8),
    })
}

/// RMS of a mono mix of the whole file. Used as the baseline for [`level_split`].
pub fn file_rms(original: &DecodedAudio) -> f64 {
    mono_rms(original).unwrap_or(0.0)
}

fn stereo_rms(audio: &DecodedAudio) -> Option<(f64, f64)> {
    let channels = usize::from(audio.channels);
    let frames = audio.frames();
    if channels < 2 || frames == 0 {
        return None;
    }
    let buf = sequential_from_interleaved(audio)?;
    let left = rms_of(buf[0].iter().copied(), frames)?;
    let right = rms_of(buf[1].iter().copied(), frames)?;
    Some((left, right))
}

fn mono_rms(audio: &DecodedAudio) -> Option<f64> {
    if audio.samples.is_empty() {
        return None;
    }
    let channels = usize::from(audio.channels).max(1);
    let buf = sequential_from_interleaved(audio)?;
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for channel in 0..channels {
        for sample in &buf[channel] {
            let value = f64::from(*sample);
            sum += value * value;
            count += 1;
        }
    }
    if count == 0 {
        None
    } else {
        Some((sum / count as f64).sqrt())
    }
}

/// Copies interleaved PCM into an `audio` crate sequential buffer so channel
/// math uses the crate the adapter is built on, not ad-hoc indexing.
fn sequential_from_interleaved(audio: &DecodedAudio) -> Option<audio::buf::Sequential<f32>> {
    let channels = usize::from(audio.channels).max(1);
    let frames = audio.frames();
    if frames == 0 {
        return None;
    }
    let mut buf = audio::buf::Sequential::<f32>::with_topology(channels, frames);
    for channel in 0..channels {
        for frame in 0..frames {
            buf[channel][frame] = audio.samples[frame * channels + channel];
        }
    }
    Some(buf)
}

fn rms_of(samples: impl Iterator<Item = f32>, frames: usize) -> Option<f64> {
    if frames == 0 {
        return None;
    }
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for sample in samples {
        let value = f64::from(sample);
        sum += value * value;
        count += 1;
    }
    if count == 0 {
        None
    } else {
        Some((sum / count as f64).sqrt())
    }
}
