#![allow(missing_docs)]

use std::fs;
use std::io::Cursor;

use evidence_trt::{
    Broker, InputMetadata, Operation, Request, RequestEnvelope, Response, ResultBody,
};
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn broker_manages_the_persistent_turboocr_protocol_bridge() {
    let temporary = tempfile::tempdir().unwrap();
    let pack = temporary.path().join("turbo-ocr");
    fs::create_dir(&pack).unwrap();
    let worker_name = if cfg!(windows) {
        "worker.exe"
    } else {
        "worker"
    };
    fs::copy(
        env!("CARGO_BIN_EXE_evidence-trt-ocr-worker"),
        pack.join(worker_name),
    )
    .unwrap();
    for (path, bytes) in [
        ("det.onnx", b"detector".as_slice()),
        ("rec.onnx", b"recognizer".as_slice()),
        ("dict.txt", b"dictionary".as_slice()),
    ] {
        fs::write(pack.join(path), bytes).unwrap();
    }
    let artifacts = [worker_name, "det.onnx", "rec.onnx", "dict.txt"]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(pack.join(path)).unwrap();
            json!({
                "role": if path == worker_name { "worker" } else { "model" },
                "path": path,
                "sha256": format!("{:x}", Sha256::digest(bytes))
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "turbo-ocr",
            "revision": "fixture-export",
            "provider": "PaddlePaddle/TurboOCR",
            "license": "Apache-2.0",
            "runtime": "tensor_rt",
            "worker": worker_name,
            "worker_args": [
                "serve",
                "--turbo-worker", worker_name,
                "--turbo-arg", "turbo-fixture",
                "--detector", "det.onnx",
                "--recognizer", "rec.onnx",
                "--dictionary", "dict.txt"
            ],
            "operations": ["page_ocr"],
            "estimated_vram_bytes": 1024,
            "artifacts": artifacts
        }))
        .unwrap(),
    )
    .unwrap();

    let image = GrayImage::from_pixel(8, 8, Luma([255]));
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let mut broker = Broker::discover(temporary.path(), 2048, "fixture-gpu", "11", None).unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "turbo-ocr".to_owned(),
                revision: "fixture-export".to_owned(),
                operation: Operation::PageOcr,
                input: InputMetadata {
                    media_type: Some("image/png".to_owned()),
                    width: Some(8),
                    height: Some(8),
                    channels: Some(1),
                    language: Some("eng".to_owned()),
                    prompt: None,
                    sample_rate: None,
                },
            },
        ),
        png.get_ref(),
    );
    let Response::Ok {
        result: ResultBody::PageOcr { lines },
    } = response.response
    else {
        panic!("unexpected OCR response: {response:?}");
    };
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "fixture text");
    assert_eq!(lines[0].confidence, Some(0.75));
    assert_eq!(lines[0].bounding_box, [1, 2, 6, 4]);
}

#[test]
#[ignore = "requires a native TurboOCR TensorRT build, models, and an NVIDIA GPU"]
fn broker_runs_real_turboocr_cuda_inference() {
    let native_worker = std::env::var_os("EVIDENCE_TURBOOCR_WORKER")
        .map(std::path::PathBuf::from)
        .expect("set EVIDENCE_TURBOOCR_WORKER to turboocr-text");
    let model_root = std::env::var_os("EVIDENCE_TURBOOCR_MODELS")
        .map(std::path::PathBuf::from)
        .expect("set EVIDENCE_TURBOOCR_MODELS to the TurboOCR models directory");
    let temporary = tempfile::tempdir().unwrap();
    let pack = temporary.path().join("turbo-ocr-real");
    fs::create_dir(&pack).unwrap();
    let bridge_name = if cfg!(windows) {
        "evidence-trt-ocr-worker.exe"
    } else {
        "evidence-trt-ocr-worker"
    };
    let native_name = if cfg!(windows) {
        "turboocr-text.exe"
    } else {
        "turboocr-text"
    };
    fs::copy(
        env!("CARGO_BIN_EXE_evidence-trt-ocr-worker"),
        pack.join(bridge_name),
    )
    .unwrap();
    fs::copy(native_worker, pack.join(native_name)).unwrap();
    for name in ["det_tiny.onnx", "rec_tiny.onnx", "keys_tiny.txt"] {
        fs::copy(model_root.join(name), pack.join(name)).unwrap();
    }
    let artifacts = [
        bridge_name,
        native_name,
        "det_tiny.onnx",
        "rec_tiny.onnx",
        "keys_tiny.txt",
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(pack.join(path)).unwrap();
        json!({
            "role": if path == bridge_name { "worker" } else { "runtime" },
            "path": path,
            "sha256": format!("{:x}", Sha256::digest(bytes))
        })
    })
    .collect::<Vec<_>>();
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "turbo-ocr-real",
            "revision": "real-cuda-probe",
            "provider": "Lege/TurboOCR",
            "license": "MIT",
            "runtime": "tensor_rt",
            "worker": bridge_name,
            "worker_args": [
                "serve",
                "--turbo-worker", native_name,
                "--detector", "det_tiny.onnx",
                "--recognizer", "rec_tiny.onnx",
                "--dictionary", "keys_tiny.txt"
            ],
            "operations": ["page_ocr"],
            "estimated_vram_bytes": 2_147_483_648_u64,
            "artifacts": artifacts
        }))
        .unwrap(),
    )
    .unwrap();

    let image = GrayImage::from_pixel(64, 64, Luma([255]));
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let mut broker = Broker::discover(
        temporary.path(),
        3 * 1024 * 1024 * 1024,
        "real-nvidia-gpu",
        "11",
        None,
    )
    .unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "turbo-ocr-real".to_owned(),
                revision: "real-cuda-probe".to_owned(),
                operation: Operation::PageOcr,
                input: InputMetadata {
                    media_type: Some("image/png".to_owned()),
                    width: Some(64),
                    height: Some(64),
                    channels: Some(1),
                    language: Some("eng".to_owned()),
                    prompt: None,
                    sample_rate: None,
                },
            },
        ),
        png.get_ref(),
    );
    assert!(
        matches!(
            response.response,
            Response::Ok {
                result: ResultBody::PageOcr { .. }
            }
        ),
        "unexpected real OCR response: {response:?}"
    );
}
