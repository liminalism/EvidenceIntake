//! The Phonon-2 speech pack: a CPU worker behind the same broker discipline.

#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use evidence_trt::{
    Broker, InputMetadata, ModelManifest, Operation, Request, RequestEnvelope, Response,
    ResultBody, RuntimeKind,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const RATE: u32 = 16_000;

fn worker_name() -> &'static str {
    if cfg!(windows) {
        "evidence-trt-phonon-worker.exe"
    } else {
        "evidence-trt-phonon-worker"
    }
}

/// A Hugging Face download stand-in: the three files the engine requires plus
/// a weights archive whose bytes are whatever the test pins.
fn fake_download(root: &Path) -> PathBuf {
    let download = root.join("Phonon-2");
    fs::create_dir_all(&download).unwrap();
    fs::write(download.join("phonon-2.bps.tar.zst"), b"five-value weights").unwrap();
    fs::write(
        download.join("config.json"),
        br#"{"model_type":"parakeet_tdt_five_value"}"#,
    )
    .unwrap();
    fs::write(
        download.join("packed_manifest.json"),
        br#"{"container":"model.fermion"}"#,
    )
    .unwrap();
    fs::write(download.join("NOTICE"), b"attribution").unwrap();
    download
}

fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

/// Installs a pack by hand, with the worker binary doubling as the engine
/// through its hidden fixture subcommand.
fn install_fixture_pack(models: &Path) {
    let pack = models.join("phonon-2");
    fs::create_dir_all(pack.join("bin")).unwrap();
    fs::create_dir_all(pack.join("engine")).unwrap();
    let download = fake_download(&pack);
    fs::rename(&download, pack.join("model")).unwrap();
    let worker = Path::new("bin").join(worker_name());
    let engine = Path::new("engine").join(worker_name());
    for relative in [&worker, &engine] {
        fs::copy(
            env!("CARGO_BIN_EXE_evidence-trt-phonon-worker"),
            pack.join(relative),
        )
        .unwrap();
    }
    let artifacts = [
        (worker.clone(), "worker"),
        (engine.clone(), "engine"),
        (PathBuf::from("model/phonon-2.bps.tar.zst"), "weights"),
        (PathBuf::from("model/config.json"), "model_metadata"),
        (
            PathBuf::from("model/packed_manifest.json"),
            "model_metadata",
        ),
    ]
    .into_iter()
    .map(|(path, role)| {
        json!({
            "role": role,
            "path": path.to_string_lossy().replace('\\', "/"),
            "sha256": hash(&pack.join(&path))
        })
    })
    .collect::<Vec<_>>();
    fs::write(
        pack.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "id": "phonon-2",
            "revision": "fixture-weights",
            "provider": "Fermion Research",
            "license": "CC-BY-4.0",
            "runtime": "native_cpu",
            "worker": worker.to_string_lossy().replace('\\', "/"),
            "worker_args": [
                "serve",
                "--engine", engine.to_string_lossy().replace('\\', "/"),
                "--engine-arg", "phonon-fixture",
                "--model", "model",
                "--cache", "cache"
            ],
            "operations": ["transcribe_audio"],
            "estimated_vram_bytes": 1024,
            "artifacts": artifacts
        }))
        .unwrap(),
    )
    .unwrap();
}

fn pcm(seconds: u32) -> Vec<u8> {
    (0..RATE * seconds)
        .flat_map(|index| {
            let phase = f64::from(index) / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation, reason = "test tone")]
            let sample = (phase * 440.0 * std::f64::consts::TAU).sin() as f32 * 0.25;
            sample.to_le_bytes()
        })
        .collect()
}

fn infer(language: Option<&str>) -> Request {
    Request::Infer {
        model: "phonon-2".to_owned(),
        revision: "fixture-weights".to_owned(),
        operation: Operation::TranscribeAudio,
        input: InputMetadata {
            media_type: Some("audio/x-f32le".to_owned()),
            sample_rate: Some(RATE),
            channels: Some(1),
            language: language.map(str::to_owned),
            ..InputMetadata::default()
        },
    }
}

#[test]
fn the_phonon_pack_is_a_native_cpu_pack_the_broker_probes_and_serves() {
    let temporary = tempfile::tempdir().unwrap();
    install_fixture_pack(temporary.path());
    let manifest = ModelManifest::read(&temporary.path().join("phonon-2")).unwrap();
    assert_eq!(manifest.runtime, RuntimeKind::NativeCpu);
    assert_eq!(manifest.operations, vec![Operation::TranscribeAudio]);

    let mut broker = Broker::discover(temporary.path(), 4096, "cpu", "none", None).unwrap();
    let response = broker.handle(&RequestEnvelope::new(1, infer(Some("en"))), &pcm(2));
    let Response::Ok {
        result: ResultBody::Transcript { segments },
    } = response.response
    else {
        panic!("unexpected transcription response: {response:?}");
    };
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].text, "fixture speech");
    assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 1_000));
    assert_eq!(segments[0].words.len(), 2);
    assert_eq!(segments[0].words[1].text, "speech");
    assert_eq!(
        (segments[0].words[1].start_ms, segments[0].words[1].end_ms),
        (500, 1_000)
    );
    assert!(segments[0].speaker.is_none(), "Phonon-2 does not diarize");
    assert!(
        segments[0].confidence.is_none(),
        "no confidence is invented"
    );
    assert!(
        temporary.path().join("phonon-2").join("cache").is_dir(),
        "the engine cache stays inside the pack"
    );
}

#[test]
fn a_non_english_hint_fails_closed_instead_of_transcribing() {
    let temporary = tempfile::tempdir().unwrap();
    install_fixture_pack(temporary.path());
    let mut broker = Broker::discover(temporary.path(), 4096, "cpu", "none", None).unwrap();
    let response = broker.handle(&RequestEnvelope::new(1, infer(Some("de"))), &pcm(1));
    assert!(
        matches!(
            response.response,
            Response::Error { ref code, ref message }
                if code == "worker_failure" && message.contains("unsupported_language")
        ),
        "unexpected response: {response:?}"
    );
    let wrong_rate = Request::Infer {
        model: "phonon-2".to_owned(),
        revision: "fixture-weights".to_owned(),
        operation: Operation::TranscribeAudio,
        input: InputMetadata {
            media_type: Some("audio/x-f32le".to_owned()),
            sample_rate: Some(48_000),
            channels: Some(1),
            ..InputMetadata::default()
        },
    };
    let response = broker.handle(&RequestEnvelope::new(2, wrong_rate), &pcm(1));
    assert!(matches!(
        response.response,
        Response::Error { ref message, .. } if message.contains("invalid_input")
    ));
}

#[test]
fn the_pack_command_refuses_weights_that_do_not_match_the_pin() {
    let temporary = tempfile::tempdir().unwrap();
    let download = fake_download(temporary.path());
    let status = Command::new(env!("CARGO_BIN_EXE_evidence-trt-phonon-worker"))
        .arg("pack")
        .arg("--weights")
        .arg(&download)
        .arg("--engine")
        .arg(env!("CARGO_BIN_EXE_evidence-trt-phonon-worker"))
        .arg("--out")
        .arg(temporary.path().join("models").join("phonon-2"))
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "the published pin must reject fixture bytes"
    );
    assert!(!temporary.path().join("models").join("phonon-2").exists());
}

#[cfg(unix)]
#[test]
fn the_pack_command_builds_a_verifiable_pack_the_broker_can_run() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let download = fake_download(temporary.path());
    let launcher = temporary.path().join("fermion");
    fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nexec \"{}\" phonon-fixture \"$@\"\n",
            env!("CARGO_BIN_EXE_evidence-trt-phonon-worker")
        ),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    let models = temporary.path().join("models");
    let pack = models.join("phonon-2");
    let output = Command::new(env!("CARGO_BIN_EXE_evidence-trt-phonon-worker"))
        .arg("pack")
        .arg("--weights")
        .arg(&download)
        .arg("--engine")
        .arg(&launcher)
        .arg("--out")
        .arg(&pack)
        .arg("--weights-sha256")
        .arg(hash(&download.join("phonon-2.bps.tar.zst")))
        .arg("--threads")
        .arg("2")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = ModelManifest::read(&pack).unwrap();
    assert_eq!(manifest.id, "phonon-2");
    assert_eq!(manifest.runtime, RuntimeKind::NativeCpu);
    assert_eq!(manifest.license, "CC-BY-4.0");
    assert_eq!(
        manifest.revision,
        hash(&download.join("phonon-2.bps.tar.zst")),
        "the revision is the weights archive itself"
    );
    assert!(
        manifest
            .artifacts
            .iter()
            .any(|artifact| artifact.role == "licence")
    );
    assert!(manifest.worker_args.contains(&"--threads".to_owned()));

    let mut broker =
        Broker::discover(&models, 2 * 1024 * 1024 * 1024, "cpu", "none", None).unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "phonon-2".to_owned(),
                revision: manifest.revision.clone(),
                operation: Operation::TranscribeAudio,
                input: InputMetadata {
                    media_type: Some("audio/x-f32le".to_owned()),
                    sample_rate: Some(RATE),
                    channels: Some(1),
                    language: Some("en".to_owned()),
                    ..InputMetadata::default()
                },
            },
        ),
        &pcm(1),
    );
    assert!(
        matches!(
            response.response,
            Response::Ok {
                result: ResultBody::Transcript { .. }
            }
        ),
        "unexpected response: {response:?}"
    );

    fs::write(pack.join("model").join("config.json"), b"{}").unwrap();
    assert!(
        ModelManifest::read(&pack).is_err(),
        "an edited model file must fail verification"
    );
}

#[test]
#[ignore = "requires the fermion-research package and the real Phonon-2 download"]
fn the_real_phonon_engine_transcribes_through_the_broker() {
    let download = std::env::var_os("EVIDENCE_PHONON_DOWNLOAD")
        .map(PathBuf::from)
        .expect("set EVIDENCE_PHONON_DOWNLOAD to the FermionResearch/Phonon-2 download");
    let launcher = std::env::var_os("EVIDENCE_PHONON_ENGINE")
        .map(PathBuf::from)
        .expect("set EVIDENCE_PHONON_ENGINE to the fermion launcher");
    let temporary = tempfile::tempdir().unwrap();
    let models = temporary.path().join("models");
    let status = Command::new(env!("CARGO_BIN_EXE_evidence-trt-phonon-worker"))
        .arg("pack")
        .arg("--weights")
        .arg(&download)
        .arg("--engine")
        .arg(&launcher)
        .arg("--out")
        .arg(models.join("phonon-2"))
        .status()
        .unwrap();
    assert!(status.success());
    let manifest = ModelManifest::read(&models.join("phonon-2")).unwrap();
    let mut broker =
        Broker::discover(&models, 2 * 1024 * 1024 * 1024, "cpu", "none", None).unwrap();
    let response = broker.handle(
        &RequestEnvelope::new(
            1,
            Request::Infer {
                model: "phonon-2".to_owned(),
                revision: manifest.revision,
                operation: Operation::TranscribeAudio,
                input: InputMetadata {
                    media_type: Some("audio/x-f32le".to_owned()),
                    sample_rate: Some(RATE),
                    channels: Some(1),
                    language: Some("en".to_owned()),
                    ..InputMetadata::default()
                },
            },
        ),
        &pcm(3),
    );
    assert!(matches!(
        response.response,
        Response::Ok {
            result: ResultBody::Transcript { .. }
        }
    ));
}
