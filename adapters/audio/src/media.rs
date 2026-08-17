//! Open a local wav or a local video's audio track.
//!
//! Video-transcriber-mcp does this with ffmpeg for local files (and yt-dlp for
//! URLs). We take the local-file half only: discovery audio is already on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

use evidence_intake::SourceKind;
use sha2::{Digest, Sha256};

use crate::decode::{DecodedAudio, decode_wav};
use crate::{Error, Result};

/// A local original plus the PCM of its audio track.
#[derive(Debug, Clone)]
pub struct OpenedMedia {
    /// SHA-256 of the untouched original bytes.
    pub sha256: String,
    /// Length of the untouched original.
    pub byte_length: u64,
    /// MIME type of the untouched original.
    pub media_type: String,
    /// Audio file, or a video whose soundtrack we pulled.
    pub source_kind: SourceKind,
    /// PCM of the audio track. For a wav this *is* the original; for a video
    /// it is a derived extract used only for analysis and ASR.
    pub audio: DecodedAudio,
}

/// Classify a path as wav, other audio, or a video container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaClass {
    /// Native wav we can decode without ffmpeg.
    Wav,
    /// Compressed audio (mp3, m4a, …) that needs ffmpeg.
    AudioContainer,
    /// Video whose audio track we will extract.
    Video,
}

/// Guess from the extension. Unknown extensions are treated as video so a
/// bodycam `.unknown` still goes through ffmpeg rather than the wav parser.
pub fn classify(path: &Path) -> MediaClass {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wav" | "wave") => MediaClass::Wav,
        Some("mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "wma") => MediaClass::AudioContainer,
        _ => MediaClass::Video,
    }
}

/// MIME type of the original, from the extension.
pub fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wav" | "wave") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("ogg" | "opus") => "audio/ogg",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

/// Hash the original and return PCM of its audio track.
pub fn open_media(path: &Path) -> Result<OpenedMedia> {
    match classify(path) {
        MediaClass::Wav => {
            let audio = decode_wav(path)?;
            Ok(OpenedMedia {
                sha256: audio.sha256.clone(),
                byte_length: audio.byte_length,
                media_type: audio.media_type.clone(),
                source_kind: SourceKind::Audio,
                audio,
            })
        }
        class => {
            let bytes = std::fs::read(path).map_err(|error| Error::Decode {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
            if bytes.is_empty() {
                return Err(Error::Empty(format!("`{}` is empty", path.display())));
            }
            let sha256 = hex::encode(Sha256::digest(&bytes));
            let byte_length = bytes.len() as u64;
            let extracted = extract_wav(path)?;
            let mut audio = decode_wav(&extracted)?;
            // The extract is a working copy. Identity belongs to the original.
            audio.sha256.clone_from(&sha256);
            audio.byte_length = byte_length;
            media_type(path).clone_into(&mut audio.media_type);
            Ok(OpenedMedia {
                sha256,
                byte_length,
                media_type: media_type(path).to_owned(),
                source_kind: match class {
                    MediaClass::Video => SourceKind::Video,
                    _ => SourceKind::Audio,
                },
                audio,
            })
        }
    }
}

/// Pull a PCM wav out of a container with ffmpeg, keeping the original
/// channel count so stereo analysis still sees L/R.
pub fn extract_wav(path: &Path) -> Result<PathBuf> {
    let out = tempfile::Builder::new()
        .prefix("evidence-audio-extract-")
        .suffix(".wav")
        .tempfile()
        .map_err(|error| Error::Extract {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let out_path = out.path().to_path_buf();
    // Keep the file after NamedTempFile drops; the caller deletes via OS tmp.
    let persist = out.into_temp_path();
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-acodec", "pcm_s16le"])
        .arg(&out_path)
        .status()
        .map_err(|error| Error::Extract {
            path: path.to_path_buf(),
            message: format!(
                "could not run ffmpeg: {error}. Install ffmpeg to transcribe video or compressed audio."
            ),
        })?;
    if !status.success() {
        return Err(Error::Extract {
            path: path.to_path_buf(),
            message: format!("ffmpeg exited with {status}"),
        });
    }
    // Leak the persist guard so the path stays until process end.
    std::mem::forget(persist);
    Ok(out_path)
}

/// Whether `ffmpeg` is on PATH. Tests skip extraction when it is not.
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}
