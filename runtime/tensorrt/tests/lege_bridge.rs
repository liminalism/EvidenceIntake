#![allow(missing_docs)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use evidence_trt::{Broker, Client};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn install_page_ocr_pack(root: &Path) {
    let pack = root.join("turbo-ocr");
    fs::create_dir_all(&pack).unwrap();
    fs::write(pack.join("model.bin"), b"fixture-model").unwrap();
    let model_hash = format!("{:x}", Sha256::digest(b"fixture-model"));
    let worker_name = if cfg!(windows) {
        "worker.exe"
    } else {
        "worker"
    };
    fs::copy(
        env!("CARGO_BIN_EXE_evidence-trt-broker"),
        pack.join(worker_name),
    )
    .unwrap();
    let worker_hash = format!(
        "{:x}",
        Sha256::digest(fs::read(pack.join(worker_name)).unwrap())
    );
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "turbo-ocr",
            "revision": "fixture-revision",
            "provider": "test",
            "license": "Apache-2.0",
            "runtime": "tensor_rt",
            "worker": worker_name,
            "worker_args": ["protocol-fixture"],
            "operations": ["page_ocr"],
            "estimated_vram_bytes": 128,
            "artifacts": [
                {"role":"worker", "path":worker_name, "sha256":worker_hash},
                {"role":"model", "path":"model.bin", "sha256":model_hash}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn bridge_forwards_a_lege_page_to_the_versioned_broker_model() {
    let temporary = tempfile::tempdir().unwrap();
    install_page_ocr_pack(temporary.path());
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let endpoint = format!("evidence-trt-lege-{}-{nonce}", std::process::id());
    let broker = Broker::discover(temporary.path(), 1024, "fixture-gpu", "10", None).unwrap();
    let serving_endpoint = endpoint.clone();
    std::thread::spawn(move || {
        let _ = evidence_trt::server::serve(&serving_endpoint, broker);
    });
    (0..100)
        .find_map(|_| {
            let client = Client::connect(&endpoint).ok();
            if client.is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
            client
        })
        .expect("broker endpoint became available");

    let mut bridge = Command::new(env!("CARGO_BIN_EXE_evidence-trt-lege-ocr-bridge"))
        .arg("--server")
        .arg("--endpoint")
        .arg(&endpoint)
        .arg("--model")
        .arg("turbo-ocr")
        .arg("--revision")
        .arg("fixture-revision")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = bridge.stdin.take().unwrap();
    let mut output = BufReader::new(bridge.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["protocol"], "lege-tensorrt-ocr");
    assert_eq!(ready["version"], 1);
    assert_eq!(ready["ready"], true);

    input.write_all(b"IMAGE\t7\t2\t2\t1\t4\n").unwrap();
    input.write_all(&[0, 64, 128, 255]).unwrap();
    input.flush().unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 7);
    assert_eq!(response["ok"], true);
    assert_eq!(response["lines"][0]["text"], "fixture text");
    assert_eq!(response["lines"][0]["bbox"], json!([1, 2, 4, 6]));

    input.write_all(b"QUIT\n").unwrap();
    input.flush().unwrap();
    drop(input);
    assert!(bridge.wait().unwrap().success());
}
