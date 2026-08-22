#![allow(missing_docs)]

use std::fs;

use evidence_trt::{ModelManifest, Operation};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn verifies_every_artifact_and_rejects_tampering() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("worker"), b"worker").unwrap();
    fs::write(temporary.path().join("model.onnx"), b"model").unwrap();
    let worker_hash = format!("{:x}", Sha256::digest(b"worker"));
    let model_hash = format!("{:x}", Sha256::digest(b"model"));
    fs::write(
        temporary.path().join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "siglip-test",
            "revision": "abc123",
            "provider": "test",
            "license": "Apache-2.0",
            "runtime": "tensor_rt",
            "worker": "worker",
            "operations": ["embed_image", "embed_text"],
            "estimated_vram_bytes": 1024,
            "artifacts": [
                {"role":"worker", "path":"worker", "sha256":worker_hash},
                {"role":"model", "path":"model.onnx", "sha256":model_hash}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = ModelManifest::read(temporary.path()).unwrap();
    assert_eq!(
        manifest.operations,
        vec![Operation::EmbedImage, Operation::EmbedText]
    );
    fs::write(temporary.path().join("model.onnx"), b"tampered").unwrap();
    assert!(ModelManifest::read(temporary.path()).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_a_pack_symlink_that_resolves_outside_the_pack() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let pack = parent.path().join("pack");
    fs::create_dir(&pack).unwrap();
    let outside = parent.path().join("outside-worker");
    fs::write(&outside, b"worker").unwrap();
    symlink(&outside, pack.join("worker")).unwrap();
    let hash = format!("{:x}", Sha256::digest(b"worker"));
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "escape-test",
            "revision": "abc123",
            "provider": "test",
            "license": "Apache-2.0",
            "runtime": "tensor_rt",
            "worker": "worker",
            "operations": ["embed_text"],
            "estimated_vram_bytes": 1024,
            "artifacts": [{"role":"worker", "path":"worker", "sha256":hash}]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(ModelManifest::read(&pack).is_err());
}
