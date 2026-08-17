//! Combined analyze-job tests.

#![allow(missing_docs)]

use std::path::PathBuf;

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ReviewState, SourceKind, Store, TemporalRelation,
};
use evidence_video::{
    Detection, Keyframe, Scene, SceneAnalysis, SceneCaption, VideoIdentity, analyze_to_batch,
    scenes_to_batch,
};

fn identity(case_id: CaseId) -> VideoIdentity {
    VideoIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: "hr-src-camera".to_owned(),
        logical_name: "Oak and Third camera prosecution clip.mp4".to_owned(),
        media_type: "video/mp4".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: "50".repeat(32),
        byte_length: 12_280_334,
    }
}

fn fixture_analysis() -> SceneAnalysis {
    SceneAnalysis {
        duration_ms: 28_000,
        last_video_pts_ms: Some(28_000),
        scenes: vec![Scene {
            index: 2,
            start_ms: 9_800,
            end_ms: 15_400,
            cut_score_millis: None,
            keyframe: Some(Keyframe {
                path: PathBuf::from("scene-0002.jpg"),
                sha256: "53".repeat(32),
                byte_length: 2_048,
                width: Some(1920),
                height: Some(1080),
            }),
        }],
        dropout: None,
    }
}

fn hatchback_hit() -> Detection {
    Detection {
        scene_index: Some(2),
        start_ms: 9_800,
        end_ms: 15_400,
        label: "car".to_owned(),
        bbox: [240.0, 400.0, 480.0, 220.0],
        confidence: Some(0.81),
    }
}

fn impact_caption() -> SceneCaption {
    SceneCaption {
        scene_index: Some(2),
        start_ms: 9_800,
        end_ms: 15_400,
        text: "A light-colored sedan is in contact with the rear of a blue hatchback.".to_owned(),
        confidence: None,
    }
}

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

#[test]
fn analyze_imports_scenes_boxes_and_captions_once() {
    let (mut store, case_id) = seeded();
    let identity = identity(case_id.clone());
    let batch = analyze_to_batch(
        &identity,
        &fixture_analysis(),
        &[hatchback_hit()],
        "yolo@test",
        &[impact_caption()],
        "vlm@test",
    )
    .expect("analyze");

    let videos: Vec<_> = batch
        .sources
        .iter()
        .filter(|source| source.source_kind == SourceKind::Video)
        .collect();
    assert_eq!(videos.len(), 1);
    let video = videos[0];
    assert_eq!(video.sha256, "50".repeat(32));
    assert!(video.segments.iter().any(|s| s.id.contains("-scene-")));
    assert!(video.segments.iter().any(|s| s.id.contains("-det-")));
    assert!(video.segments.iter().any(|s| s.id.contains("-cap-")));

    let det = video
        .segments
        .iter()
        .find(|s| s.id.contains("-det-"))
        .expect("box");
    assert_eq!(
        det.content[0].extraction.review_state,
        ReviewState::Suggested
    );
    let cap = video
        .segments
        .iter()
        .find(|s| s.id.contains("-cap-"))
        .expect("caption");
    assert_eq!(cap.content[0].kind, ContentKind::Observation);
    assert!(cap.bounding_box.is_none());
    assert!(det.bounding_box.is_some());

    store.import_normalized(&batch).expect("import once");
    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    let camera = ledger
        .iter()
        .filter(|item| item.source == "Oak and Third camera prosecution clip.mp4")
        .count();
    assert_eq!(camera, 1, "the original is listed once: {ledger:?}");
}

#[test]
fn analyze_without_backends_is_just_scenes() {
    let (_, case_id) = seeded();
    let identity = identity(case_id);
    let analysis = fixture_analysis();
    let scenes = scenes_to_batch(&identity, &analysis).expect("scenes");
    let analyzed = analyze_to_batch(&identity, &analysis, &[], "", &[], "").expect("analyze");
    assert_eq!(scenes.sources.len(), analyzed.sources.len());
    assert_eq!(
        scenes.sources[0].segments.len(),
        analyzed.sources[0].segments.len()
    );
    assert!(
        analyzed.sources[0]
            .segments
            .iter()
            .all(|segment| !segment.id.contains("-det-") && !segment.id.contains("-cap-"))
    );
}
