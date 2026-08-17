//! Command-line shell for the audio intake adapter.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_audio::{IntakeRequest, WhisperxCliBackend, map_json_file, transcribe};

#[cfg(feature = "native-whisper")]
use evidence_audio::NativeWhisperBackend;
use evidence_intake::{CaseId, TemporalRelation};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Transcribe a discovery recording into a NormalizedBatch"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode, clean, transcribe, and write a NormalizedBatch JSON document.
    Transcribe {
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Existing production that will own the source.
        #[arg(long)]
        production: String,
        /// Adapter-assigned source identifier.
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
        /// Band-limit the working copy to the telephone band.
        #[arg(long)]
        phone_band: bool,
        /// Emit near/far level observations. Off because AGC flattens 911 audio.
        #[arg(long)]
        level_split: bool,
        /// Already-produced WhisperX JSON. Skips a live model.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Local ggml / gguf Whisper model, run in process. Preferred over the WhisperX CLI.
        #[arg(long)]
        model_path: Option<PathBuf>,
        /// WhisperX binary, used when `--from-json` and `--model-path` are omitted.
        #[arg(long, default_value = "whisperx")]
        whisperx: PathBuf,
        /// WhisperX `--model` value (Python backend only).
        #[arg(long, default_value = "large-v3")]
        model: String,
        /// Minimum hole, in milliseconds, that becomes a recording_gap.
        #[arg(long, default_value_t = 2_000)]
        gap_ms: u64,
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
        Command::Transcribe {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            phone_band,
            level_split,
            from_json,
            model_path,
            whisperx,
            model,
            gap_ms,
        } => {
            let request = IntakeRequest {
                case_id: CaseId(case),
                production_id: production,
                source_id,
                path: file,
                logical_name: None,
                temporal_relation: TemporalRelation::from(temporal),
                phone_band,
                level_split,
                gap_ms,
            };
            let batch = if let Some(json) = from_json {
                map_json_file(&request, &json, "whisperx@from-json")?
            } else if let Some(model_path) = model_path {
                transcribe_native(&request, model_path)?
            } else {
                let backend = WhisperxCliBackend {
                    bin: whisperx,
                    model,
                };
                transcribe(&request, &backend)?
            };
            let json = serde_json::to_string_pretty(&batch)?;
            if out.as_os_str() == "-" {
                let stdout = io::stdout();
                let mut lock = stdout.lock();
                lock.write_all(json.as_bytes())?;
                lock.write_all(b"\n")?;
            } else {
                fs::write(&out, format!("{json}\n"))?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "native-whisper")]
fn transcribe_native(
    request: &IntakeRequest,
    model_path: PathBuf,
) -> Result<evidence_intake::NormalizedBatch, Box<dyn std::error::Error>> {
    Ok(transcribe(request, &NativeWhisperBackend::new(model_path))?)
}

#[cfg(not(feature = "native-whisper"))]
fn transcribe_native(
    _request: &IntakeRequest,
    _model_path: PathBuf,
) -> Result<evidence_intake::NormalizedBatch, Box<dyn std::error::Error>> {
    Err(
        "this binary was built without the native-whisper feature. Rebuild with `--features native-whisper` and pass --model-path, or use --from-json."
            .into(),
    )
}
