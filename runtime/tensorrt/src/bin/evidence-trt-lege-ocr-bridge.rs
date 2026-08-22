//! Persistent Lege page-worker bridge to the Evidence TensorRT broker.

use std::io::{self, BufRead, Read, Write};

use clap::Parser;
use evidence_trt::{Client, InputMetadata, Operation, Request, ResultBody};
use image::{DynamicImage, GrayImage, ImageFormat};
use serde::Serialize;

const LEGE_PROTOCOL: &str = "lege-tensorrt-ocr";
const LEGE_VERSION: u32 = 1;

#[derive(Debug, Parser)]
struct Cli {
    #[arg(long)]
    server: bool,
    #[arg(long, default_value = "evidence-trt")]
    endpoint: String,
    #[arg(long)]
    model: String,
    #[arg(long)]
    revision: String,
}

#[derive(Serialize)]
struct BridgeReady<'a> {
    protocol: &'a str,
    version: u32,
    ready: bool,
    gpu: String,
}

#[derive(Serialize)]
struct Reply<'a> {
    protocol: &'a str,
    version: u32,
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    lines: Vec<Line>,
}

#[derive(Serialize)]
struct Line {
    text: String,
    confidence: f32,
    bbox: [i32; 4],
}

fn main() {
    if let Err(error) = run() {
        eprintln!("evidence-trt-lege-ocr-bridge: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if !cli.server {
        return Err("--server is required".into());
    }
    let mut client = Client::connect(&cli.endpoint)?;
    let models = client.request(Request::Models, &[])?;
    let ResultBody::Models { models } = models else {
        return Err("broker returned a non-model result during preflight".into());
    };
    let model = models
        .iter()
        .find(|model| model.id == cli.model)
        .ok_or_else(|| format!("broker model `{}` is not installed", cli.model))?;
    if model.revision != cli.revision {
        return Err(format!(
            "broker model `{}` revision is `{}`, expected `{}`",
            cli.model, model.revision, cli.revision
        )
        .into());
    }
    if !model.operations.contains(&Operation::PageOcr) {
        return Err(format!("broker model `{}` does not support page_ocr", cli.model).into());
    }
    let loaded = client.request(
        Request::Load {
            model: cli.model.clone(),
        },
        &[],
    )?;
    if !matches!(loaded, ResultBody::Loaded { ref model } if model == &cli.model) {
        return Err("broker returned an incompatible model-load result".into());
    }
    let health = client.request(Request::Health, &[])?;
    let ResultBody::Health(health) = health else {
        return Err("broker returned a non-health result".into());
    };
    write_json(&BridgeReady {
        protocol: LEGE_PROTOCOL,
        version: LEGE_VERSION,
        ready: health.ready,
        gpu: health.gpu,
    })?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    loop {
        let mut header = String::new();
        if input.read_line(&mut header)? == 0 {
            return Ok(());
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header == "QUIT" {
            return Ok(());
        }
        let fields = header.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 || fields[0] != "IMAGE" {
            return Err(format!("malformed Lege worker request `{header}`").into());
        }
        let id: u64 = fields[1].parse()?;
        let width: u32 = fields[2].parse()?;
        let height: u32 = fields[3].parse()?;
        let channels: u8 = fields[4].parse()?;
        let length: usize = fields[5].parse()?;
        if channels != 1 || length != width as usize * height as usize {
            write_json(&Reply {
                protocol: LEGE_PROTOCOL,
                version: LEGE_VERSION,
                id,
                ok: false,
                error: Some("bridge requires tightly packed grayscale pixels".to_owned()),
                lines: Vec::new(),
            })?;
            continue;
        }
        let mut pixels = vec![0_u8; length];
        input.read_exact(&mut pixels)?;
        let Some(gray) = GrayImage::from_raw(width, height, pixels) else {
            return Err("invalid grayscale raster dimensions".into());
        };
        let mut png = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(gray).write_to(&mut png, ImageFormat::Png)?;
        match client.request(
            Request::Infer {
                model: cli.model.clone(),
                revision: cli.revision.clone(),
                operation: Operation::PageOcr,
                input: InputMetadata {
                    media_type: Some("image/png".to_owned()),
                    width: Some(width),
                    height: Some(height),
                    channels: Some(1),
                    language: Some("eng".to_owned()),
                    ..InputMetadata::default()
                },
            },
            png.get_ref(),
        ) {
            Ok(ResultBody::PageOcr { lines }) => write_json(&Reply {
                protocol: LEGE_PROTOCOL,
                version: LEGE_VERSION,
                id,
                ok: true,
                error: None,
                lines: lines
                    .into_iter()
                    .map(|line| {
                        let [x, y, width, height] = line.bounding_box;
                        Line {
                            text: line.text,
                            confidence: line.confidence.unwrap_or(0.0),
                            bbox: [
                                signed_coordinate(x),
                                signed_coordinate(y),
                                signed_coordinate(x.saturating_add(width)),
                                signed_coordinate(y.saturating_add(height)),
                            ],
                        }
                    })
                    .collect(),
            })?,
            Ok(_) => write_json(&Reply {
                protocol: LEGE_PROTOCOL,
                version: LEGE_VERSION,
                id,
                ok: false,
                error: Some("broker returned a non-OCR result".to_owned()),
                lines: Vec::new(),
            })?,
            Err(error) => write_json(&Reply {
                protocol: LEGE_PROTOCOL,
                version: LEGE_VERSION,
                id,
                ok: false,
                error: Some(error.to_string()),
                lines: Vec::new(),
            })?,
        }
    }
}

fn signed_coordinate(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn write_json(value: &impl Serialize) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
