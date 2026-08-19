//! Command-line shell for the video intake adapter.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_audio::NativeWhisperBackend;
use evidence_intake::NormalizedBatch;
use evidence_intake::{CaseId, TemporalRelation};
use evidence_video::{
    CaptionInput, ClockInput, DEFAULT_GAP_MS, DEFAULT_PROMPT, DEFAULT_THRESHOLD, DetectionInput,
    SceneRequest, SoundtrackInput, SyncOptions, SyncPair, SyncSide, TesseractCliBackend,
    VlmCliBackend, YoloCliBackend, analyze, cut_scenes, describe_from_json, describe_scenes,
    detect_from_json, detect_objects, sync_pair,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Cut a discovery video into scenes, boxes, and scene descriptions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(
    clippy::large_enum_variant,
    reason = "parsed once at startup; `Analyze` carries every optional backend flag"
)]
enum Command {
    /// Detect scene cuts, extract keyframe stills, and write a NormalizedBatch.
    Scenes {
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Existing production that will own the sources.
        #[arg(long)]
        production: String,
        /// Adapter-assigned source identifier of the original video.
        #[arg(long)]
        source_id: String,
        /// Path to the untouched original recording.
        #[arg(long)]
        file: PathBuf,
        /// Where to write the batch. `-` writes stdout.
        #[arg(long)]
        out: PathBuf,
        /// How the recording relates to the event. Defaults to unknown.
        #[arg(long, value_enum, default_value_t = TemporalArg::Unknown)]
        temporal: TemporalArg,
        /// ffmpeg scene score that counts as a cut.
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f64,
        /// Minimum missing tail, in milliseconds, that becomes a recording_gap.
        #[arg(long, default_value_t = DEFAULT_GAP_MS)]
        gap_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
    },
    /// Run a detector on scene stills (or load JSON) and write a NormalizedBatch.
    Detect {
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Existing production that will own the sources.
        #[arg(long)]
        production: String,
        /// Adapter-assigned source identifier of the original video.
        #[arg(long)]
        source_id: String,
        /// Path to the untouched original recording.
        #[arg(long)]
        file: PathBuf,
        /// Where to write the batch. `-` writes stdout.
        #[arg(long)]
        out: PathBuf,
        /// How the recording relates to the event. Defaults to unknown.
        #[arg(long, value_enum, default_value_t = TemporalArg::Unknown)]
        temporal: TemporalArg,
        /// ffmpeg scene score that counts as a cut.
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f64,
        /// Minimum missing tail, in milliseconds, that becomes a recording_gap.
        #[arg(long, default_value_t = DEFAULT_GAP_MS)]
        gap_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Already-produced detection JSON. Skips the YOLO CLI.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Ultralytics binary, used when `--from-json` is omitted.
        #[arg(long, default_value = "yolo")]
        yolo: PathBuf,
        /// Ultralytics model name or path.
        #[arg(long, default_value = "yolov8n.pt")]
        model: String,
        /// Minimum detector confidence.
        #[arg(long, default_value_t = 0.25)]
        confidence: f64,
    },
    /// Describe each scene still with a VLM (or load JSON) and write a NormalizedBatch.
    Describe {
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Existing production that will own the sources.
        #[arg(long)]
        production: String,
        /// Adapter-assigned source identifier of the original video.
        #[arg(long)]
        source_id: String,
        /// Path to the untouched original recording.
        #[arg(long)]
        file: PathBuf,
        /// Where to write the batch. `-` writes stdout.
        #[arg(long)]
        out: PathBuf,
        /// How the recording relates to the event. Defaults to unknown.
        #[arg(long, value_enum, default_value_t = TemporalArg::Unknown)]
        temporal: TemporalArg,
        /// ffmpeg scene score that counts as a cut.
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f64,
        /// Minimum missing tail, in milliseconds, that becomes a recording_gap.
        #[arg(long, default_value_t = DEFAULT_GAP_MS)]
        gap_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Already-produced caption JSON. Skips the VLM CLI.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// VLM binary, used when `--from-json` is omitted. Default `ollama`.
        #[arg(long, default_value = "ollama")]
        vlm: PathBuf,
        /// VLM model name (`llava`, `qwen2.5vl`, …).
        #[arg(long, default_value = "llava")]
        model: String,
        /// Instruction given with each still.
        #[arg(long, default_value = DEFAULT_PROMPT)]
        prompt: String,
    },
    /// Overnight job: cut scenes once, optionally detect and describe, one batch.
    Analyze {
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Existing production that will own the sources.
        #[arg(long)]
        production: String,
        /// Adapter-assigned source identifier of the original video.
        #[arg(long)]
        source_id: String,
        /// Path to the untouched original recording.
        #[arg(long)]
        file: PathBuf,
        /// Where to write the batch. `-` writes stdout.
        #[arg(long)]
        out: PathBuf,
        /// How the recording relates to the event. Defaults to unknown.
        #[arg(long, value_enum, default_value_t = TemporalArg::Unknown)]
        temporal: TemporalArg,
        /// ffmpeg scene score that counts as a cut.
        #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
        threshold: f64,
        /// Minimum missing tail, in milliseconds, that becomes a recording_gap.
        #[arg(long, default_value_t = DEFAULT_GAP_MS)]
        gap_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Run the YOLO CLI on each still.
        #[arg(long)]
        detect: bool,
        /// Already-produced detection JSON. Implies detect; skips the YOLO CLI.
        #[arg(long)]
        detect_json: Option<PathBuf>,
        /// Ultralytics binary.
        #[arg(long, default_value = "yolo")]
        yolo: PathBuf,
        /// Ultralytics model name or path.
        #[arg(long, default_value = "yolov8n.pt")]
        yolo_model: String,
        /// Minimum detector confidence.
        #[arg(long, default_value_t = 0.25)]
        confidence: f64,
        /// Run the VLM CLI on each still.
        #[arg(long)]
        describe: bool,
        /// Already-produced caption JSON. Implies describe; skips the VLM CLI.
        #[arg(long)]
        caption_json: Option<PathBuf>,
        /// VLM binary.
        #[arg(long, default_value = "ollama")]
        vlm: PathBuf,
        /// VLM model name.
        #[arg(long, default_value = "llava")]
        vlm_model: String,
        /// Instruction given with each still.
        #[arg(long, default_value = DEFAULT_PROMPT)]
        prompt: String,
        /// Already-produced clock-overlay JSON document. Skips OCR.
        #[arg(long)]
        clock_json: Option<PathBuf>,
        /// Read the burned-in clock overlay with the local `tesseract` CLI.
        #[arg(long)]
        tesseract: bool,
        /// Already-produced WhisperX JSON for the soundtrack. Skips ASR.
        #[arg(long)]
        transcript_json: Option<PathBuf>,
        /// Version stamped on statements read from `--transcript-json`.
        #[arg(long, default_value = "from-json")]
        transcript_version: String,
        /// Local ggml/gguf Whisper model. Transcribes the soundtrack in process.
        #[arg(long)]
        model_path: Option<PathBuf>,
    },
    /// Measure the audio offset between two recordings of one scene.
    Sync {
        /// Existing case identifier that already holds both sources.
        #[arg(long)]
        case: String,
        /// Source identifier of the recording measured from.
        #[arg(long)]
        a_source_id: String,
        /// Path to that untouched original.
        #[arg(long)]
        a_path: PathBuf,
        /// Display name for it. The file name when omitted.
        #[arg(long)]
        a_name: Option<String>,
        /// Source identifier of the recording measured to.
        #[arg(long)]
        b_source_id: String,
        /// Path to that untouched original.
        #[arg(long)]
        b_path: PathBuf,
        /// Display name for it. The file name when omitted.
        #[arg(long)]
        b_name: Option<String>,
        /// Where to write the batch. `-` writes stdout.
        #[arg(long)]
        out: PathBuf,
        /// Mono rate, in hertz, the correlation runs at.
        #[arg(long, default_value_t = SyncOptions::default().work_rate_hz)]
        work_rate: u32,
        /// Largest offset considered, in milliseconds, either direction.
        #[arg(long, default_value_t = SyncOptions::default().max_lag_ms)]
        max_lag_ms: u64,
        /// Least peak height, in units of the correlation's own RMS.
        #[arg(long, default_value_t = SyncOptions::default().min_prominence)]
        min_prominence: f64,
        /// Least normalized correlation coefficient that counts as a match.
        #[arg(long, default_value_t = SyncOptions::default().min_peak)]
        min_peak: f64,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TemporalArg {
    Contemporaneous,
    AfterEvent,
    Mixed,
    Unknown,
}

impl From<TemporalArg> for TemporalRelation {
    fn from(value: TemporalArg) -> Self {
        match value {
            TemporalArg::Contemporaneous => Self::Contemporaneous,
            TemporalArg::AfterEvent => Self::AfterEvent,
            TemporalArg::Mixed => Self::Mixed,
            TemporalArg::Unknown => Self::Unknown,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Scenes {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            stills_dir,
        } => {
            let request = SceneRequest {
                case_id: CaseId(case),
                production_id: production,
                source_id,
                path: file,
                logical_name: None,
                temporal_relation: TemporalRelation::from(temporal),
                threshold,
                gap_ms,
                stills_dir,
            };
            let batch = cut_scenes(&request)?;
            write_batch(&out, &batch)?;
        }
        Command::Detect {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            stills_dir,
            from_json,
            yolo,
            model,
            confidence,
        } => {
            let request = SceneRequest {
                case_id: CaseId(case),
                production_id: production,
                source_id,
                path: file,
                logical_name: None,
                temporal_relation: TemporalRelation::from(temporal),
                threshold,
                gap_ms,
                stills_dir,
            };
            let batch = if let Some(json) = from_json {
                detect_from_json(&request, &json)?
            } else {
                let backend = YoloCliBackend {
                    bin: yolo,
                    model,
                    confidence,
                };
                detect_objects(&request, &backend)?
            };
            write_batch(&out, &batch)?;
        }
        Command::Describe {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            stills_dir,
            from_json,
            vlm,
            model,
            prompt,
        } => {
            let request = SceneRequest {
                case_id: CaseId(case),
                production_id: production,
                source_id,
                path: file,
                logical_name: None,
                temporal_relation: TemporalRelation::from(temporal),
                threshold,
                gap_ms,
                stills_dir,
            };
            let batch = if let Some(json) = from_json {
                describe_from_json(&request, &json)?
            } else {
                let backend = VlmCliBackend {
                    bin: vlm,
                    model,
                    prompt,
                };
                describe_scenes(&request, &backend)?
            };
            write_batch(&out, &batch)?;
        }
        Command::Analyze {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            stills_dir,
            detect,
            detect_json,
            yolo,
            yolo_model,
            confidence,
            describe,
            caption_json,
            vlm,
            vlm_model,
            prompt,
            clock_json,
            tesseract,
            transcript_json,
            transcript_version,
            model_path,
        } => {
            let request = SceneRequest {
                case_id: CaseId(case),
                production_id: production,
                source_id,
                path: file,
                logical_name: None,
                temporal_relation: TemporalRelation::from(temporal),
                threshold,
                gap_ms,
                stills_dir,
            };
            let yolo_backend = YoloCliBackend {
                bin: yolo,
                model: yolo_model,
                confidence,
            };
            let vlm_backend = VlmCliBackend {
                bin: vlm,
                model: vlm_model,
                prompt,
            };
            let detections = if let Some(path) = detect_json.as_deref() {
                DetectionInput::Json(path)
            } else if detect {
                DetectionInput::Live(&yolo_backend)
            } else {
                DetectionInput::None
            };
            let captions = if let Some(path) = caption_json.as_deref() {
                CaptionInput::Json(path)
            } else if describe {
                CaptionInput::Live(&vlm_backend)
            } else {
                CaptionInput::None
            };
            let ocr = TesseractCliBackend::default_local();
            let clocks = if let Some(path) = clock_json.as_deref() {
                ClockInput::Json(path)
            } else if tesseract {
                ClockInput::Live(&ocr)
            } else {
                ClockInput::None
            };
            let whisper = model_path.map(NativeWhisperBackend::new);
            let soundtrack = if let Some(path) = transcript_json.as_deref() {
                SoundtrackInput::Json {
                    path,
                    version: &transcript_version,
                }
            } else if let Some(backend) = whisper.as_ref() {
                SoundtrackInput::Live(backend)
            } else {
                SoundtrackInput::None
            };
            let batch = analyze(&request, detections, captions, clocks, soundtrack)?;
            write_batch(&out, &batch)?;
        }
        Command::Sync {
            case,
            a_source_id,
            a_path,
            a_name,
            b_source_id,
            b_path,
            b_name,
            out,
            work_rate,
            max_lag_ms,
            min_prominence,
            min_peak,
        } => {
            let pair = SyncPair {
                case_id: CaseId(case),
                a: sync_side(a_source_id, a_name, a_path),
                b: sync_side(b_source_id, b_name, b_path),
            };
            let options = SyncOptions {
                work_rate_hz: work_rate,
                max_lag_ms,
                min_prominence,
                min_peak,
            };
            if let Some(batch) = sync_pair(&pair, &options)? {
                write_batch(&out, &batch)?;
            } else {
                // An honest nothing is a result: no edge is written, and the
                // exit status stays zero so a batch job keeps going.
                let reason = format!(
                    "no correlation peak reached the floors \
                     (min-peak {min_peak}, min-prominence {min_prominence}) \
                     over lags up to {max_lag_ms} ms at {work_rate} Hz"
                );
                let report = SyncReport {
                    synced: false,
                    reason,
                };
                println!("{}", serde_json::to_string(&report)?);
            }
        }
    }
    Ok(())
}

/// What `sync` prints when the correlation found nothing worth proposing.
#[derive(Debug, Serialize)]
struct SyncReport {
    /// Always false; a measured offset is written to `--out` instead.
    synced: bool,
    /// Why nothing was proposed, in the operator's own thresholds.
    reason: String,
}

/// One side of a sync pair, named by the file when the operator did not.
fn sync_side(source_id: String, logical_name: Option<String>, path: PathBuf) -> SyncSide {
    let logical_name = logical_name.unwrap_or_else(|| {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recording.mp4")
            .to_owned()
    });
    SyncSide {
        source_id,
        logical_name,
        path,
    }
}

fn write_batch(path: &PathBuf, batch: &NormalizedBatch) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(batch)?;
    if path.as_os_str() == "-" {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(json.as_bytes())?;
        lock.write_all(b"\n")?;
    } else {
        fs::write(path, format!("{json}\n"))?;
    }
    Ok(())
}
