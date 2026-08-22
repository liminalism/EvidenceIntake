//! Command-line document adapter.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterArtifact, AdapterEvent, AdapterEventKind, AdapterJobRequest,
    AdapterProfile, AdapterResultManifest, AdapterSourceLocation, TemporalRelationArg,
};
use evidence_document::{
    BrokeredLegeOcrCliBackend, DocumentRequest, LegeOcrCliBackend, from_docir, process,
};
use evidence_intake::{CaseId, TemporalRelation};
use sha2::{Digest, Sha256};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run one persistent GUI intake job and publish its result manifest.
    Job {
        /// Adapter job request JSON.
        #[arg(long)]
        request: PathBuf,
    },
    /// Run packaged Lege OCR and emit a NormalizedBatch.
    Process {
        /// Untouched PDF.
        pdf: PathBuf,
        #[command(flatten)]
        identity: IdentityArgs,
        /// Durable output directory for canonical DocIR and QA artifacts.
        #[arg(long)]
        artifacts_dir: PathBuf,
        /// Packaged Lege OCR executable.
        #[arg(long, default_value = "lege-ocr")]
        lege_ocr: PathBuf,
        /// Packaged TensorRT OCR root.
        #[arg(long)]
        tensorrt_ocr_root: Option<PathBuf>,
        /// Evidence page-OCR bridge; selects brokered TensorRT when present.
        #[arg(long)]
        broker_bridge: Option<PathBuf>,
        /// Evidence broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        broker_endpoint: String,
        /// Broker OCR model identifier.
        #[arg(long, default_value = "turbo-ocr")]
        broker_model: String,
        /// Immutable broker OCR model revision.
        #[arg(long)]
        broker_revision: Option<String>,
        /// NormalizedBatch output, or `-` for stdout.
        #[arg(long, default_value = "-")]
        output: PathBuf,
    },
    /// Map an existing canonical DocIR artifact without inference.
    FromDocir {
        /// Untouched PDF described by DocIR.
        pdf: PathBuf,
        /// Canonical `.lege.json` artifact.
        docir: PathBuf,
        #[command(flatten)]
        identity: IdentityArgs,
        /// NormalizedBatch output, or `-` for stdout.
        #[arg(long, default_value = "-")]
        output: PathBuf,
    },
}

#[derive(Debug, clap::Args)]
struct IdentityArgs {
    /// Existing case identifier.
    #[arg(long)]
    case: String,
    /// Existing production identifier.
    #[arg(long)]
    production: String,
    /// Adapter-assigned source identifier.
    #[arg(long)]
    source: String,
    /// Display name; PDF file name when omitted.
    #[arg(long)]
    logical_name: Option<String>,
    /// Relationship between document creation and the event.
    #[arg(long, value_enum, default_value_t = TemporalArg::Unknown)]
    temporal_relation: TemporalArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TemporalArg {
    Contemporaneous,
    AfterEvent,
    Mixed,
    Unknown,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("evidence-document: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Command::Job { request } = &cli.command {
        return run_job(request);
    }
    let (batch, output) = match cli.command {
        Command::Job { .. } => unreachable!("handled above"),
        Command::Process {
            pdf,
            identity,
            artifacts_dir,
            lege_ocr,
            tensorrt_ocr_root,
            broker_bridge,
            broker_endpoint,
            broker_model,
            broker_revision,
            output,
        } => {
            let request = request(pdf, identity, artifacts_dir);
            let batch = if let Some(bridge) = broker_bridge {
                process(
                    &request,
                    &BrokeredLegeOcrCliBackend {
                        bin: lege_ocr,
                        bridge,
                        endpoint: broker_endpoint,
                        model: broker_model,
                        revision: broker_revision
                            .ok_or("--broker-revision is required with --broker-bridge")?,
                    },
                )?
            } else {
                process(
                    &request,
                    &LegeOcrCliBackend {
                        bin: lege_ocr,
                        tensorrt_root: tensorrt_ocr_root,
                    },
                )?
            };
            (batch, output)
        }
        Command::FromDocir {
            pdf,
            docir,
            identity,
            output,
        } => {
            let request = request(pdf, identity, PathBuf::new());
            (from_docir(&request, &docir)?, output)
        }
    };
    let json = serde_json::to_vec_pretty(&batch)?;
    if output.as_os_str() == "-" {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&json)?;
        stdout.write_all(b"\n")?;
    } else {
        std::fs::write(output, json)?;
    }
    Ok(())
}

fn run_job(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let request: AdapterJobRequest = serde_json::from_slice(&std::fs::read(path)?)?;
    request.validate()?;
    let AdapterProfile::Document {
        lege_ocr,
        broker_bridge,
        broker_endpoint,
        ocr,
    } = &request.profile
    else {
        return Err("document adapter received a non-document profile".into());
    };
    verify_original(&request)?;
    std::fs::create_dir_all(&request.artifacts_dir)?;
    emit(
        &request,
        "document_ocr",
        AdapterEventKind::Started,
        "Running Lege search-profile OCR",
    )?;
    let document_request = DocumentRequest {
        case_id: CaseId(request.case_id.clone()),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        path: request.original_path.clone(),
        logical_name: Some(request.logical_name.clone()),
        temporal_relation: temporal(request.temporal_relation),
        artifacts_dir: request.artifacts_dir.join("lege"),
    };
    let batch = process(
        &document_request,
        &BrokeredLegeOcrCliBackend {
            bin: lege_ocr.clone(),
            bridge: broker_bridge.clone(),
            endpoint: broker_endpoint.clone(),
            model: ocr.id.clone(),
            revision: ocr.revision.clone(),
        },
    )?;
    let batch_path = request.artifacts_dir.join("normalized-batch.json");
    std::fs::write(&batch_path, serde_json::to_vec_pretty(&batch)?)?;
    let mut artifacts = collect_artifacts(&document_request.artifacts_dir)?;
    let (batch_hash, _) = file_identity(&batch_path)?;
    artifacts.push(AdapterArtifact {
        kind: "normalized_batch".to_owned(),
        path: batch_path.clone(),
        sha256: Some(batch_hash),
    });
    let manifest = AdapterResultManifest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        batch_path,
        keyframe_index_path: None,
        source_locations: vec![AdapterSourceLocation {
            source_id: request.source_id.clone(),
            path: request.original_path.clone(),
            sha256: request.original_sha256.clone(),
            byte_length: request.original_byte_length,
        }],
        artifacts,
    };
    manifest.write_atomic(&request.artifacts_dir.join("result.json"))?;
    emit(
        &request,
        "completed",
        AdapterEventKind::Completed,
        "Document result is ready to import",
    )?;
    Ok(())
}

fn collect_artifacts(root: &Path) -> Result<Vec<AdapterArtifact>, Box<dyn std::error::Error>> {
    let mut pending = vec![root.to_path_buf()];
    let mut artifacts = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let (hash, _) = file_identity(&path)?;
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                artifacts.push(AdapterArtifact {
                    kind: if name.ends_with(".lege.json") {
                        "docir"
                    } else {
                        "qa"
                    }
                    .to_owned(),
                    path,
                    sha256: Some(hash),
                });
            }
        }
    }
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn emit(
    request: &AdapterJobRequest,
    stage: &str,
    kind: AdapterEventKind,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let event = AdapterEvent {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        stage: stage.to_owned(),
        kind,
        completed: None,
        total: None,
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

    let mut file = std::fs::File::open(path)?;
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

fn request(pdf: PathBuf, args: IdentityArgs, artifacts_dir: PathBuf) -> DocumentRequest {
    DocumentRequest {
        case_id: CaseId(args.case),
        production_id: args.production,
        source_id: args.source,
        path: pdf,
        logical_name: args.logical_name,
        temporal_relation: match args.temporal_relation {
            TemporalArg::Contemporaneous => TemporalRelation::Contemporaneous,
            TemporalArg::AfterEvent => TemporalRelation::AfterEvent,
            TemporalArg::Mixed => TemporalRelation::Mixed,
            TemporalArg::Unknown => TemporalRelation::Unknown,
        },
        artifacts_dir,
    }
}
