//! Evidence protocol worker for the Phonon-2 speech engine.
//!
//! Phonon-2 (`FermionResearch/Phonon-2`, a five-value quantization of NVIDIA's
//! `parakeet-tdt-0.6b-v3`) is the default `transcribe_audio` pack. It runs on the
//! CPU through the `fermion` command line, so this worker never touches
//! TensorRT: it bridges the framed `evidence.trt` protocol to one
//! `fermion transcribe <model> <wav> --json` invocation per request and maps
//! the word-timed JSON into typed [`TranscriptSegment`]s. The engine launcher,
//! the model directory and the worker itself are checksummed pack artifacts;
//! the engine is pointed at a pack-local cache and forbidden from reaching the
//! network, so a pack transcribes with exactly the bytes its manifest pins.
//!
//! Phonon-2 is English-only and performs no diarization. A non-English
//! language hint fails closed instead of producing a transcript the model
//! cannot stand behind, and every returned segment carries `speaker: None`;
//! speaker hypotheses come from the audio adapter's deterministic channel
//! analysis, never from this worker.

use std::ffi::OsString;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{Parser, Subcommand};
use evidence_trt::frame::{read_frame, write_frame};
use evidence_trt::manifest::{Artifact, MANIFEST_VERSION};
use evidence_trt::{
    ModelManifest, Operation, Request, RequestEnvelope, Response, ResponseEnvelope, ResultBody,
    RuntimeKind, TranscriptSegment, TranscriptWord,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Installed pack identifier the adapters and the GUI prefer for speech.
pub const PHONON_2_MODEL_ID: &str = "phonon-2";
/// Published SHA-256 of `phonon-2.bps.tar.zst` in `FermionResearch/Phonon-2`.
pub const PHONON_2_WEIGHTS_SHA256: &str =
    "98125795b6dda72f5c6eee9ba33d19815df65dcb18b50a357bf9f73c9935309e";
/// File name of the published Phonon-2 weights archive.
pub const PHONON_2_WEIGHTS_FILE: &str = "phonon-2.bps.tar.zst";
/// Conservative resident-memory estimate: the unpacked 178 MB model plus the
/// CPU engine's working set, rounded up so admission stays honest.
const PHONON_2_RESIDENT_BYTES: u64 = 1024 * 1024 * 1024;
/// Sample rate the audio adapter sends and the engine expects.
const EXPECTED_RATE: u32 = 16_000;
/// Media type of the broker's PCM payload.
const PCM_MEDIA_TYPE: &str = "audio/x-f32le";

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: WorkerCommand,
}

#[derive(Debug, Subcommand)]
enum WorkerCommand {
    /// Bridge the Evidence framed protocol to the pack's Phonon-2 engine.
    Serve {
        /// Engine launcher accepting `transcribe MODEL AUDIO --json`, relative to the pack.
        #[arg(long)]
        engine: PathBuf,
        /// Model directory holding `config.json` and `packed_manifest.json`, relative to the pack.
        #[arg(long)]
        model: PathBuf,
        /// Pack-local directory the engine may unpack the model into.
        #[arg(long, default_value = "cache")]
        cache: PathBuf,
        /// CPU threads handed to the engine; the engine's own default when omitted.
        #[arg(long)]
        threads: Option<usize>,
        /// Arguments inserted before the engine's `transcribe` arguments.
        #[arg(long, allow_hyphen_values = true)]
        engine_arg: Vec<OsString>,
    },
    /// Assemble a checksummed Phonon-2 model pack from a Hugging Face download.
    Pack {
        /// Directory produced by `hf download FermionResearch/Phonon-2 --local-dir DIR`.
        #[arg(long)]
        weights: PathBuf,
        /// Engine launcher executable copied into the pack and checksummed.
        #[arg(long)]
        engine: PathBuf,
        /// Pack directory to create; its name should equal the model id.
        #[arg(long)]
        out: PathBuf,
        /// Expected SHA-256 of the weights archive. Defaults to the published pin.
        #[arg(long, default_value = PHONON_2_WEIGHTS_SHA256)]
        weights_sha256: String,
        /// Stable model identifier.
        #[arg(long, default_value = PHONON_2_MODEL_ID)]
        id: String,
        /// Resident-memory estimate used for admission.
        #[arg(long, default_value_t = PHONON_2_RESIDENT_BYTES)]
        estimated_bytes: u64,
        /// CPU threads recorded in the pack's worker arguments.
        #[arg(long)]
        threads: Option<usize>,
    },
    /// Internal engine fixture used by integration tests.
    #[command(hide = true)]
    PhononFixture {
        /// The `transcribe MODEL AUDIO --json` arguments the worker passes.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<OsString>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("evidence-trt-phonon-worker: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        WorkerCommand::Serve {
            engine,
            model,
            cache,
            threads,
            engine_arg,
        } => serve(&EngineConfig::resolve(
            &engine, &model, &cache, threads, engine_arg,
        )?),
        WorkerCommand::Pack {
            weights,
            engine,
            out,
            weights_sha256,
            id,
            estimated_bytes,
            threads,
        } => pack(&PackRequest {
            weights,
            engine,
            out,
            weights_sha256,
            id,
            estimated_bytes,
            threads,
        }),
        WorkerCommand::PhononFixture { arguments } => phonon_fixture(&arguments),
    }
}

#[derive(Debug)]
struct EngineConfig {
    engine: PathBuf,
    model: PathBuf,
    cache: PathBuf,
    threads: Option<usize>,
    engine_args: Vec<OsString>,
}

impl EngineConfig {
    fn resolve(
        engine: &Path,
        model: &Path,
        cache: &Path,
        threads: Option<usize>,
        engine_args: Vec<OsString>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pack_root = std::env::current_dir()?.canonicalize()?;
        let model = resolve_pack_path(&pack_root, model)?;
        for required in ["config.json", "packed_manifest.json"] {
            if !model.join(required).is_file() {
                return Err(format!(
                    "Phonon model directory {} has no {required}",
                    model.display()
                )
                .into());
            }
        }
        require_contained(cache)?;
        let cache = pack_root.join(cache);
        std::fs::create_dir_all(&cache)?;
        Ok(Self {
            engine: resolve_pack_path(&pack_root, engine)?,
            model,
            cache: cache.canonicalize()?,
            threads,
            engine_args,
        })
    }

    /// Run `transcribe` on one wav and return the engine's JSON document.
    fn transcribe(&self, wav: &Path) -> Result<PhononOutput, Box<dyn std::error::Error>> {
        let mut command = Command::new(&self.engine);
        command
            .args(&self.engine_args)
            .arg("transcribe")
            .arg(&self.model)
            .arg(wav)
            .arg("--json")
            .env("FERMION_CACHE_DIR", &self.cache)
            .env("HF_HOME", self.cache.join("hf"))
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(threads) = self.threads {
            command.arg("--threads").arg(threads.to_string());
        }
        let output = command.output().map_err(|error| {
            format!(
                "could not start Phonon engine {}: {error}",
                self.engine.display()
            )
        })?;
        if !output.status.success() {
            return Err(format!("Phonon engine failed with {}", output.status).into());
        }
        let parsed: PhononOutput = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Phonon engine returned invalid JSON: {error}"))?;
        if parsed.truncated == Some(true) {
            return Err("Phonon engine reported a truncated transcript".into());
        }
        Ok(parsed)
    }
}

fn serve(config: &EngineConfig) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    while let Some((envelope, payload)) = read_frame::<_, RequestEnvelope>(&mut reader)? {
        let response = dispatch(config, &envelope, &payload);
        write_frame(
            &mut writer,
            &ResponseEnvelope::new(envelope.id, response),
            &[],
        )?;
    }
    Ok(())
}

fn dispatch(config: &EngineConfig, envelope: &RequestEnvelope, payload: &[u8]) -> Response {
    if let Err(error) = envelope.validate() {
        return failure("incompatible_protocol", error);
    }
    match &envelope.request {
        Request::Load { model } => match probe(config) {
            Ok(()) => Response::Ok {
                result: ResultBody::Loaded {
                    model: model.clone(),
                },
            },
            Err(error) => failure("load_failure", error),
        },
        Request::Unload { model } => Response::Ok {
            result: ResultBody::Unloaded {
                model: model.clone(),
            },
        },
        Request::Infer {
            operation: Operation::TranscribeAudio,
            input,
            ..
        } => {
            let media_type = input.media_type.as_deref().unwrap_or_default();
            if media_type != PCM_MEDIA_TYPE {
                return failure(
                    "invalid_input",
                    format!("transcription requires {PCM_MEDIA_TYPE}, got `{media_type}`"),
                );
            }
            if input.channels.is_some_and(|channels| channels != 1) {
                return failure("invalid_input", "transcription requires mono PCM");
            }
            let rate = input.sample_rate.unwrap_or(EXPECTED_RATE);
            if rate != EXPECTED_RATE {
                return failure(
                    "invalid_input",
                    format!("transcription requires {EXPECTED_RATE} Hz PCM, got {rate} Hz"),
                );
            }
            if let Some(language) = input.language.as_deref()
                && !is_english(language)
            {
                return failure(
                    "unsupported_language",
                    format!("Phonon-2 transcribes English only; `{language}` was requested"),
                );
            }
            let samples = match pcm_from_payload(payload) {
                Ok(samples) => samples,
                Err(error) => return failure("invalid_input", error),
            };
            match transcribe_samples(config, rate, &samples) {
                Ok(segments) => Response::Ok {
                    result: ResultBody::Transcript { segments },
                },
                Err(error) => failure("inference_failure", error),
            }
        }
        Request::Infer { operation, .. } => failure(
            "unsupported_operation",
            format!("Phonon worker does not implement {operation}"),
        ),
        Request::Health | Request::Models => failure(
            "unsupported_operation",
            "worker control requests are handled by the broker",
        ),
    }
}

/// Real load probe: the engine must decode one second of audio end to end.
fn probe(config: &EngineConfig) -> Result<(), Box<dyn std::error::Error>> {
    let samples = vec![0.0_f32; EXPECTED_RATE as usize];
    transcribe_samples(config, EXPECTED_RATE, &samples).map(|_| ())
}

fn transcribe_samples(
    config: &EngineConfig,
    rate: u32,
    samples: &[f32],
) -> Result<Vec<TranscriptSegment>, Box<dyn std::error::Error>> {
    let wav = tempfile::Builder::new()
        .prefix("evidence-phonon-")
        .suffix(".wav")
        .tempfile()?;
    write_pcm16_wav(wav.path(), rate, samples)?;
    let output = config.transcribe(wav.path())?;
    segments_from(&output)
}

fn failure(code: &str, error: impl std::fmt::Display) -> Response {
    Response::Error {
        code: code.to_owned(),
        message: error.to_string(),
    }
}

fn is_english(language: &str) -> bool {
    let language = language.trim().to_ascii_lowercase();
    language.is_empty()
        || matches!(language.as_str(), "en" | "eng" | "english")
        || language.starts_with("en-")
        || language.starts_with("en_")
}

fn pcm_from_payload(payload: &[u8]) -> Result<Vec<f32>, String> {
    if payload.is_empty() {
        return Err("transcription payload is empty".to_owned());
    }
    if payload.len() % 4 != 0 {
        return Err("f32le payload length is not a multiple of four".to_owned());
    }
    payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| {
            let sample = f32::from_le_bytes(*bytes);
            if sample.is_finite() {
                Ok(sample)
            } else {
                Err("f32le payload contains a non-finite sample".to_owned())
            }
        })
        .collect()
}

/// Writes mono 16-bit PCM; the engine reads any wav `soundfile` reads.
fn write_pcm16_wav(path: &Path, rate: u32, samples: &[f32]) -> std::io::Result<()> {
    let data_len = u32::try_from(samples.len() * 2)
        .map_err(|_| std::io::Error::other("audio is too long for a wav container"))?;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        bytes.extend_from_slice(&scaled.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

/// `fermion transcribe --json` as documented for release 0.2.10 and later.
#[derive(Debug, Default, Deserialize)]
struct PhononOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<PhononSegment>,
    #[serde(default)]
    words: Option<Vec<PhononWord>>,
    #[serde(default)]
    truncated: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PhononSegment {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Debug, Deserialize)]
struct PhononWord {
    text: String,
    start: f64,
    end: f64,
}

/// Map engine segments and words onto the typed transcript.
///
/// Words are attached to the segment whose span contains their start. The
/// engine reports no confidence and no speaker, so neither is invented.
fn segments_from(
    output: &PhononOutput,
) -> Result<Vec<TranscriptSegment>, Box<dyn std::error::Error>> {
    let words = output.words.as_deref().unwrap_or_default();
    for word in words {
        check_span(word.start, word.end, &format!("word `{}`", word.text))?;
    }
    let mut segments = Vec::new();
    if output.segments.is_empty() {
        let text = output.text.trim();
        if text.is_empty() {
            return Ok(segments);
        }
        let (start, end) = match (words.first(), words.last()) {
            (Some(first), Some(last)) => (first.start, last.end),
            _ => return Err("Phonon engine returned text without any timing".into()),
        };
        segments.push(segment(start, end, text, words.iter()));
        return Ok(segments);
    }
    for (index, item) in output.segments.iter().enumerate() {
        check_span(item.start, item.end, &format!("segment {index}"))?;
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let limit = item.end.max(item.start + 0.001);
        segments.push(segment(
            item.start,
            item.end,
            text,
            words
                .iter()
                .filter(|word| word.start >= item.start && word.start < limit),
        ));
    }
    Ok(segments)
}

fn segment<'a>(
    start: f64,
    end: f64,
    text: &str,
    words: impl Iterator<Item = &'a PhononWord>,
) -> TranscriptSegment {
    TranscriptSegment {
        start_ms: ms(start),
        end_ms: ms(end),
        text: text.to_owned(),
        confidence: None,
        words: words
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| TranscriptWord {
                start_ms: ms(word.start),
                end_ms: ms(word.end),
                text: word.text.trim().to_owned(),
                confidence: None,
            })
            .collect(),
        speaker: None,
    }
}

fn check_span(start: f64, end: f64, what: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
        return Err(format!("Phonon engine returned an invalid time span for {what}").into());
    }
    Ok(())
}

#[allow(
    clippy::cast_sign_loss,
    reason = "spans are validated non-negative before conversion"
)]
fn ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1_000.0).round() as u64
}

fn require_contained(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(format!(
            "Phonon pack path must be relative and contained: {}",
            path.display()
        )
        .into());
    }
    Ok(())
}

fn resolve_pack_path(pack_root: &Path, path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    require_contained(path)?;
    let candidate = pack_root.join(path).canonicalize()?;
    if !candidate.starts_with(pack_root) {
        return Err(format!(
            "Phonon pack path resolves outside its pack: {}",
            path.display()
        )
        .into());
    }
    Ok(candidate)
}

#[derive(Debug)]
struct PackRequest {
    weights: PathBuf,
    engine: PathBuf,
    out: PathBuf,
    weights_sha256: String,
    id: String,
    estimated_bytes: u64,
    threads: Option<usize>,
}

/// Build `<out>/manifest.json` plus checksummed copies of the model, the
/// engine launcher and this worker, then verify the result the way the
/// broker will.
fn pack(request: &PackRequest) -> Result<(), Box<dyn std::error::Error>> {
    let archive = request.weights.join(PHONON_2_WEIGHTS_FILE);
    let archive_hash = sha256_file(&archive)?;
    if !archive_hash.eq_ignore_ascii_case(&request.weights_sha256) {
        return Err(format!(
            "{} does not match the pinned weights: expected {}, got {archive_hash}",
            archive.display(),
            request.weights_sha256
        )
        .into());
    }
    if request.out.exists() {
        return Err(format!("pack directory already exists: {}", request.out.display()).into());
    }
    let model_dir = request.out.join("model");
    let engine_dir = request.out.join("engine");
    let bin_dir = request.out.join("bin");
    std::fs::create_dir_all(&model_dir)?;
    std::fs::create_dir_all(&engine_dir)?;
    std::fs::create_dir_all(&bin_dir)?;

    let mut artifacts = Vec::new();
    let mut copy = |from: &Path, to_relative: PathBuf, role: &str| -> std::io::Result<()> {
        let destination = request.out.join(&to_relative);
        std::fs::copy(from, &destination)?;
        artifacts.push(Artifact {
            role: role.to_owned(),
            path: to_relative,
            sha256: sha256_file(&destination)?,
        });
        Ok(())
    };
    for required in [PHONON_2_WEIGHTS_FILE, "config.json", "packed_manifest.json"] {
        let source = request.weights.join(required);
        if !source.is_file() {
            return Err(format!("Phonon download is missing {}", source.display()).into());
        }
        let role = if required == PHONON_2_WEIGHTS_FILE {
            "weights"
        } else {
            "model_metadata"
        };
        copy(&source, Path::new("model").join(required), role)?;
    }
    for optional in [
        "NOTICE",
        "LICENSE-WEIGHTS-CC-BY-4.0.txt",
        "LICENSE-CODE-Apache-2.0.txt",
        "SHA256SUMS.txt",
    ] {
        let source = request.weights.join(optional);
        if source.is_file() {
            copy(&source, Path::new("model").join(optional), "licence")?;
        }
    }
    let engine_name = request
        .engine
        .file_name()
        .ok_or("engine launcher has no file name")?
        .to_owned();
    let engine_relative = Path::new("engine").join(&engine_name);
    copy(&request.engine, engine_relative.clone(), "engine")?;
    let worker_source = std::env::current_exe()?;
    let worker_name = worker_source
        .file_name()
        .ok_or("worker executable has no file name")?
        .to_owned();
    let worker_relative = Path::new("bin").join(&worker_name);
    copy(&worker_source, worker_relative.clone(), "worker")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for relative in [&engine_relative, &worker_relative] {
            let path = request.out.join(relative);
            let mut permissions = std::fs::metadata(&path)?.permissions();
            permissions.set_mode(permissions.mode() | 0o111);
            std::fs::set_permissions(&path, permissions)?;
        }
    }

    let mut worker_args = vec![
        "serve".to_owned(),
        "--engine".to_owned(),
        unix_style(&engine_relative),
        "--model".to_owned(),
        "model".to_owned(),
        "--cache".to_owned(),
        "cache".to_owned(),
    ];
    if let Some(threads) = request.threads {
        worker_args.push("--threads".to_owned());
        worker_args.push(threads.to_string());
    }
    let manifest = ModelManifest {
        schema_version: MANIFEST_VERSION,
        id: request.id.clone(),
        revision: archive_hash,
        provider: "Fermion Research".to_owned(),
        license: "CC-BY-4.0".to_owned(),
        runtime: RuntimeKind::NativeCpu,
        worker: worker_relative,
        worker_args,
        operations: vec![Operation::TranscribeAudio],
        estimated_vram_bytes: request.estimated_bytes,
        artifacts,
    };
    let manifest_path = request.out.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    let verified = ModelManifest::read(&request.out)?;
    println!(
        "packed {} revision {} with {} artifact(s) at {}",
        verified.id,
        verified.revision,
        verified.artifacts.len(),
        request.out.display()
    );
    Ok(())
}

fn unix_style(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Stand-in for `fermion transcribe MODEL AUDIO --json`.
///
/// Checks the arguments the worker is contractually required to pass, reads
/// the wav header to prove a real file arrived, and prints a fixed
/// word-timed document shaped like the documented engine output.
#[allow(
    clippy::cast_precision_loss,
    reason = "a fixture duration needs no sub-sample precision"
)]
fn phonon_fixture(arguments: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let [command, model, audio, rest @ ..] = arguments.as_slice() else {
        return Err("fixture expected `transcribe MODEL AUDIO --json`".into());
    };
    if command != "transcribe" || !rest.iter().any(|argument| argument == "--json") {
        return Err("fixture expected `transcribe MODEL AUDIO --json`".into());
    }
    if !Path::new(model).join("config.json").is_file() {
        return Err(format!("fixture model directory has no config.json: {model}").into());
    }
    if std::env::var_os("HF_HUB_OFFLINE").is_none_or(|value| value != "1") {
        return Err("fixture requires the worker to forbid network model lookups".into());
    }
    let wav = std::fs::read(audio)?;
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("fixture received something other than a wav".into());
    }
    let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    let frames = (wav.len() - 44) / 2;
    let duration = frames as f64 / f64::from(rate);
    let end = duration.min(1.0);
    println!(
        "{}",
        serde_json::json!({
            "text": "fixture speech",
            "model": "phonon-2",
            "engine": "fixture",
            "duration_seconds": duration,
            "segment_count": 1,
            "segments": [{"id": 0, "start": 0.0, "end": end, "text": "fixture speech"}],
            "words": [
                {"text": "fixture", "start": 0.0, "end": end / 2.0},
                {"text": "speech", "start": end / 2.0, "end": end}
            ],
            "truncated": false
        })
    );
    std::io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        PhononOutput, PhononSegment, PhononWord, is_english, pcm_from_payload, segments_from,
    };

    fn word(text: &str, start: f64, end: f64) -> PhononWord {
        PhononWord {
            text: text.to_owned(),
            start,
            end,
        }
    }

    #[test]
    fn words_attach_to_the_segment_that_contains_their_start() {
        let output = PhononOutput {
            text: "it just happened. no I am not hurt.".to_owned(),
            segments: vec![
                PhononSegment {
                    start: 8.2,
                    end: 9.0,
                    text: " it just happened. ".to_owned(),
                },
                PhononSegment {
                    start: 34.0,
                    end: 36.0,
                    text: "no I am not hurt.".to_owned(),
                },
            ],
            words: Some(vec![
                word("it", 8.2, 8.4),
                word("just", 8.4, 8.7),
                word("happened.", 8.7, 9.0),
                word("no", 34.0, 34.2),
            ]),
            truncated: Some(false),
        };
        let segments = segments_from(&output).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start_ms, 8_200);
        assert_eq!(segments[0].end_ms, 9_000);
        assert_eq!(segments[0].text, "it just happened.");
        assert_eq!(segments[0].words.len(), 3);
        assert_eq!(segments[0].words[2].text, "happened.");
        assert_eq!(segments[1].words.len(), 1);
        assert!(segments.iter().all(|segment| segment.speaker.is_none()));
        assert!(segments.iter().all(|segment| segment.confidence.is_none()));
    }

    #[test]
    fn text_without_segments_spans_its_words_and_silence_is_empty() {
        let output = PhononOutput {
            text: "hello there".to_owned(),
            segments: Vec::new(),
            words: Some(vec![word("hello", 1.0, 1.4), word("there", 1.5, 2.0)]),
            truncated: None,
        };
        let segments = segments_from(&output).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (1_000, 2_000));
        assert_eq!(segments_from(&PhononOutput::default()).unwrap().len(), 0);
    }

    #[test]
    fn inverted_or_negative_spans_fail_closed() {
        let output = PhononOutput {
            text: "x".to_owned(),
            segments: vec![PhononSegment {
                start: 2.0,
                end: 1.0,
                text: "x".to_owned(),
            }],
            words: None,
            truncated: None,
        };
        assert!(segments_from(&output).is_err());
    }

    #[test]
    fn only_english_hints_are_accepted() {
        assert!(is_english("en"));
        assert!(is_english("EN-us"));
        assert!(is_english("english"));
        assert!(!is_english("de"));
        assert!(!is_english("fr-CA"));
    }

    #[test]
    fn payloads_must_be_whole_finite_samples() {
        assert!(pcm_from_payload(&[]).is_err());
        assert!(pcm_from_payload(&[0, 0, 0]).is_err());
        assert!(pcm_from_payload(&f32::NAN.to_le_bytes()).is_err());
        assert_eq!(pcm_from_payload(&0.5_f32.to_le_bytes()).unwrap(), vec![0.5]);
    }
}
