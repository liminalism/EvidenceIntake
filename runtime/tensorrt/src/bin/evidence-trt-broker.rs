//! `TensorRT` broker service and installation doctor.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use evidence_trt::{Broker, Request, RequestEnvelope, Response, ResultBody};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the current user's local inference endpoint.
    Serve {
        /// Namespaced local endpoint; a named pipe on Windows.
        #[arg(long, default_value = "evidence-trt")]
        endpoint: String,
        /// Directory whose children are checksum-pinned model packs.
        #[arg(long)]
        models: PathBuf,
        /// Maximum resident model bytes.
        #[arg(long)]
        vram_budget: u64,
        /// GPU name reported by the packaged native probe.
        #[arg(long)]
        gpu: String,
        /// Packaged `TensorRT` version.
        #[arg(long)]
        tensorrt_version: String,
        /// Packaged `TensorRT-LLM` version.
        #[arg(long)]
        tensorrt_llm_version: Option<String>,
    },
    /// Verify every pack and execute its worker load/inference probe.
    Doctor {
        /// Directory whose children are checksum-pinned model packs.
        #[arg(long)]
        models: PathBuf,
        /// Maximum resident model bytes.
        #[arg(long)]
        vram_budget: u64,
        /// GPU name reported by the packaged native probe.
        #[arg(long)]
        gpu: String,
        /// Packaged `TensorRT` version.
        #[arg(long)]
        tensorrt_version: String,
        /// Packaged `TensorRT-LLM` version.
        #[arg(long)]
        tensorrt_llm_version: Option<String>,
    },
    /// Internal framed-protocol fixture used by broker integration tests.
    #[command(hide = true)]
    ProtocolFixture,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("evidence-trt-broker: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Serve {
            endpoint,
            models,
            vram_budget,
            gpu,
            tensorrt_version,
            tensorrt_llm_version,
        } => evidence_trt::server::serve(
            &endpoint,
            broker(
                &models,
                vram_budget,
                gpu,
                tensorrt_version,
                tensorrt_llm_version,
            )?,
        )?,
        Command::Doctor {
            models,
            vram_budget,
            gpu,
            tensorrt_version,
            tensorrt_llm_version,
        } => {
            let mut broker = broker(
                &models,
                vram_budget,
                gpu,
                tensorrt_version,
                tensorrt_llm_version,
            )?;
            let models = broker.handle(&RequestEnvelope::new(1, Request::Models), &[]);
            let Response::Ok {
                result: ResultBody::Models { models },
            } = models.response
            else {
                return Err("could not list verified model packs".into());
            };
            for (offset, model) in models.iter().enumerate() {
                let response = broker.handle(
                    &RequestEnvelope::new(
                        u64::try_from(offset)? + 2,
                        Request::Load {
                            model: model.id.clone(),
                        },
                    ),
                    &[],
                );
                if !matches!(response.response, Response::Ok { .. }) {
                    return Err(format!("model {} failed its worker probe", model.id).into());
                }
            }
            println!("verified and probed {} model pack(s)", models.len());
        }
        Command::ProtocolFixture => protocol_fixture()?,
    }
    Ok(())
}

fn protocol_fixture() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufReader, BufWriter};

    use evidence_trt::frame::{read_frame, write_frame};
    use evidence_trt::{OcrLine, Operation, ResponseEnvelope};

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    while let Some((request, _payload)) = read_frame::<_, RequestEnvelope>(&mut reader)? {
        let response = match request.request {
            Request::Load { model } => Response::Ok {
                result: ResultBody::Loaded { model },
            },
            Request::Infer {
                operation: Operation::EmbedText,
                ..
            } => Response::Ok {
                result: ResultBody::Embedding {
                    vector: vec![1.0, 1.0],
                },
            },
            Request::Infer {
                operation: Operation::PageOcr,
                ..
            } => Response::Ok {
                result: ResultBody::PageOcr {
                    lines: vec![OcrLine {
                        text: "fixture text".to_owned(),
                        confidence: Some(0.9),
                        bounding_box: [1, 2, 3, 4],
                    }],
                },
            },
            _ => Response::Error {
                code: "unsupported".to_owned(),
                message: "fixture does not implement request".to_owned(),
            },
        };
        write_frame(
            &mut writer,
            &ResponseEnvelope::new(request.id, response),
            &[],
        )?;
    }
    Ok(())
}

fn broker(
    models: &std::path::Path,
    vram_budget: u64,
    gpu: String,
    tensorrt_version: String,
    tensorrt_llm_version: Option<String>,
) -> evidence_trt::Result<Broker> {
    Broker::discover(
        models,
        vram_budget,
        gpu,
        tensorrt_version,
        tensorrt_llm_version,
    )
}
