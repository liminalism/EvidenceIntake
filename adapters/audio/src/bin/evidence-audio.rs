//! Command-line shell for the audio intake adapter.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterArtifact, AdapterEvent, AdapterEventKind, AdapterJobRequest,
    AdapterProfile, AdapterResultManifest, AdapterSourceLocation, TemporalRelationArg,
};
use evidence_audio::{IntakeRequest, TrtWhisperBackend, map_json_file, transcribe};
use evidence_intake::{CaseId, TemporalRelation};
use sha2::{Digest, Sha256};

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
#[allow(
    clippy::large_enum_variant,
    reason = "command line is parsed once and specialist flags remain source-compatible"
)]
enum Command {
    /// Run one persistent GUI intake job and atomically publish its result manifest.
    Job {
        /// Adapter job request JSON.
        #[arg(long)]
        request: PathBuf,
    },
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
        /// Already-produced reference transcript JSON. Skips live inference.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Current-user TensorRT broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Installed TensorRT Whisper model pack.
        #[arg(long, default_value = "whisper-large-v3")]
        trt_model: String,
        /// Immutable model/export revision stamped on statements.
        #[arg(long)]
        trt_revision: Option<String>,
        /// Language hint passed to the TensorRT worker.
        #[arg(long, default_value = "en")]
        language: String,
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
        Command::Job { request } => run_job(&request)?,
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
            trt_endpoint,
            trt_model,
            trt_revision,
            language,
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
                map_json_file(&request, &json, "reference@from-json")?
            } else {
                let trt_revision = trt_revision.ok_or(
                    "--trt-revision is required when live TensorRT transcription is enabled",
                )?;
                transcribe(
                    &request,
                    &TrtWhisperBackend {
                        endpoint: trt_endpoint,
                        model: trt_model,
                        revision: trt_revision,
                        language: Some(language),
                    },
                )?
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

fn run_job(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let request: AdapterJobRequest = serde_json::from_slice(&fs::read(path)?)?;
    request.validate()?;
    let AdapterProfile::Audio {
        broker_endpoint,
        whisper,
        language,
        phone_band,
        level_split,
        gap_ms,
    } = &request.profile
    else {
        return Err("audio adapter received a non-audio profile".into());
    };
    verify_original(&request)?;
    fs::create_dir_all(&request.artifacts_dir)?;
    emit(
        &request,
        "transcription",
        AdapterEventKind::Started,
        None,
        None,
        "Transcribing referenced audio",
    )?;
    let intake = IntakeRequest {
        case_id: CaseId(request.case_id.clone()),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        path: request.original_path.clone(),
        logical_name: Some(request.logical_name.clone()),
        temporal_relation: temporal(request.temporal_relation),
        phone_band: *phone_band,
        level_split: *level_split,
        gap_ms: *gap_ms,
    };
    let batch = transcribe(
        &intake,
        &TrtWhisperBackend {
            endpoint: broker_endpoint.clone(),
            model: whisper.id.clone(),
            revision: whisper.revision.clone(),
            language: Some(language.clone()),
        },
    )?;
    let batch_path = request.artifacts_dir.join("normalized-batch.json");
    fs::write(&batch_path, serde_json::to_vec_pretty(&batch)?)?;
    let (batch_hash, _) = file_identity(&batch_path)?;
    let manifest = AdapterResultManifest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        batch_path: batch_path.clone(),
        keyframe_index_path: None,
        source_locations: vec![AdapterSourceLocation {
            source_id: request.source_id.clone(),
            path: request.original_path.clone(),
            sha256: request.original_sha256.clone(),
            byte_length: request.original_byte_length,
        }],
        artifacts: vec![AdapterArtifact {
            kind: "normalized_batch".to_owned(),
            path: batch_path,
            sha256: Some(batch_hash),
        }],
    };
    manifest.write_atomic(&request.artifacts_dir.join("result.json"))?;
    emit(
        &request,
        "completed",
        AdapterEventKind::Completed,
        Some(1),
        Some(1),
        "Audio result is ready to import",
    )?;
    Ok(())
}

fn emit(
    request: &AdapterJobRequest,
    stage: &str,
    kind: AdapterEventKind,
    completed: Option<u64>,
    total: Option<u64>,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let event = AdapterEvent {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        stage: stage.to_owned(),
        kind,
        completed,
        total,
        message: message.to_owned(),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &event)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn verify_original(request: &AdapterJobRequest) -> Result<(), Box<dyn std::error::Error>> {
    let (hash, length) = file_identity(&request.original_path)?;
    if length != request.original_byte_length
        || !hash.eq_ignore_ascii_case(&request.original_sha256)
    {
        return Err("referenced original changed after the job was queued".into());
    }
    Ok(())
}

fn file_identity(path: &PathBuf) -> Result<(String, u64), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), length))
}

fn temporal(value: TemporalRelationArg) -> TemporalRelation {
    match value {
        TemporalRelationArg::Contemporaneous => TemporalRelation::Contemporaneous,
        TemporalRelationArg::AfterEvent => TemporalRelation::AfterEvent,
        TemporalRelationArg::Mixed => TemporalRelation::Mixed,
        TemporalRelationArg::Unknown => TemporalRelation::Unknown,
    }
}
