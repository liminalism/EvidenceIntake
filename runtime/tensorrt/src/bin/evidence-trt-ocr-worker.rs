//! Evidence protocol bridge for the persistent native TurboOCR TensorRT worker.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use clap::{Parser, Subcommand};
use evidence_trt::frame::{read_frame, write_frame};
use evidence_trt::{
    OcrLine, Operation, Request, RequestEnvelope, Response, ResponseEnvelope, ResultBody,
};
use serde::Deserialize;

const TURBO_PROTOCOL: &str = "lege-tensorrt-ocr";
const TURBO_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: WorkerCommand,
}

#[derive(Debug, Subcommand)]
enum WorkerCommand {
    /// Bridge the Evidence framed protocol to a resident TurboOCR process.
    Serve {
        /// Native `turboocr-text` executable, relative to the model pack.
        #[arg(long)]
        turbo_worker: PathBuf,
        /// TensorRT detector ONNX graph, relative to the model pack.
        #[arg(long)]
        detector: PathBuf,
        /// TensorRT recognizer ONNX graph, relative to the model pack.
        #[arg(long)]
        recognizer: PathBuf,
        /// Recognition dictionary, relative to the model pack.
        #[arg(long)]
        dictionary: PathBuf,
        /// Maximum recognizer batch used by TurboOCR.
        #[arg(long, default_value_t = 8)]
        recognition_batch: usize,
        /// App-local runtime directories prepended to the child PATH.
        #[arg(long)]
        runtime_dir: Vec<PathBuf>,
        /// Arguments inserted before TurboOCR's server arguments.
        #[arg(long, allow_hyphen_values = true)]
        turbo_arg: Vec<OsString>,
    },
    /// Internal TurboOCR protocol fixture used by integration tests.
    #[command(hide = true)]
    TurboFixture {
        /// Ignore the real server arguments appended by the bridge.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        ignored: Vec<OsString>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("evidence-trt-ocr-worker: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        WorkerCommand::Serve {
            turbo_worker,
            detector,
            recognizer,
            dictionary,
            recognition_batch,
            runtime_dir,
            turbo_arg,
        } => serve(&BridgeConfig {
            turbo_worker,
            detector,
            recognizer,
            dictionary,
            recognition_batch,
            runtime_dirs: runtime_dir,
            turbo_args: turbo_arg,
        }),
        WorkerCommand::TurboFixture { ignored: _ } => turbo_fixture(),
    }
}

#[derive(Debug)]
struct BridgeConfig {
    turbo_worker: PathBuf,
    detector: PathBuf,
    recognizer: PathBuf,
    dictionary: PathBuf,
    recognition_batch: usize,
    runtime_dirs: Vec<PathBuf>,
    turbo_args: Vec<OsString>,
}

fn serve(config: &BridgeConfig) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=32).contains(&config.recognition_batch) {
        return Err("recognition batch must be in 1..=32".into());
    }
    let mut turbo = TurboOcr::start(config)?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    while let Some((envelope, payload)) = read_frame::<_, RequestEnvelope>(&mut reader)? {
        let response = dispatch(&mut turbo, &envelope, &payload);
        write_frame(
            &mut writer,
            &ResponseEnvelope::new(envelope.id, response),
            &[],
        )?;
    }
    Ok(())
}

fn dispatch(turbo: &mut TurboOcr, envelope: &RequestEnvelope, payload: &[u8]) -> Response {
    if let Err(error) = envelope.validate() {
        return failure("incompatible_protocol", error);
    }
    match &envelope.request {
        Request::Load { model } => Response::Ok {
            result: ResultBody::Loaded {
                model: model.clone(),
            },
        },
        Request::Unload { model } => Response::Ok {
            result: ResultBody::Unloaded {
                model: model.clone(),
            },
        },
        Request::Infer {
            operation: Operation::PageOcr,
            input,
            ..
        } => {
            let media_type = input.media_type.as_deref().unwrap_or_default();
            if !matches!(media_type, "image/png" | "image/jpeg") {
                return failure(
                    "invalid_input",
                    format!("page OCR requires image/png or image/jpeg, got `{media_type}`"),
                );
            }
            let image = match image::load_from_memory(payload) {
                Ok(image) => image.to_luma8(),
                Err(error) => return failure("invalid_input", error),
            };
            if input.width.is_some_and(|width| width != image.width())
                || input.height.is_some_and(|height| height != image.height())
            {
                return failure(
                    "invalid_input",
                    "declared image dimensions do not match the encoded payload",
                );
            }
            match turbo.recognize(image.width(), image.height(), image.as_raw()) {
                Ok(lines) => Response::Ok {
                    result: ResultBody::PageOcr { lines },
                },
                Err(error) => failure("inference_failure", error),
            }
        }
        Request::Infer { operation, .. } => failure(
            "unsupported_operation",
            format!("OCR worker does not implement {operation}"),
        ),
        Request::Health | Request::Models => failure(
            "unsupported_operation",
            "worker control requests are handled by the broker",
        ),
    }
}

fn failure(code: &str, error: impl std::fmt::Display) -> Response {
    Response::Error {
        code: code.to_owned(),
        message: error.to_string(),
    }
}

struct TurboOcr {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    failed: bool,
}

impl TurboOcr {
    fn start(config: &BridgeConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let pack_root = std::env::current_dir()?.canonicalize()?;
        let worker = resolve_pack_path(&pack_root, &config.turbo_worker)?;
        let detector = resolve_pack_path(&pack_root, &config.detector)?;
        let recognizer = resolve_pack_path(&pack_root, &config.recognizer)?;
        let dictionary = resolve_pack_path(&pack_root, &config.dictionary)?;
        let mut command = Command::new(worker);
        command
            .args(&config.turbo_args)
            .arg("--server")
            .arg("--det")
            .arg(detector)
            .arg("--rec")
            .arg(recognizer)
            .arg("--dict")
            .arg(dictionary)
            .arg("--rec-batch")
            .arg(config.recognition_batch.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if !config.runtime_dirs.is_empty() {
            command.env("PATH", augmented_path(&pack_root, &config.runtime_dirs)?);
        }
        command.env("TURBO_OCR_CUDA_GRAPHS", "0");
        command.env("TRT_OPT_LEVEL", "3");
        let mut child = command.spawn()?;
        let startup = (|| -> Result<_, Box<dyn std::error::Error>> {
            let stdin = child.stdin.take().ok_or("TurboOCR stdin was not created")?;
            let stdout = child
                .stdout
                .take()
                .ok_or("TurboOCR stdout was not created")?;
            let mut stdout = BufReader::new(stdout);
            let mut ready_line = String::new();
            if stdout.read_line(&mut ready_line)? == 0 {
                return Err("TurboOCR exited before its real inference preflight completed".into());
            }
            let ready: TurboReady = serde_json::from_str(ready_line.trim_end())?;
            if ready.protocol != TURBO_PROTOCOL
                || ready.version != TURBO_PROTOCOL_VERSION
                || !ready.ready
                || ready.gpu.trim().is_empty()
            {
                return Err("TurboOCR returned an incompatible READY response".into());
            }
            Ok((stdin, stdout))
        })();
        let (stdin, stdout) = match startup {
            Ok(streams) => streams,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };
        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            failed: false,
        })
    }

    fn recognize(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<Vec<OcrLine>, Box<dyn std::error::Error>> {
        if self.failed {
            return Err("TurboOCR is unavailable after a runtime failure".into());
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if let Err(error) = (|| -> std::io::Result<()> {
            writeln!(
                self.stdin,
                "IMAGE\t{id}\t{width}\t{height}\t1\t{}",
                pixels.len()
            )?;
            self.stdin.write_all(pixels)?;
            self.stdin.flush()
        })() {
            self.failed = true;
            return Err(error.into());
        }
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) => {
                self.failed = true;
                return Err(format!("TurboOCR exited during request {id}").into());
            }
            Ok(_) => {}
            Err(error) => {
                self.failed = true;
                return Err(error.into());
            }
        }
        let response: TurboResponse = match serde_json::from_str(line.trim_end()) {
            Ok(response) => response,
            Err(error) => {
                self.failed = true;
                return Err(error.into());
            }
        };
        if response.protocol != TURBO_PROTOCOL
            || response.version != TURBO_PROTOCOL_VERSION
            || response.id != id
        {
            self.failed = true;
            return Err(format!("TurboOCR response did not match request {id}").into());
        }
        if !response.ok {
            self.failed = true;
            return Err(response
                .error
                .unwrap_or_else(|| "unknown TurboOCR failure".to_owned())
                .into());
        }
        Ok(response
            .lines
            .iter()
            .filter_map(|line| map_line(line, width, height))
            .collect())
    }
}

impl Drop for TurboOcr {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "QUIT");
        let _ = self.stdin.flush();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn resolve_pack_path(pack_root: &Path, path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(format!(
            "OCR pack path must be relative and contained: {}",
            path.display()
        )
        .into());
    }
    let candidate = pack_root.join(path).canonicalize()?;
    if !candidate.starts_with(pack_root) {
        return Err(format!(
            "OCR pack path resolves outside its pack: {}",
            path.display()
        )
        .into());
    }
    Ok(candidate)
}

fn augmented_path(
    pack_root: &Path,
    additions: &[PathBuf],
) -> Result<OsString, Box<dyn std::error::Error>> {
    let mut paths = additions
        .iter()
        .map(|path| resolve_pack_path(pack_root, path))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    Ok(std::env::join_paths(paths)?)
}

#[derive(Debug, Deserialize)]
struct TurboReady {
    protocol: String,
    version: u32,
    ready: bool,
    gpu: String,
}

#[derive(Debug, Deserialize)]
struct TurboResponse {
    protocol: String,
    version: u32,
    id: u64,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    lines: Vec<TurboLine>,
}

#[derive(Debug, Deserialize)]
struct TurboLine {
    text: String,
    confidence: f32,
    bbox: [i32; 4],
}

fn map_line(line: &TurboLine, width: u32, height: u32) -> Option<OcrLine> {
    let max_x = i32::try_from(width).unwrap_or(i32::MAX);
    let max_y = i32::try_from(height).unwrap_or(i32::MAX);
    let x0 = u32::try_from(line.bbox[0].clamp(0, max_x)).ok()?;
    let y0 = u32::try_from(line.bbox[1].clamp(0, max_y)).ok()?;
    let x1 = u32::try_from(line.bbox[2].clamp(0, max_x)).ok()?;
    let y1 = u32::try_from(line.bbox[3].clamp(0, max_y)).ok()?;
    let text = line.text.trim();
    if text.is_empty() || x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(OcrLine {
        text: text.to_owned(),
        confidence: line
            .confidence
            .is_finite()
            .then_some(line.confidence.clamp(0.0, 1.0)),
        bounding_box: [x0, y0, x1 - x0, y1 - y0],
    })
}

fn turbo_fixture() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::json!({
            "protocol": TURBO_PROTOCOL,
            "version": TURBO_PROTOCOL_VERSION,
            "ready": true,
            "gpu": "fixture-gpu"
        })
    );
    std::io::stdout().flush()?;
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end() == "QUIT" {
            break;
        }
        let fields = header.trim_end().split('\t').collect::<Vec<_>>();
        if fields.len() != 6 || fields[0] != "IMAGE" {
            return Err("fixture received an invalid IMAGE header".into());
        }
        let id: u64 = fields[1].parse()?;
        let width: i32 = fields[2].parse()?;
        let height: i32 = fields[3].parse()?;
        let bytes: usize = fields[5].parse()?;
        let mut pixels = vec![0_u8; bytes];
        reader.read_exact(&mut pixels)?;
        println!(
            "{}",
            serde_json::json!({
                "protocol": TURBO_PROTOCOL,
                "version": TURBO_PROTOCOL_VERSION,
                "id": id,
                "ok": true,
                "lines": [{
                    "text": "fixture text",
                    "confidence": 0.75,
                    "bbox": [1, 2, width - 1, height - 2]
                }]
            })
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_pack_path;

    #[test]
    fn ocr_runtime_paths_must_stay_inside_the_pack() {
        let temporary = tempfile::tempdir().unwrap();
        let pack = temporary.path().join("pack");
        std::fs::create_dir(&pack).unwrap();
        std::fs::write(pack.join("model.onnx"), b"model").unwrap();
        let root = pack.canonicalize().unwrap();

        assert_eq!(
            resolve_pack_path(&root, std::path::Path::new("model.onnx")).unwrap(),
            root.join("model.onnx")
        );
        assert!(resolve_pack_path(&root, &root.join("model.onnx")).is_err());
        assert!(resolve_pack_path(&root, std::path::Path::new("../model.onnx")).is_err());
    }
}
