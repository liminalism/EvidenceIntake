#![allow(missing_docs)]

use std::fs;
use std::path::Path;

use evidence_trt::{
    Broker, InputMetadata, Operation, Request, RequestEnvelope, Response, ResultBody,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn install_pack(root: &Path, id: &str, vram: u64) {
    let pack = root.join(id);
    fs::create_dir_all(&pack).unwrap();
    fs::write(pack.join("model.bin"), id.as_bytes()).unwrap();
    let hash = format!("{:x}", Sha256::digest(id.as_bytes()));
    let executable = env!("CARGO_BIN_EXE_evidence-trt-broker");
    let worker_name = if cfg!(windows) {
        "worker.exe"
    } else {
        "worker"
    };
    fs::copy(executable, pack.join(worker_name)).unwrap();
    let worker_hash = format!(
        "{:x}",
        Sha256::digest(fs::read(pack.join(worker_name)).unwrap())
    );
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": id,
            "revision": "test-revision",
            "provider": "test",
            "license": "Apache-2.0",
            "runtime": "tensor_rt",
            "worker": worker_name,
            "worker_args": ["protocol-fixture"],
            "operations": ["embed_text"],
            "estimated_vram_bytes": vram,
            "artifacts": [
                {"role":"worker", "path":worker_name, "sha256":worker_hash},
                {"role":"model", "path":"model.bin", "sha256":hash}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn broker_loads_workers_evicts_lru_and_forwards_typed_results() {
    let temporary = tempfile::tempdir().unwrap();
    install_pack(temporary.path(), "model-a", 700);
    install_pack(temporary.path(), "model-b", 700);
    let mut broker = Broker::discover(temporary.path(), 1_000, "test-gpu", "10", None).unwrap();

    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "model-a".to_owned(),
                revision: "test-revision".to_owned(),
                operation: Operation::EmbedText,
                input: InputMetadata::default(),
            },
        ),
        b"first",
    );
    assert!(
        matches!(
            response.response,
            Response::Ok {
                result: ResultBody::Embedding { .. }
            }
        ),
        "unexpected broker response: {response:?}"
    );

    let response = broker.handle(
        &RequestEnvelope::new(
            2,
            Request::Infer {
                model: "model-b".to_owned(),
                revision: "test-revision".to_owned(),
                operation: Operation::EmbedText,
                input: InputMetadata::default(),
            },
        ),
        b"second",
    );
    assert!(matches!(response.response, Response::Ok { .. }));

    let models = broker.handle(&RequestEnvelope::new(3, Request::Models), &[]);
    let Response::Ok {
        result: ResultBody::Models { models },
    } = models.response
    else {
        panic!("models request failed");
    };
    assert!(!models.iter().find(|m| m.id == "model-a").unwrap().resident);
    assert!(models.iter().find(|m| m.id == "model-b").unwrap().resident);
}

#[test]
fn unsupported_operation_fails_without_starting_or_falling_back() {
    let temporary = tempfile::tempdir().unwrap();
    install_pack(temporary.path(), "embedder", 100);
    let mut broker = Broker::discover(temporary.path(), 1_000, "test-gpu", "10", None).unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "embedder".to_owned(),
                revision: "test-revision".to_owned(),
                operation: Operation::CaptionImage,
                input: InputMetadata::default(),
            },
        ),
        b"image",
    );
    assert!(matches!(
        response.response,
        Response::Error { ref code, .. } if code == "unsupported_operation"
    ));
}

#[test]
fn requested_revision_must_match_the_verified_pack() {
    let temporary = tempfile::tempdir().unwrap();
    install_pack(temporary.path(), "embedder", 100);
    let mut broker = Broker::discover(temporary.path(), 1_000, "test-gpu", "10", None).unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "embedder".to_owned(),
                revision: "another-export".to_owned(),
                operation: Operation::EmbedText,
                input: InputMetadata::default(),
            },
        ),
        b"query",
    );
    assert!(matches!(
        response.response,
        Response::Error { ref code, .. } if code == "revision_mismatch"
    ));
}
