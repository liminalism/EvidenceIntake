//! Embedding a still writes a vector against the derived still, not a finding.

#![allow(missing_docs)]

use std::path::PathBuf;
use std::process::Command;

use evidence_intake::CaseId;
use evidence_intake::TemporalRelation;
use evidence_video::{
    CliEmbeddingBackend, EXTRACTOR_EMBED, EmbeddingBackend, EmbeddingDocument,
    JsonEmbeddingBackend, Keyframe, QueryEmbedding, Scene, SceneAnalysis, StillEmbedding,
    VideoIdentity, embed_from_document, embed_keyframes,
};

fn identity(case_id: CaseId) -> VideoIdentity {
    VideoIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: "cam".to_owned(),
        logical_name: "clip.mp4".to_owned(),
        media_type: "video/mp4".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: "50".repeat(32),
        byte_length: 4_096,
    }
}

fn scenes() -> SceneAnalysis {
    SceneAnalysis {
        duration_ms: 20_000,
        last_video_pts_ms: Some(20_000),
        scenes: vec![
            Scene {
                index: 1,
                start_ms: 0,
                end_ms: 10_000,
                cut_score_millis: None,
                keyframe: Some(Keyframe {
                    path: PathBuf::from("scene-1.jpg"),
                    sha256: "01".repeat(32),
                    byte_length: 1_024,
                    width: Some(1920),
                    height: Some(1080),
                }),
            },
            Scene {
                index: 2,
                start_ms: 10_000,
                end_ms: 20_000,
                cut_score_millis: None,
                keyframe: Some(Keyframe {
                    path: PathBuf::from("scene-2.jpg"),
                    sha256: "02".repeat(32),
                    byte_length: 1_024,
                    width: Some(1920),
                    height: Some(1080),
                }),
            },
        ],
        dropout: None,
    }
}

fn document() -> EmbeddingDocument {
    EmbeddingDocument {
        model: Some("test-clip".to_owned()),
        extractor: Some(EXTRACTOR_EMBED.to_owned()),
        version: Some("from-json".to_owned()),
        stills: vec![
            StillEmbedding {
                scene_index: Some(1),
                path: Some("scene-1.jpg".to_owned()),
                source_id: None,
                vector: vec![1.0, 0.0],
            },
            StillEmbedding {
                scene_index: Some(2),
                path: Some("scene-2.jpg".to_owned()),
                source_id: None,
                vector: vec![0.0, 1.0],
            },
        ],
        queries: vec![QueryEmbedding {
            text: "handcuffs".to_owned(),
            vector: vec![1.0, 0.0],
        }],
    }
}

/// The vectors are addressed to the same still source ids scenes_to_batch uses.
#[test]
fn embed_keyframes_names_the_still_source_ids() {
    let index = embed_from_document(
        &identity(CaseId("case-1".to_owned())),
        &scenes(),
        &document(),
    )
    .expect("embed");
    assert_eq!(index.case_id.0, "case-1");
    assert_eq!(index.embeddings.len(), 2);
    assert_eq!(index.embeddings[0].source_id, "cam-still-0001");
    assert_eq!(index.embeddings[1].source_id, "cam-still-0002");
    assert_eq!(index.embeddings[0].model, "test-clip");
    assert_eq!(index.embeddings[0].extractor, EXTRACTOR_EMBED);
    assert_eq!(index.embeddings[0].vector, vec![1.0, 0.0]);
}

/// A missing still is a hard error: recall cannot hide a gap in the index.
#[test]
fn a_still_missing_from_the_document_is_refused() {
    let mut document = document();
    document.stills.pop();
    let error = embed_from_document(&identity(CaseId("case-1".to_owned())), &scenes(), &document)
        .expect_err("missing still");
    assert!(error.to_string().contains("cam-still-0002"), "{error}");
}

/// The JSON backend embeds a query by exact text, the same space as the stills.
#[test]
fn json_backend_embeds_a_query() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("embed.json");
    std::fs::write(&path, serde_json::to_string(&document()).expect("json")).expect("write");
    let backend = JsonEmbeddingBackend { path };
    let vector = backend.embed_query("handcuffs").expect("query");
    assert_eq!(vector, vec![1.0, 0.0]);
}

/// Live embed_keyframes walks the still paths the JSON backend keys by file name.
#[test]
fn json_backend_embeds_stills_by_file_name() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("embed.json");
    std::fs::write(&path, serde_json::to_string(&document()).expect("json")).expect("write");
    let backend = JsonEmbeddingBackend { path };
    let index = embed_keyframes(&identity(CaseId("case-1".to_owned())), &scenes(), &backend)
        .expect("embed");
    assert_eq!(index.embeddings[0].vector, vec![1.0, 0.0]);
    assert_eq!(index.embeddings[1].vector, vec![0.0, 1.0]);
}

#[test]
fn live_embedding_cli_contract() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let model = workspace.join("siglip");
    let script = workspace.join("adapters/video/python/embed_cli.py");
    if !model.join("model.safetensors").is_file() || !script.is_file() {
        return;
    }
    let python = std::env::var_os("EVIDENCE_VIDEO_PYTHON")
        .map_or_else(|| PathBuf::from("python"), PathBuf::from);
    if !Command::new(&python)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let dir = tempfile::tempdir().expect("temp");
    let frame = dir.path().join("blue.jpg");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x64:d=1",
            "-frames:v",
            "1",
        ])
        .arg(&frame)
        .status()
        .expect("ffmpeg frame");
    if !status.success() {
        return;
    }
    let backend = CliEmbeddingBackend {
        bin: script,
        python: Some(python),
        model_dir: Some(model),
        model: "google/siglip2-base-patch16-384".to_owned(),
    };
    let vectors = backend
        .embed_stills(&[frame.clone(), frame])
        .expect("batch stills");
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|vector| vector.len() == 768));
    assert!(vectors.iter().flatten().all(|value| value.is_finite()));
    assert!(
        vectors
            .iter()
            .all(|vector| (norm(vector) - 1.0).abs() < 1e-4)
    );

    let query = backend
        .embed_query("vehicle interior at night")
        .expect("text query");
    assert_eq!(query.len(), 768);
    assert!((norm(&query) - 1.0).abs() < 1e-4);
}

fn norm(vector: &[f32]) -> f32 {
    vector.iter().map(|value| value * value).sum::<f32>().sqrt()
}
