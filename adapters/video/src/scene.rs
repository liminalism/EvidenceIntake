//! Scene cuts and keyframe stills. No neural net.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Default ffmpeg `scene` score that counts as a cut.
pub const DEFAULT_THRESHOLD: f64 = 0.30;
/// Tail shorter than claimed duration by this much becomes a `recording_gap`.
pub const DEFAULT_GAP_MS: u64 = 2_000;
/// Longest intended interval between visual-index samples.
pub const DEFAULT_SAMPLE_GAP_MS: u64 = 5_000;
/// Scene-triggered samples this close to the previous sample are suppressed.
pub const DEFAULT_SAMPLE_DEDUP_MS: u64 = 250;

/// One scene on the original timeline, optionally with a derived still.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    /// One-based scene number, matching fixture locators (`scene 1`, …).
    pub index: u32,
    /// Inclusive start on the original timeline.
    pub start_ms: u64,
    /// Exclusive end on the original timeline.
    pub end_ms: u64,
    /// ffmpeg scene score of the cut that *opened* this scene, when known.
    pub cut_score_millis: Option<u32>,
    /// Working-copy still taken at `start_ms`.
    pub keyframe: Option<Keyframe>,
}

/// A jpeg still extracted from one frame. Not the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyframe {
    /// Path of the working-copy jpeg.
    pub path: PathBuf,
    /// SHA-256 of those jpeg bytes.
    pub sha256: String,
    /// Length of those jpeg bytes.
    pub byte_length: u64,
    /// Pixel width of the still, when the jpeg header can be read.
    pub width: Option<u32>,
    /// Pixel height of the still, when the jpeg header can be read.
    pub height: Option<u32>,
}

/// Scene cuts plus an optional tail dropout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneAnalysis {
    /// Container-claimed duration.
    pub duration_ms: u64,
    /// Last video packet timestamp, when ffprobe reported one.
    pub last_video_pts_ms: Option<u64>,
    /// Ordered scenes covering `[0, last_decodable]`.
    pub scenes: Vec<Scene>,
    /// Missing tail `(start_ms, end_ms)` when the stream ends early.
    pub dropout: Option<(u64, u64)>,
}

/// Detect scene cuts with ffmpeg's `scene` filter and extract one still per scene.
pub fn detect_scenes(
    path: &Path,
    threshold: f64,
    gap_ms: u64,
    stills_dir: Option<&Path>,
) -> Result<SceneAnalysis> {
    detect_scenes_with_sampling(
        path,
        threshold,
        gap_ms,
        stills_dir,
        DEFAULT_SAMPLE_GAP_MS,
        DEFAULT_SAMPLE_DEDUP_MS,
    )
}

/// Detect scene changes while also bounding the interval between samples.
///
/// One ffmpeg decode selects the first frame, periodic coverage frames, and
/// scene-change frames. A scene-triggered frame inside `sample_dedup_ms` of
/// the previous retained frame is suppressed. Set `max_sample_gap_ms` to zero
/// for scene-change-only sampling.
pub fn detect_scenes_with_sampling(
    path: &Path,
    threshold: f64,
    gap_ms: u64,
    stills_dir: Option<&Path>,
    max_sample_gap_ms: u64,
    sample_dedup_ms: u64,
) -> Result<SceneAnalysis> {
    if !path.is_file() {
        return Err(Error::Open {
            path: path.to_path_buf(),
            message: "file does not exist".to_owned(),
        });
    }
    if !ffprobe_available() {
        return Err(Error::Probe(
            "ffprobe is not on PATH. Install ffmpeg (which includes ffprobe) to cut scenes."
                .to_owned(),
        ));
    }

    let duration_ms = probe_duration_ms(path)?;
    if duration_ms == 0 {
        return Err(Error::Open {
            path: path.to_path_buf(),
            message: "container reports zero duration".to_owned(),
        });
    }
    let last_video_pts_ms = probe_last_video_pts_ms(path)?;
    let decodable_end = last_video_pts_ms.unwrap_or(duration_ms).min(duration_ms);
    let frame_interval_ms = probe_frame_interval_ms(path).unwrap_or(0);

    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(Error::Probe(
            "scene threshold must be a finite value in 0..=1".to_owned(),
        ));
    }

    let keep_dir = match stills_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            Some(dir.to_path_buf())
        }
        None => None,
    };
    let scratch = if keep_dir.is_none() {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let stills_root = keep_dir
        .as_deref()
        .or_else(|| scratch.as_ref().map(tempfile::TempDir::path));
    let Some(stills_root) = stills_root else {
        return Err(Error::Probe(
            "could not create a stills directory".to_owned(),
        ));
    };

    let samples = extract_samples(
        path,
        threshold,
        max_sample_gap_ms,
        sample_dedup_ms,
        frame_interval_ms,
        stills_root,
    )?;
    let mut scenes = Vec::with_capacity(samples.len());
    for (index, (start_ms, keyframe)) in samples.iter().enumerate() {
        let end_ms = samples
            .get(index + 1)
            .map_or(decodable_end, |(next_ms, _)| *next_ms);
        if end_ms <= *start_ms {
            continue;
        }
        scenes.push(Scene {
            index: u32::try_from(index + 1).unwrap_or(u32::MAX),
            start_ms: *start_ms,
            end_ms,
            cut_score_millis: None,
            keyframe: Some(keyframe.clone()),
        });
    }

    // Leak scratch so stills stay readable for the rest of the process when
    // the operator did not pass --stills-dir. The OS tmp cleaner takes them.
    if let Some(dir) = scratch {
        std::mem::forget(dir);
    }

    let dropout = dropout_span(decodable_end, duration_ms, gap_ms);
    Ok(SceneAnalysis {
        duration_ms,
        last_video_pts_ms,
        scenes,
        dropout,
    })
}

/// Whether the tail of the container is missing enough to count as a gap.
pub fn dropout_span(
    decodable_end_ms: u64,
    claimed_duration_ms: u64,
    gap_ms: u64,
) -> Option<(u64, u64)> {
    if gap_ms == 0 {
        return None;
    }
    if claimed_duration_ms.saturating_sub(decodable_end_ms) >= gap_ms {
        Some((decodable_end_ms, claimed_duration_ms))
    } else {
        None
    }
}

/// Whether `ffprobe` is on PATH.
pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn probe_duration_ms(path: &Path) -> Result<u64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|error| Error::Probe(format!("could not run ffprobe: {error}")))?;
    if !output.status.success() {
        return Err(Error::Probe(format!(
            "ffprobe duration failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    parse_seconds_ms(String::from_utf8_lossy(&output.stdout).trim()).ok_or_else(|| {
        Error::Probe(format!(
            "ffprobe duration was not a number: {}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })
}

fn probe_last_video_pts_ms(path: &Path) -> Result<Option<u64>> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|error| Error::Probe(format!("could not run ffprobe stream duration: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(parse_seconds_ms(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn probe_frame_interval_ms(path: &Path) -> Result<u64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .map_err(|error| Error::Probe(format!("could not probe video frame rate: {error}")))?;
    if !output.status.success() {
        return Ok(0);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some((numerator, denominator)) = text.trim().split_once('/') else {
        return Ok(0);
    };
    let numerator: f64 = numerator.parse().unwrap_or(0.0);
    let denominator: f64 = denominator.parse().unwrap_or(0.0);
    if numerator <= 0.0 || denominator <= 0.0 {
        return Ok(0);
    }
    Ok((1_000.0 * denominator / numerator).ceil() as u64)
}

fn extract_samples(
    path: &Path,
    threshold: f64,
    max_sample_gap_ms: u64,
    sample_dedup_ms: u64,
    frame_interval_ms: u64,
    stills_root: &Path,
) -> Result<Vec<(u64, Keyframe)>> {
    let report_dir = tempfile::tempdir()?;
    let report = report_dir.path().join("selected.txt");
    // ffmpeg runs in `report_dir` so its metadata sidecar has a short, safely
    // quoted name. Resolve caller paths before changing the child's working
    // directory; otherwise valid relative input/output paths point into the
    // temporary report directory.
    let invocation_dir = std::env::current_dir()?;
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        invocation_dir.join(path)
    };
    let stills_root = if stills_root.is_absolute() {
        stills_root.to_path_buf()
    } else {
        invocation_dir.join(stills_root)
    };
    let pattern = stills_root.join("scene-%04d.jpg");
    // `select` can only retain an actual frame. Start looking one nominal
    // frame early so a constant-frame-rate stream's next frame stays inside
    // the caller's requested maximum rather than exceeding it by one tick.
    let trigger_gap_ms = max_sample_gap_ms.saturating_sub(frame_interval_ms).max(1);
    let max_gap_s = trigger_gap_ms as f64 / 1_000.0;
    let dedup_s = sample_dedup_ms as f64 / 1_000.0;
    let periodic = if max_sample_gap_ms == 0 {
        String::new()
    } else {
        format!("+gte(t-prev_selected_t,{max_gap_s:.6})")
    };
    let filter = format!(
        "setpts=PTS-STARTPTS,select='isnan(prev_selected_t){periodic}+gte(t-prev_selected_t,{dedup_s:.6})*gte(scene,{threshold:.6})',metadata=print:file=selected.txt"
    );
    let status = Command::new("ffmpeg")
        .current_dir(report_dir.path())
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&input)
        .args(["-filter:v", &filter, "-an", "-fps_mode", "vfr", "-q:v", "3"])
        .arg(&pattern)
        .status()
        .map_err(|error| {
            Error::Probe(format!("could not run ffmpeg sample extraction: {error}"))
        })?;
    if !status.success() {
        return Err(Error::Probe(format!(
            "ffmpeg sample extraction exited with {status}"
        )));
    }
    let text = if report.is_file() {
        std::fs::read_to_string(&report)?
    } else {
        String::new()
    };
    let times = parse_scene_report(&text);
    if times.is_empty() {
        return Err(Error::Probe(
            "ffmpeg selected no frames for the visual index".to_owned(),
        ));
    }
    let mut samples = Vec::with_capacity(times.len());
    for (offset, time_ms) in times.into_iter().enumerate() {
        let number = offset + 1;
        let still = stills_root.join(format!("scene-{number:04}.jpg"));
        let bytes = std::fs::read(&still).map_err(|error| {
            Error::Probe(format!(
                "ffmpeg reported sample {number} at {time_ms}ms but {} could not be read: {error}",
                still.display()
            ))
        })?;
        if bytes.is_empty() {
            return Err(Error::Probe(format!(
                "ffmpeg wrote an empty still at {time_ms}ms"
            )));
        }
        let (width, height) = jpeg_dimensions(&bytes).unzip();
        samples.push((
            time_ms,
            Keyframe {
                path: still,
                sha256: hex::encode(Sha256::digest(&bytes)),
                byte_length: bytes.len() as u64,
                width,
                height,
            },
        ));
    }
    Ok(samples)
}

/// Parses `metadata=print` output for `pts_time` lines.
pub fn parse_scene_report(text: &str) -> Vec<u64> {
    let mut cuts = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.split("pts_time:").nth(1) else {
            continue;
        };
        let token = rest.split_whitespace().next().unwrap_or("");
        if let Some(ms) = parse_seconds_ms(token)
            && cuts.last() != Some(&ms)
        {
            cuts.push(ms);
        }
    }
    cuts
}

/// SOF0/SOF2 width and height from a jpeg. Used to scale normalized boxes
/// into original-frame pixels.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2_usize;
    while index + 8 < bytes.len() {
        if bytes[index] != 0xFF {
            return None;
        }
        let marker = bytes[index + 1];
        let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC2) {
            let height = u32::from(u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]));
            if width > 0 && height > 0 {
                return Some((width, height));
            }
            return None;
        }
        index = index.saturating_add(2).saturating_add(length);
    }
    None
}

fn parse_seconds_ms(text: &str) -> Option<u64> {
    let value: f64 = text.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * 1_000.0).round() as u64)
}
