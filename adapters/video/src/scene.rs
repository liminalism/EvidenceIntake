//! Scene cuts and keyframe stills. No neural net.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Default ffmpeg `scene` score that counts as a cut.
pub const DEFAULT_THRESHOLD: f64 = 0.30;
/// Tail shorter than claimed duration by this much becomes a `recording_gap`.
pub const DEFAULT_GAP_MS: u64 = 2_000;

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

    let cuts = probe_cut_times(path, threshold)?;
    let mut boundaries = vec![0_u64];
    for cut in cuts {
        if cut > 0 && cut < decodable_end && boundaries.last() != Some(&cut) {
            boundaries.push(cut);
        }
    }
    if *boundaries.last().unwrap_or(&0) < decodable_end {
        boundaries.push(decodable_end);
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

    let mut scenes = Vec::new();
    for (index, window) in boundaries.windows(2).enumerate() {
        let start_ms = window[0];
        let end_ms = window[1];
        if end_ms <= start_ms {
            continue;
        }
        let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let keyframe = extract_keyframe(path, start_ms, stills_root, number).ok();
        scenes.push(Scene {
            index: number,
            start_ms,
            end_ms,
            cut_score_millis: None,
            keyframe,
        });
    }
    if scenes.is_empty() {
        let keyframe = extract_keyframe(path, 0, stills_root, 1).ok();
        scenes.push(Scene {
            index: 1,
            start_ms: 0,
            end_ms: decodable_end.max(1),
            cut_score_millis: None,
            keyframe,
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

fn probe_cut_times(path: &Path, threshold: f64) -> Result<Vec<u64>> {
    let dir = tempfile::tempdir()?;
    let report = dir.path().join("cuts.txt");
    let status = Command::new("ffmpeg")
        .current_dir(dir.path())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-filter:v",
            &format!("select='gte(scene,{threshold})',metadata=print:file=cuts.txt"),
            "-an",
            "-f",
            "null",
            "-",
        ])
        .status()
        .map_err(|error| Error::Probe(format!("could not run ffmpeg scene filter: {error}")))?;
    if !status.success() {
        return Err(Error::Probe(format!(
            "ffmpeg scene filter exited with {status}"
        )));
    }
    let text = if report.is_file() {
        std::fs::read_to_string(&report)?
    } else {
        String::new()
    };
    Ok(parse_scene_report(&text))
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

fn extract_keyframe(path: &Path, at_ms: u64, dir: &Path, index: u32) -> Result<Keyframe> {
    let still = dir.join(format!("scene-{index:04}.jpg"));
    let seconds = format!("{:.3}", at_ms as f64 / 1_000.0);
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-ss", &seconds, "-frames:v", "1", "-q:v", "3"])
        .arg(&still)
        .status()
        .map_err(|error| Error::Probe(format!("could not extract keyframe: {error}")))?;
    if !status.success() || !still.is_file() {
        return Err(Error::Probe(format!(
            "ffmpeg did not write a still at {seconds}s"
        )));
    }
    let bytes = std::fs::read(&still)?;
    if bytes.is_empty() {
        return Err(Error::Probe("keyframe jpeg is empty".to_owned()));
    }
    let (width, height) = jpeg_dimensions(&bytes).unzip();
    Ok(Keyframe {
        path: still,
        sha256: hex::encode(Sha256::digest(&bytes)),
        byte_length: bytes.len() as u64,
        width,
        height,
    })
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
