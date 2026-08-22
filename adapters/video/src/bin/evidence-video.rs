//! Command-line shell for the video intake adapter.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterArtifact, AdapterEvent, AdapterEventKind, AdapterJobRequest,
    AdapterProfile, AdapterResultManifest, AdapterSourceLocation, TemporalRelationArg, VideoTier,
};
use evidence_audio::TrtWhisperBackend;
use evidence_intake::NormalizedBatch;
use evidence_intake::{CaseId, Store, TemporalRelation};
use evidence_video::{
    CaptionInput, ClockInput, DEFAULT_GAP_MS, DEFAULT_PROMPT, DEFAULT_SAMPLE_DEDUP_MS,
    DEFAULT_SAMPLE_GAP_MS, DEFAULT_THRESHOLD, DetectionInput, EmbeddingBackend,
    JsonEmbeddingBackend, SceneRequest, SoundtrackInput, SyncOptions, SyncPair, SyncSide,
    TrtCaptionBackend, TrtClockBackend, TrtEmbeddingBackend, TrtVisionBackend, analyze,
    analyze_and_embed, cut_scenes, describe_from_json, describe_scenes, detect_from_json,
    detect_objects, embed_from_json, embed_scenes, sync_pair,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
    reason = "parsed once at startup; `Analyze` and `Embed` carry optional backend flags"
)]
enum Command {
    /// Run one persistent GUI intake job and publish its result manifest.
    Job {
        /// Adapter job request JSON.
        #[arg(long)]
        request: PathBuf,
    },
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
        /// Longest intended interval between retained visual-index frames.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_GAP_MS)]
        sample_gap_ms: u64,
        /// Suppress a scene-triggered frame this close to the previous sample.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_DEDUP_MS)]
        sample_dedup_ms: u64,
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
        /// Longest intended interval between retained visual-index frames.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_GAP_MS)]
        sample_gap_ms: u64,
        /// Suppress a scene-triggered frame this close to the previous sample.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_DEDUP_MS)]
        sample_dedup_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Already-produced detection JSON. Skips TensorRT inference.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Local TensorRT broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Checksum-pinned detector model-pack identifier.
        #[arg(long, default_value = "yolo-v8n")]
        model: String,
        /// Immutable detector export revision.
        #[arg(long)]
        revision: Option<String>,
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
        /// Longest intended interval between retained visual-index frames.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_GAP_MS)]
        sample_gap_ms: u64,
        /// Suppress a scene-triggered frame this close to the previous sample.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_DEDUP_MS)]
        sample_dedup_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Already-produced caption JSON. Skips TensorRT inference.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Local TensorRT broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Checksum-pinned TensorRT-LLM model-pack identifier.
        #[arg(long, default_value = "qwen2-vl-2b")]
        model: String,
        /// Immutable TensorRT-LLM export revision.
        #[arg(long)]
        revision: Option<String>,
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
        /// Longest intended interval between retained visual-index frames.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_GAP_MS)]
        sample_gap_ms: u64,
        /// Suppress a scene-triggered frame this close to the previous sample.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_DEDUP_MS)]
        sample_dedup_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Run the TensorRT detector on each still.
        #[arg(long)]
        detect: bool,
        /// Already-produced detection JSON. Implies detect; skips inference.
        #[arg(long)]
        detect_json: Option<PathBuf>,
        /// Local TensorRT broker endpoint shared by every live model.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Checksum-pinned detector model-pack identifier.
        #[arg(long, default_value = "yolo-v8n")]
        detect_model: String,
        /// Immutable detector export revision.
        #[arg(long)]
        detect_revision: Option<String>,
        /// Minimum detector confidence.
        #[arg(long, default_value_t = 0.25)]
        confidence: f64,
        /// Run the TensorRT-LLM caption model on each still.
        #[arg(long)]
        describe: bool,
        /// Already-produced caption JSON. Implies describe; skips inference.
        #[arg(long)]
        caption_json: Option<PathBuf>,
        /// Checksum-pinned TensorRT-LLM model-pack identifier.
        #[arg(long, default_value = "qwen2-vl-2b")]
        caption_model: String,
        /// Immutable caption export revision.
        #[arg(long)]
        caption_revision: Option<String>,
        /// Instruction given with each still.
        #[arg(long, default_value = DEFAULT_PROMPT)]
        prompt: String,
        /// Already-produced clock-overlay JSON document. Skips OCR.
        #[arg(long)]
        clock_json: Option<PathBuf>,
        /// Read burned-in clock overlays with the shared TensorRT OCR model.
        #[arg(long)]
        clock_ocr: bool,
        /// Checksum-pinned OCR model-pack identifier.
        #[arg(long, default_value = "turbo-ocr")]
        ocr_model: String,
        /// Immutable OCR export revision.
        #[arg(long)]
        ocr_revision: Option<String>,
        /// Already-produced WhisperX JSON for the soundtrack. Skips ASR.
        #[arg(long)]
        transcript_json: Option<PathBuf>,
        /// Version stamped on statements read from `--transcript-json`.
        #[arg(long, default_value = "from-json")]
        transcript_version: String,
        /// Transcribe the soundtrack with the shared TensorRT Whisper model.
        #[arg(long)]
        transcribe: bool,
        /// Checksum-pinned Whisper model-pack identifier.
        #[arg(long, default_value = "whisper-large-v3")]
        whisper_model: String,
        /// Immutable Whisper export revision.
        #[arg(long)]
        whisper_revision: Option<String>,
        /// Optional language hint for Whisper.
        #[arg(long)]
        whisper_language: Option<String>,
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
    /// Embed each scene keyframe and write a KeyframeIndex.
    Embed {
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
        /// Where to write the index. `-` writes stdout.
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
        /// Longest intended interval between retained visual-index frames.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_GAP_MS)]
        sample_gap_ms: u64,
        /// Suppress a scene-triggered frame this close to the previous sample.
        #[arg(long, default_value_t = DEFAULT_SAMPLE_DEDUP_MS)]
        sample_dedup_ms: u64,
        /// Directory to keep derived jpeg stills. Temp when omitted.
        #[arg(long)]
        stills_dir: Option<PathBuf>,
        /// Already-produced embedding JSON. Skips TensorRT inference.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Local TensorRT broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Checksum-pinned model pack and embedding-space name.
        #[arg(long, default_value = "siglip2-base-patch16-384")]
        model: String,
        /// Immutable embedding export revision.
        #[arg(long)]
        revision: Option<String>,
    },
    /// Find stills matching a text query. Prints KeyframeHit JSON.
    Find {
        /// `SQLite` case database that already holds the stills and vectors.
        #[arg(long, default_value = "evidence.sqlite")]
        database: PathBuf,
        /// Existing case identifier.
        #[arg(long)]
        case: String,
        /// Text to embed and compare against stored stills.
        query: String,
        /// Embedding space the stills were indexed under.
        #[arg(long, default_value = "siglip2-base-patch16-384")]
        model: String,
        /// Most hits to return.
        #[arg(long, default_value_t = 25)]
        limit: u32,
        /// Already-produced embedding JSON with a matching `queries` entry.
        #[arg(long)]
        from_json: Option<PathBuf>,
        /// Local TensorRT broker endpoint.
        #[arg(long, default_value = "evidence-trt")]
        trt_endpoint: String,
        /// Immutable embedding export revision.
        #[arg(long)]
        revision: Option<String>,
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
    // A normalized video batch can contain many nested source/segment records.
    // Windows executables otherwise start `main` with a small stack that can
    // overflow while serde constructs or drops an ordinary two-minute batch.
    let worker = std::thread::Builder::new()
        .name("evidence-video".to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| run().map_err(|error| error.to_string()))
        .unwrap_or_else(|error| {
            eprintln!("error: could not start evidence-video worker: {error}");
            std::process::exit(1);
        });
    match worker.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Job { request } => run_job(&request)?,
        Command::Scenes {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            sample_gap_ms,
            sample_dedup_ms,
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
                max_sample_gap_ms: sample_gap_ms,
                sample_dedup_ms,
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
            sample_gap_ms,
            sample_dedup_ms,
            stills_dir,
            from_json,
            trt_endpoint,
            model,
            revision,
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
                max_sample_gap_ms: sample_gap_ms,
                sample_dedup_ms,
                stills_dir,
            };
            let batch = if let Some(json) = from_json {
                detect_from_json(&request, &json)?
            } else {
                let backend = TrtVisionBackend {
                    endpoint: trt_endpoint,
                    model,
                    revision: require_revision(revision, "--revision")?,
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
            sample_gap_ms,
            sample_dedup_ms,
            stills_dir,
            from_json,
            trt_endpoint,
            model,
            revision,
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
                max_sample_gap_ms: sample_gap_ms,
                sample_dedup_ms,
                stills_dir,
            };
            let batch = if let Some(json) = from_json {
                describe_from_json(&request, &json)?
            } else {
                let backend = TrtCaptionBackend {
                    endpoint: trt_endpoint,
                    model,
                    revision: require_revision(revision, "--revision")?,
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
            sample_gap_ms,
            sample_dedup_ms,
            stills_dir,
            detect,
            detect_json,
            trt_endpoint,
            detect_model,
            detect_revision,
            confidence,
            describe,
            caption_json,
            caption_model,
            caption_revision,
            prompt,
            clock_json,
            clock_ocr,
            ocr_model,
            ocr_revision,
            transcript_json,
            transcript_version,
            transcribe,
            whisper_model,
            whisper_revision,
            whisper_language,
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
                max_sample_gap_ms: sample_gap_ms,
                sample_dedup_ms,
                stills_dir,
            };
            let detector = (detect && detect_json.is_none())
                .then(|| {
                    Ok::<_, Box<dyn std::error::Error>>(TrtVisionBackend {
                        endpoint: trt_endpoint.clone(),
                        model: detect_model,
                        revision: require_revision(detect_revision, "--detect-revision")?,
                        confidence,
                    })
                })
                .transpose()?;
            let detections = if let Some(path) = detect_json.as_deref() {
                DetectionInput::Json(path)
            } else if let Some(backend) = detector.as_ref() {
                DetectionInput::Live(backend)
            } else {
                DetectionInput::None
            };
            let captioner = (describe && caption_json.is_none())
                .then(|| {
                    Ok::<_, Box<dyn std::error::Error>>(TrtCaptionBackend {
                        endpoint: trt_endpoint.clone(),
                        model: caption_model,
                        revision: require_revision(caption_revision, "--caption-revision")?,
                        prompt,
                    })
                })
                .transpose()?;
            let captions = if let Some(path) = caption_json.as_deref() {
                CaptionInput::Json(path)
            } else if let Some(backend) = captioner.as_ref() {
                CaptionInput::Live(backend)
            } else {
                CaptionInput::None
            };
            let ocr = (clock_ocr && clock_json.is_none())
                .then(|| {
                    Ok::<_, Box<dyn std::error::Error>>(TrtClockBackend {
                        endpoint: trt_endpoint.clone(),
                        model: ocr_model,
                        revision: require_revision(ocr_revision, "--ocr-revision")?,
                        language: "eng".to_owned(),
                    })
                })
                .transpose()?;
            let clocks = if let Some(path) = clock_json.as_deref() {
                ClockInput::Json(path)
            } else if let Some(backend) = ocr.as_ref() {
                ClockInput::Live(backend)
            } else {
                ClockInput::None
            };
            let whisper = (transcribe && transcript_json.is_none())
                .then(|| {
                    Ok::<_, Box<dyn std::error::Error>>(TrtWhisperBackend {
                        endpoint: trt_endpoint,
                        model: whisper_model,
                        revision: require_revision(whisper_revision, "--whisper-revision")?,
                        language: whisper_language,
                    })
                })
                .transpose()?;
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
        Command::Embed {
            case,
            production,
            source_id,
            file,
            out,
            temporal,
            threshold,
            gap_ms,
            sample_gap_ms,
            sample_dedup_ms,
            stills_dir,
            from_json,
            trt_endpoint,
            model,
            revision,
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
                max_sample_gap_ms: sample_gap_ms,
                sample_dedup_ms,
                stills_dir,
            };
            let index = if let Some(json) = from_json {
                embed_from_json(&request, &json)?
            } else {
                let backend = TrtEmbeddingBackend {
                    endpoint: trt_endpoint,
                    model,
                    revision: require_revision(revision, "--revision")?,
                };
                embed_scenes(&request, &backend)?
            };
            write_json(&out, &index)?;
        }
        Command::Find {
            database,
            case,
            query,
            model,
            limit,
            from_json,
            trt_endpoint,
            revision,
        } => {
            let store = Store::open(database)?;
            let vector = if let Some(json) = from_json {
                JsonEmbeddingBackend { path: json }.embed_query(&query)?
            } else {
                let backend = TrtEmbeddingBackend {
                    endpoint: trt_endpoint,
                    model: model.clone(),
                    revision: require_revision(revision, "--revision")?,
                };
                backend.embed_query(&query)?
            };
            let hits = store.search_keyframes(&CaseId(case), &model, &vector, limit)?;
            println!("{}", serde_json::to_string_pretty(&hits)?);
        }
    }
    Ok(())
}

fn run_job(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let request: AdapterJobRequest = serde_json::from_slice(&fs::read(path)?)?;
    request.validate()?;
    let AdapterProfile::Video {
        broker_endpoint,
        tier,
        ocr,
        whisper,
        embedding,
        detector,
        caption,
        language,
        threshold,
        gap_ms,
        sample_gap_ms,
        sample_dedup_ms,
        detector_confidence,
        caption_prompt,
    } = &request.profile
    else {
        return Err("video adapter received a non-video profile".into());
    };
    verify_original(&request)?;
    fs::create_dir_all(&request.artifacts_dir)?;
    let stills_dir = request.artifacts_dir.join("stills");
    fs::create_dir_all(&stills_dir)?;
    emit_job(
        &request,
        "scene_preparation",
        AdapterEventKind::Started,
        "Detecting scenes and retaining keyframes",
    )?;
    let scene_request = SceneRequest {
        case_id: CaseId(request.case_id.clone()),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        path: request.original_path.clone(),
        logical_name: Some(request.logical_name.clone()),
        temporal_relation: temporal_job(request.temporal_relation),
        threshold: *threshold,
        gap_ms: *gap_ms,
        max_sample_gap_ms: *sample_gap_ms,
        sample_dedup_ms: *sample_dedup_ms,
        stills_dir: Some(stills_dir.clone()),
    };
    let clock = TrtClockBackend {
        endpoint: broker_endpoint.clone(),
        model: ocr.id.clone(),
        revision: ocr.revision.clone(),
        language: "eng".to_owned(),
    };
    let speech = TrtWhisperBackend {
        endpoint: broker_endpoint.clone(),
        model: whisper.id.clone(),
        revision: whisper.revision.clone(),
        language: Some(language.clone()),
    };
    let detector_backend = detector.as_ref().map(|model| TrtVisionBackend {
        endpoint: broker_endpoint.clone(),
        model: model.id.clone(),
        revision: model.revision.clone(),
        confidence: *detector_confidence,
    });
    let caption_backend = caption.as_ref().map(|model| TrtCaptionBackend {
        endpoint: broker_endpoint.clone(),
        model: model.id.clone(),
        revision: model.revision.clone(),
        prompt: caption_prompt.clone(),
    });
    let embedding_backend = embedding.as_ref().map(|model| TrtEmbeddingBackend {
        endpoint: broker_endpoint.clone(),
        model: model.id.clone(),
        revision: model.revision.clone(),
    });
    let detections = match tier {
        VideoTier::Tier1 => DetectionInput::None,
        VideoTier::Overnight => DetectionInput::Live(
            detector_backend
                .as_ref()
                .ok_or("overnight profile omitted its detector")?,
        ),
    };
    let captions = match tier {
        VideoTier::Tier1 => CaptionInput::None,
        VideoTier::Overnight => CaptionInput::Live(
            caption_backend
                .as_ref()
                .ok_or("overnight profile omitted its caption model")?,
        ),
    };
    let (batch, index) = analyze_and_embed(
        &scene_request,
        detections,
        captions,
        ClockInput::Live(&clock),
        SoundtrackInput::Live(&speech),
        embedding_backend
            .as_ref()
            .map(|backend| backend as &dyn EmbeddingBackend),
    )?;
    let batch_path = request.artifacts_dir.join("normalized-batch.json");
    fs::write(&batch_path, serde_json::to_vec_pretty(&batch)?)?;
    let index_path = index
        .as_ref()
        .map(|_| request.artifacts_dir.join("keyframe-index.json"));
    if let (Some(index), Some(path)) = (&index, &index_path) {
        fs::write(path, serde_json::to_vec_pretty(index)?)?;
    }

    let mut locations = vec![AdapterSourceLocation {
        source_id: request.source_id.clone(),
        path: request.original_path.clone(),
        sha256: request.original_sha256.clone(),
        byte_length: request.original_byte_length,
    }];
    let still_files = files_below(&stills_dir)?;
    for source in batch
        .sources
        .iter()
        .filter(|source| source.id != request.source_id)
    {
        let path = still_files
            .iter()
            .find(|path| {
                file_identity(path).is_ok_and(|(hash, length)| {
                    hash.eq_ignore_ascii_case(&source.sha256) && length == source.byte_length
                })
            })
            .ok_or_else(|| format!("retained still for source `{}` was not found", source.id))?;
        locations.push(AdapterSourceLocation {
            source_id: source.id.clone(),
            path: path.clone(),
            sha256: source.sha256.clone(),
            byte_length: source.byte_length,
        });
    }
    let mut artifacts = Vec::new();
    for path in still_files {
        let (hash, _) = file_identity(&path)?;
        artifacts.push(AdapterArtifact {
            kind: "still".to_owned(),
            path,
            sha256: Some(hash),
        });
    }
    let (batch_hash, _) = file_identity(&batch_path)?;
    artifacts.push(AdapterArtifact {
        kind: "normalized_batch".to_owned(),
        path: batch_path.clone(),
        sha256: Some(batch_hash),
    });
    if let Some(path) = &index_path {
        let (hash, _) = file_identity(path)?;
        artifacts.push(AdapterArtifact {
            kind: "keyframe_index".to_owned(),
            path: path.clone(),
            sha256: Some(hash),
        });
    }
    AdapterResultManifest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        batch_path,
        keyframe_index_path: index_path,
        source_locations: locations,
        artifacts,
    }
    .write_atomic(&request.artifacts_dir.join("result.json"))?;
    emit_job(
        &request,
        "completed",
        AdapterEventKind::Completed,
        "Video result is ready to import",
    )?;
    Ok(())
}

fn emit_job(
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

fn files_below(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn temporal_job(value: TemporalRelationArg) -> TemporalRelation {
    match value {
        TemporalRelationArg::Contemporaneous => TemporalRelation::Contemporaneous,
        TemporalRelationArg::AfterEvent => TemporalRelation::AfterEvent,
        TemporalRelationArg::Mixed => TemporalRelation::Mixed,
        TemporalRelationArg::Unknown => TemporalRelation::Unknown,
    }
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

fn require_revision(
    revision: Option<String>,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    revision
        .ok_or_else(|| format!("{flag} is required when its live TensorRT stage is enabled").into())
}

fn write_batch(path: &PathBuf, batch: &NormalizedBatch) -> Result<(), Box<dyn std::error::Error>> {
    write_json(path, batch)
}

fn write_json(path: &PathBuf, value: &impl Serialize) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(value)?;
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
