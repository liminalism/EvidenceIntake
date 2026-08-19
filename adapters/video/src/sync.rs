//! Align two recordings of one scene by cross-correlating their audio.
//!
//! Two officers' body cameras, or a dashcam and a body camera, record the same
//! street on two unsynchronised clocks. This module measures how far apart
//! their *soundtracks* sit -- the same siren chirp lands at `t` on one
//! recording and at `t + offset` on the other -- and proposes that measurement
//! as one `temporally_overlaps` edge between the two originals.
//!
//! Nothing about who or what is on either recording is inferred. The signal
//! path is a mono downmix, an anti-alias low-pass, a decimation to a few
//! kilohertz, and an FFT cross-correlation: no model, no identity, no score.
//! A weak or ambiguous peak emits nothing at all, which is a result and not a
//! failure -- the alternative is a guessed alignment a reviewer would have no
//! way to tell apart from a measured one.

use std::path::PathBuf;

use evidence_audio::DecodedAudio;
use evidence_intake::{
    CaseId, EdgeKind, ExtractionProvenance, NodeKind, NormalizedBatch, NormalizedEdge, ReviewState,
};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Extractor name stamped on every offset edge.
pub const EXTRACTOR_SYNC: &str = "audio_cross_correlation";

/// Version of the measurement itself. Bump it when the numbers would change.
pub const SYNC_VERSION: &str = "0.1.0";

/// RMS below which a working signal is treated as silence rather than sound.
const SILENT_RMS: f64 = 1e-6;

/// Half-width, in milliseconds, of the neighbourhood around the winning lag
/// that is excluded from both the prominence floor and the rival search.
const PEAK_NEIGHBOURHOOD_MS: u64 = 50;

/// A rival peak this close to the winner, further away than the neighbourhood,
/// makes the alignment ambiguous.
const AMBIGUITY_RATIO: f64 = 0.9;

/// Shortest overlap, in seconds, that may carry a measurement at all.
const MIN_OVERLAP_SECONDS: u64 = 2;

/// The overlap must also be at least this fraction of the shorter recording.
const MIN_OVERLAP_DIVISOR: usize = 5;

/// Prominence is reported no higher than this. A synthetic pair can drive the
/// background RMS to zero, and an unbounded ratio would serialise badly.
const MAX_PROMINENCE: f64 = 1e6;

/// Largest combined working length the correlation will allocate for.
///
/// The FFT is over the next power of two at or above `len_a + len_b`, in
/// complex `f64`; this ceiling keeps that under roughly a gigabyte. At the
/// default 4 kHz it is about half an hour per side.
const MAX_WORK_SAMPLES: usize = 8_000_000;

/// How hard to look for the offset between two recordings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncOptions {
    /// Mono rate the correlation runs at. Lower is faster and coarser.
    pub work_rate_hz: u32,
    /// Largest offset, in milliseconds, considered in either direction.
    pub max_lag_ms: u64,
    /// Least peak height, in units of the correlation's RMS over the searched
    /// lags, that counts as a measurement rather than as background.
    pub min_prominence: f64,
    /// Least normalized correlation coefficient that counts as a match.
    pub min_peak: f64,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            work_rate_hz: 4_000,
            max_lag_ms: 120_000,
            min_prominence: 8.0,
            min_peak: 0.2,
        }
    }
}

/// One measured offset between two recordings, with the evidence for it.
///
/// Every field is reported so a reviewer can judge the measurement instead of
/// trusting it: the coefficient says how alike the two soundtracks are at the
/// winning lag, the prominence says how far that lag stands above every other
/// lag tried, and the durations say how much recording was behind both.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SyncMeasurement {
    /// Offset in milliseconds. Positive means B lags A: the sound heard at
    /// `t` on A is heard at `t + offset_ms` on B.
    pub offset_ms: i64,
    /// Normalized cross-correlation coefficient in `[-1, 1]` at the winning
    /// lag.
    pub peak: f64,
    /// Peak height in units of the correlation's RMS over the searched lags,
    /// excluding a neighbourhood of the peak itself.
    pub prominence: f64,
    /// Mono rate the correlation ran at.
    pub work_rate_hz: u32,
    /// Largest absolute lag actually evaluated, in milliseconds.
    pub searched_lag_ms: u64,
    /// Duration of the first recording, in milliseconds.
    pub a_duration_ms: u64,
    /// Duration of the second recording, in milliseconds.
    pub b_duration_ms: u64,
}

/// One side of a pair: which original, under which identifier and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSide {
    /// Identifier the case already holds for this original.
    pub source_id: String,
    /// Display name, used verbatim in the rationale.
    pub logical_name: String,
    /// Path to the untouched original.
    pub path: PathBuf,
}

/// Two originals claimed to cover the same event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPair {
    /// Existing case that already holds both sources.
    pub case_id: CaseId,
    /// The recording the offset is measured *from*.
    pub a: SyncSide,
    /// The recording the offset is measured *to*.
    pub b: SyncSide,
}

/// Measures how far B sits from A, or reports that nothing can be said.
///
/// Both recordings are mixed to mono, low-passed, decimated to
/// `work_rate_hz`, stripped of DC, and scaled to unit RMS. The correlation is
/// computed with a zero-padded FFT, so it is linear rather than circular, and
/// each lag is normalized by the energies of exactly the samples that overlap
/// at that lag -- which makes the reported `peak` a true correlation
/// coefficient rather than a lag-dependent one.
///
/// Returns `Ok(None)`, never a guess, when either side is silent, when the
/// overlap is too short to mean anything, when the peak is below `min_peak`
/// or its prominence below `min_prominence`, or when some other lag more than
/// 50 ms away comes within 90% of the winner's height. That last rule is what
/// keeps a periodic sound -- an idling engine, a repeating siren cycle --
/// from being reported as an alignment when the recordings could equally sit
/// one period apart.
pub fn measure_offset(
    a: &DecodedAudio,
    b: &DecodedAudio,
    options: &SyncOptions,
) -> Result<Option<SyncMeasurement>> {
    let work_rate = options.work_rate_hz;
    if work_rate == 0 {
        return Err(Error::Mapping(
            "the correlation work rate must be positive".to_owned(),
        ));
    }
    if !options.min_peak.is_finite() || !options.min_prominence.is_finite() {
        return Err(Error::Mapping(
            "the peak and prominence floors must be finite".to_owned(),
        ));
    }

    let (Some(signal_a), Some(signal_b)) = (work_signal(a, work_rate)?, work_signal(b, work_rate)?)
    else {
        return Ok(None);
    };
    let (len_a, len_b) = (signal_a.len(), signal_b.len());
    if len_a.saturating_add(len_b) > MAX_WORK_SAMPLES {
        return Err(Error::Mapping(format!(
            "{len_a} + {len_b} working samples exceeds the {MAX_WORK_SAMPLES}-sample correlation \
             ceiling; lower the work rate or correlate a shorter span"
        )));
    }

    let correlation = raw_correlation(&signal_a, &signal_b);
    let evaluated = normalized_lags(&signal_a, &signal_b, &correlation, options);
    let Some(&(best_lag, peak)) = evaluated
        .iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
    else {
        return Ok(None);
    };
    if peak < options.min_peak {
        return Ok(None);
    }

    let neighbourhood = (u64::from(work_rate) * PEAK_NEIGHBOURHOOD_MS / 1_000) as i64;
    let mut sum_squares = 0.0_f64;
    let mut counted = 0_usize;
    let mut rival = 0.0_f64;
    for &(lag, value) in &evaluated {
        if (lag - best_lag).abs() <= neighbourhood {
            continue;
        }
        sum_squares += value * value;
        counted += 1;
        rival = rival.max(value.abs());
    }
    if counted == 0 {
        // Nothing to compare the winner against, so nothing can be claimed.
        return Ok(None);
    }
    let background = (sum_squares / counted as f64).sqrt().max(1e-12);
    let prominence = (peak / background).min(MAX_PROMINENCE);
    if prominence < options.min_prominence {
        return Ok(None);
    }
    if rival >= AMBIGUITY_RATIO * peak {
        return Ok(None);
    }

    let searched = evaluated
        .iter()
        .map(|(lag, _)| lag.unsigned_abs())
        .max()
        .unwrap_or_default();
    Ok(Some(SyncMeasurement {
        offset_ms: lag_to_ms(best_lag, work_rate),
        peak,
        prominence,
        work_rate_hz: work_rate,
        searched_lag_ms: searched * 1_000 / u64::from(work_rate),
        a_duration_ms: a.duration_ms(),
        b_duration_ms: b.duration_ms(),
    }))
}

/// Turns one measurement into a batch carrying a single suggested edge.
///
/// The batch has no sources: both originals are already in the case, and the
/// only new fact is how they sit relative to each other. The measurement
/// itself lives in the rationale, in a stable leading form, so a later reader
/// -- person or parser -- can recover the offset without re-running anything.
pub fn measurement_to_batch(
    pair: &SyncPair,
    measurement: &SyncMeasurement,
) -> Result<NormalizedBatch> {
    let from = pair.a.source_id.trim();
    let to = pair.b.source_id.trim();
    if from.is_empty() || to.is_empty() {
        return Err(Error::Mapping(
            "both sides of a sync pair must name a source".to_owned(),
        ));
    }
    if from == to {
        return Err(Error::Mapping(format!(
            "`{from}` cannot be synchronised against itself"
        )));
    }
    if !measurement.peak.is_finite() || !measurement.prominence.is_finite() {
        return Err(Error::Mapping(
            "a sync measurement must carry finite statistics".to_owned(),
        ));
    }

    Ok(NormalizedBatch {
        case_id: pair.case_id.clone(),
        sources: Vec::new(),
        edges: vec![NormalizedEdge {
            id: edge_id(from, to),
            from_kind: NodeKind::Source,
            from_id: from.to_owned(),
            relation: EdgeKind::TemporallyOverlaps,
            to_kind: NodeKind::Source,
            to_id: to.to_owned(),
            rationale: rationale(pair, measurement),
            extraction: ExtractionProvenance {
                extractor: EXTRACTOR_SYNC.to_owned(),
                version: SYNC_VERSION.to_owned(),
                machine_generated: true,
                confidence: None,
                review_state: ReviewState::Suggested,
            },
        }],
    })
}

/// Opens both originals, measures the offset, and maps it to a batch.
///
/// Decoding is the audio adapter's: a wav is read directly, a video container
/// has its soundtrack pulled by ffmpeg. Either way the offset addresses the
/// original timelines, because nothing on the path time-stretches.
pub fn sync_pair(pair: &SyncPair, options: &SyncOptions) -> Result<Option<NormalizedBatch>> {
    if pair.a.source_id.trim() == pair.b.source_id.trim() {
        return Err(Error::Mapping(format!(
            "`{}` cannot be synchronised against itself",
            pair.a.source_id.trim()
        )));
    }
    let a = evidence_audio::open_media(&pair.a.path)?;
    let b = evidence_audio::open_media(&pair.b.path)?;
    let Some(measurement) = measure_offset(&a.audio, &b.audio, options)? else {
        return Ok(None);
    };
    measurement_to_batch(pair, &measurement).map(Some)
}

/// The stable leading form a later parser reads the offset back out of.
fn rationale(pair: &SyncPair, measurement: &SyncMeasurement) -> String {
    let verb = if measurement.offset_ms < 0 {
        "leads"
    } else {
        "lags"
    };
    format!(
        "audio cross-correlation: `{b}` {verb} `{a}` by {magnitude} ms (peak {peak:.2}, \
         prominence {prominence:.1}, {rate} Hz mono, lags searched \u{b1}{searched} ms). \
         Suggested; verify by playing both originals at the aligned instant.",
        b = pair.b.logical_name,
        a = pair.a.logical_name,
        magnitude = measurement.offset_ms.unsigned_abs(),
        peak = measurement.peak,
        prominence = measurement.prominence,
        rate = measurement.work_rate_hz,
        searched = measurement.searched_lag_ms,
    )
}

/// Deterministic, collision-safe identifier for one ordered pair.
///
/// The readable half is a slug of both identifiers; the trailing digest is
/// what makes two pairs that slug alike still distinct.
fn edge_id(from: &str, to: &str) -> String {
    let digest = Sha256::digest(format!("{from}\u{1f}{to}").as_bytes());
    let hash = hex::encode(digest);
    format!(
        "sync-{}-{}-{}",
        slug(from),
        slug(to),
        hash.get(..12).unwrap_or(hash.as_str())
    )
}

/// Lowercase, hyphen-separated, at most 24 characters.
fn slug(value: &str) -> String {
    value
        .chars()
        .take(24)
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Converts a lag in working samples to milliseconds.
fn lag_to_ms(lag: i64, work_rate_hz: u32) -> i64 {
    (lag as f64 * 1_000.0 / f64::from(work_rate_hz)).round() as i64
}

/// Mono, band-limited, decimated, DC-free, unit-RMS samples at the work rate.
///
/// `None` means the recording carries no usable signal -- it is empty, or it
/// is silence -- which is a reason to say nothing rather than an error.
fn work_signal(audio: &DecodedAudio, work_rate_hz: u32) -> Result<Option<Vec<f64>>> {
    if audio.sample_rate == 0 {
        return Err(Error::Mapping(
            "a recording with no sample rate cannot be correlated".to_owned(),
        ));
    }
    let mono = audio.mix_mono();
    if mono.samples.is_empty() {
        return Ok(None);
    }

    let mut samples: Vec<f64> = mono
        .samples
        .iter()
        .map(|sample| f64::from(*sample))
        .collect();
    if mono.sample_rate != work_rate_hz {
        if work_rate_hz < mono.sample_rate {
            // Both sides get the same one-pass filter, so whatever phase it
            // introduces is common-mode and cancels in the correlation.
            low_pass_in_place(
                &mut samples,
                mono.sample_rate,
                0.45 * f64::from(work_rate_hz),
            );
        }
        samples = resample_linear(&samples, mono.sample_rate, work_rate_hz);
    }
    if samples.is_empty() {
        return Ok(None);
    }

    let count = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / count;
    for sample in &mut samples {
        *sample -= mean;
    }
    let rms = (samples.iter().map(|sample| sample * sample).sum::<f64>() / count).sqrt();
    if !rms.is_finite() || rms < SILENT_RMS {
        return Ok(None);
    }
    for sample in &mut samples {
        *sample /= rms;
    }
    Ok(Some(samples))
}

/// One RBJ low-pass biquad at `cutoff_hz`, applied forward in place.
fn low_pass_in_place(samples: &mut [f64], sample_rate: u32, cutoff_hz: f64) {
    let rate = f64::from(sample_rate);
    if cutoff_hz <= 0.0 || cutoff_hz >= rate / 2.0 {
        return;
    }
    let w0 = 2.0 * std::f64::consts::PI * cutoff_hz / rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / std::f64::consts::SQRT_2;
    let b0 = (1.0 - cos) / 2.0;
    let b1 = 1.0 - cos;
    let b2 = (1.0 - cos) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos;
    let a2 = 1.0 - alpha;
    let (c0, c1, c2) = (b0 / a0, b1 / a0, b2 / a0);
    let (d1, d2) = (a1 / a0, a2 / a0);

    let mut z1 = 0.0_f64;
    let mut z2 = 0.0_f64;
    for sample in samples.iter_mut() {
        let x = *sample;
        let y = c0 * x + z1;
        z1 = c1 * x - d1 * y + z2;
        z2 = c2 * x - d2 * y;
        *sample = y;
    }
}

/// Linear-interpolating resample. Duration is preserved, and both sides are
/// read off the same grid from sample zero, so no relative shift is added.
fn resample_linear(samples: &[f64], from_rate: u32, to_rate: u32) -> Vec<f64> {
    if samples.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    let step = f64::from(from_rate) / f64::from(to_rate);
    let out_len = (samples.len() as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let position = index as f64 * step;
        let base = position.floor() as usize;
        let fraction = position - position.floor();
        let left = samples.get(base).copied().unwrap_or_default();
        let right = samples.get(base + 1).copied().unwrap_or(left);
        out.push(left + (right - left) * fraction);
    }
    out
}

/// Unnormalized linear cross-correlation, `sum_t a[t] * b[t + lag]`.
///
/// Index `lag` holds a non-negative lag and index `n - lag` a negative one;
/// the zero padding to a power of two at or above `len_a + len_b` is what
/// keeps the circular FFT correlation from wrapping one into the other.
fn raw_correlation(a: &[f64], b: &[f64]) -> Vec<f64> {
    let n = (a.len() + b.len()).next_power_of_two();
    let mut left = vec![Complex::new(0.0_f64, 0.0_f64); n];
    let mut right = vec![Complex::new(0.0_f64, 0.0_f64); n];
    for (slot, sample) in left.iter_mut().zip(a) {
        slot.re = *sample;
    }
    for (slot, sample) in right.iter_mut().zip(b) {
        slot.re = *sample;
    }

    let mut planner = FftPlanner::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    forward.process(&mut left);
    forward.process(&mut right);
    for (slot, other) in left.iter_mut().zip(&right) {
        *slot = slot.conj() * other;
    }
    inverse.process(&mut left);

    let scale = 1.0 / n as f64;
    left.into_iter().map(|value| value.re * scale).collect()
}

/// Every lag worth looking at, as a proper correlation coefficient.
///
/// A lag is skipped when the two recordings barely overlap there: a handful
/// of aligned samples can correlate perfectly by accident, and the resulting
/// coefficient would say nothing about the recordings.
fn normalized_lags(
    a: &[f64],
    b: &[f64],
    correlation: &[f64],
    options: &SyncOptions,
) -> Vec<(i64, f64)> {
    let (len_a, len_b) = (a.len(), b.len());
    let n = correlation.len();
    let energy_a = prefix_energy(a);
    let energy_b = prefix_energy(b);

    let max_lag = i64::try_from(
        options
            .max_lag_ms
            .saturating_mul(u64::from(options.work_rate_hz))
            / 1_000,
    )
    .unwrap_or(i64::MAX);
    let low = -max_lag.min(len_b as i64 - 1);
    let high = max_lag.min(len_a as i64 - 1);
    if low > high {
        return Vec::new();
    }
    let min_overlap = usize::max(
        (u64::from(options.work_rate_hz) * MIN_OVERLAP_SECONDS) as usize,
        len_a.min(len_b) / MIN_OVERLAP_DIVISOR,
    );

    let mut lags = Vec::new();
    for lag in low..=high {
        let start = if lag < 0 { (-lag) as usize } else { 0 };
        let end = (len_a as i64).min(len_b as i64 - lag);
        if end <= start as i64 {
            continue;
        }
        let end = end as usize;
        if end - start < min_overlap {
            continue;
        }
        let left = energy_a[end] - energy_a[start];
        let right = energy_b[(end as i64 + lag) as usize] - energy_b[(start as i64 + lag) as usize];
        if left <= 0.0 || right <= 0.0 {
            continue;
        }
        let index = if lag >= 0 {
            lag as usize
        } else {
            n - (-lag) as usize
        };
        let Some(raw) = correlation.get(index) else {
            continue;
        };
        lags.push((lag, (raw / (left * right).sqrt()).clamp(-1.0, 1.0)));
    }
    lags
}

/// `out[i]` is the energy of the first `i` samples.
fn prefix_energy(samples: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(samples.len() + 1);
    let mut total = 0.0_f64;
    out.push(total);
    for sample in samples {
        total += sample * sample;
        out.push(total);
    }
    out
}
