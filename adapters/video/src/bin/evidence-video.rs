//! Command-line shell for the video intake adapter.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_intake::NormalizedBatch;
use evidence_intake::{CaseId, TemporalRelation};
use evidence_video::{
    CaptionInput, DEFAULT_GAP_MS, DEFAULT_PROMPT, DEFAULT_THRESHOLD, DetectionInput, SceneRequest,
    VlmCliBackend, YoloCliBackend, analyze, cut_scenes, describe_from_json, describe_scenes,
    detect_from_json, detect_objects,
};

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
            let batch = analyze(&request, detections, captions)?;
            write_batch(&out, &batch)?;
        }
    }
    Ok(())
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
