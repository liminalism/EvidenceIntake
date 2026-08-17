//! Working-copy cleanup. Locators never come from this audio.

use nnnoiseless::DenoiseState;
use resampler::{ResamplerFft, SampleRate};

use crate::decode::DecodedAudio;
use crate::{Error, Result};

/// Target rate for `nnnoiseless` (and a fine rate to hand WhisperX).
pub const WORKING_RATE: u32 = 48_000;
/// Telephone / radio band used when `--phone-band` is set.
pub const PHONE_LOW_HZ: f32 = 300.0;
/// Upper edge of the telephone band.
pub const PHONE_HIGH_HZ: f32 = 3_400.0;

/// Resample to 48 kHz, optionally band-limit, then denoise. Duration is
/// preserved so WhisperX seconds still address the original timeline.
pub fn prepare_working_copy(original: &DecodedAudio, phone_band: bool) -> Result<DecodedAudio> {
    let mut working = original.mix_mono();
    working = resample_to(&working, WORKING_RATE)?;
    if phone_band {
        bandpass_inplace(&mut working, PHONE_LOW_HZ, PHONE_HIGH_HZ);
    }
    denoise_inplace(&mut working);
    Ok(working)
}

/// Resample without changing duration. Used for the 48 kHz working copy and
/// for the 16 kHz mono buffer Whisper requires.
pub fn resample_to(audio: &DecodedAudio, target_rate: u32) -> Result<DecodedAudio> {
    if audio.sample_rate == target_rate {
        return Ok(audio.clone());
    }
    let source_rate = audio.sample_rate;
    let prepared = if source_rate == 8_000 {
        upsample_2x(audio)
    } else {
        audio.clone()
    };
    let from = SampleRate::try_from(prepared.sample_rate)
        .map_err(|()| Error::UnsupportedRate { rate: source_rate })?;
    let to = SampleRate::try_from(target_rate)
        .map_err(|()| Error::UnsupportedRate { rate: target_rate })?;
    if from == to {
        return Ok(prepared);
    }

    let channels = usize::from(prepared.channels).max(1);
    let mut resampler = ResamplerFft::new(channels, from, to);
    let input_chunk = resampler.chunk_size_input();
    let output_chunk = resampler.chunk_size_output();
    if input_chunk == 0 {
        return Err(Error::UnsupportedRate { rate: source_rate });
    }

    let mut input = prepared.samples.clone();
    let remainder = input.len() % input_chunk;
    if remainder != 0 {
        input.resize(input.len() + (input_chunk - remainder), 0.0);
    }

    let mut output = Vec::with_capacity((input.len() / input_chunk) * output_chunk);
    let mut out_buf = vec![0.0_f32; output_chunk];
    for chunk in input.chunks_exact(input_chunk) {
        resampler
            .resample(chunk, &mut out_buf)
            .map_err(|error| Error::Transcript(format!("resampler: {error}")))?;
        output.extend_from_slice(&out_buf);
    }

    // Trim the padded tail so duration stays within one resampler chunk of
    // the original. WhisperX timestamps are still original-timeline seconds
    // because we never time-stretch.
    let expected = ((prepared.frames() as u64) * u64::from(target_rate)
        / u64::from(prepared.sample_rate)) as usize;
    output.truncate(expected.max(1));

    Ok(DecodedAudio {
        sha256: prepared.sha256,
        byte_length: prepared.byte_length,
        media_type: prepared.media_type,
        sample_rate: target_rate,
        channels: prepared.channels,
        samples: output,
    })
}

/// Linear 2× upsample so 8 kHz telephone audio can enter [`ResamplerFft`] at 16 kHz.
fn upsample_2x(audio: &DecodedAudio) -> DecodedAudio {
    let channels = usize::from(audio.channels).max(1);
    let frames = audio.frames();
    let mut samples = Vec::with_capacity(audio.samples.len() * 2);
    for frame in 0..frames {
        let start = frame * channels;
        let next = ((frame + 1) * channels).min(audio.samples.len());
        let current = &audio.samples[start..start + channels];
        samples.extend_from_slice(current);
        if next + channels <= audio.samples.len() {
            for ch in 0..channels {
                samples.push(f32::midpoint(
                    audio.samples[start + ch],
                    audio.samples[next + ch],
                ));
            }
        } else {
            samples.extend_from_slice(current);
        }
    }
    DecodedAudio {
        sha256: audio.sha256.clone(),
        byte_length: audio.byte_length,
        media_type: audio.media_type.clone(),
        sample_rate: audio.sample_rate.saturating_mul(2),
        channels: audio.channels,
        samples,
    }
}

/// RBJ band-pass, applied in place, one biquad, 48 kHz assumed.
fn bandpass_inplace(audio: &mut DecodedAudio, low_hz: f32, high_hz: f32) {
    let rate = audio.sample_rate as f32;
    if rate <= 0.0 {
        return;
    }
    let center = (low_hz * high_hz).sqrt();
    let bandwidth = high_hz - low_hz;
    if center <= 0.0 || bandwidth <= 0.0 {
        return;
    }
    let q = center / bandwidth;
    let w0 = 2.0 * std::f32::consts::PI * center / rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let b0 = alpha;
    let b1 = 0.0;
    let b2 = -alpha;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos;
    let a2 = 1.0 - alpha;
    let c0 = b0 / a0;
    let c1 = b1 / a0;
    let c2 = b2 / a0;
    let d1 = a1 / a0;
    let d2 = a2 / a0;

    let channels = usize::from(audio.channels).max(1);
    let mut z1 = vec![0.0_f32; channels];
    let mut z2 = vec![0.0_f32; channels];
    for frame in audio.samples.chunks_mut(channels) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let x = *sample;
            let y = c0 * x + z1[channel];
            z1[channel] = c1 * x - d1 * y + z2[channel];
            z2[channel] = c2 * x - d2 * y;
            *sample = y;
        }
    }
}

fn denoise_inplace(audio: &mut DecodedAudio) {
    // nnnoiseless is mono and wants 16-bit-range f32 at 48 kHz. Prepend one
    // silent frame so the discarded fade-in does not shift the timeline.
    let frame = DenoiseState::FRAME_SIZE;
    let mut input: Vec<f32> = std::iter::repeat_n(0.0, frame)
        .chain(
            audio
                .samples
                .iter()
                .map(|sample| sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)),
        )
        .collect();
    let pad = (frame - (input.len() % frame)) % frame;
    input.resize(input.len() + pad, 0.0);

    let mut denoise = DenoiseState::new();
    let mut out_buf = [0.0_f32; DenoiseState::FRAME_SIZE];
    let mut output = Vec::with_capacity(input.len());
    let mut first = true;
    for chunk in input.chunks_exact(frame) {
        denoise.process_frame(&mut out_buf, chunk);
        if first {
            first = false;
            continue;
        }
        output.extend(
            out_buf
                .iter()
                .map(|sample| (sample / f32::from(i16::MAX)).clamp(-1.0, 1.0)),
        );
    }
    output.truncate(audio.samples.len());
    if !output.is_empty() {
        audio.samples = output;
    }
}
