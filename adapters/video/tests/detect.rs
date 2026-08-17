//! Detector tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::path::PathBuf;

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ReviewState, SourceKind, Store, TemporalRelation,
};
use evidence_video::{
    Detection, EXTRACTOR_DETECT, Keyframe, Scene, SceneAnalysis, VideoIdentity, attach_detections,
    coco_label, parse_yolo_txt, place_on_scene, scenes_and_detections_to_batch, scenes_to_batch,
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
                sha256: "51".repeat(32),
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

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

#[test]
fn a_boxed_detection_is_an_observation_not_a_finding() {
    let (mut store, case_id) = seeded();
    let batch = scenes_and_detections_to_batch(
        &identity(case_id.clone()),
        &fixture_analysis(),
        &[hatchback_hit()],
        "yolo@yolov8n.pt",
    )
    .expect("map");

    let video = batch
        .sources
        .iter()
        .find(|source| source.source_kind == SourceKind::Video)
        .expect("video");
    let det = video
        .segments
        .iter()
        .find(|segment| segment.id.contains("-det-"))
        .expect("detection segment");
    assert_eq!(
        det.locator,
        "scene 2, 00:00:09.800–00:00:15.400; detector proposed car (#1)"
    );
    assert_eq!(det.start_ms, Some(9_800));
    assert_eq!(det.end_ms, Some(15_400));
    assert_eq!(det.bounding_box, Some([240.0, 400.0, 480.0, 220.0]));
    let content = &det.content[0];
    assert_eq!(content.kind, ContentKind::Observation);
    assert_eq!(content.extraction.extractor, EXTRACTOR_DETECT);
    assert_eq!(content.extraction.review_state, ReviewState::Suggested);
    assert!(content.speaker_entity_id.is_none());
    assert!(content.text.contains("Detector proposed `car`"));
    assert!(content.text.contains("Suggested"));
    assert!(!content.text.to_lowercase().contains("there is a car"));
    assert!(!content.text.to_lowercase().contains("identified"));

    store.import_normalized(&batch).expect("import");
}

#[test]
fn machine_detections_cannot_arrive_verified() {
    let (_, case_id) = seeded();
    let mut batch = scenes_and_detections_to_batch(
        &identity(case_id),
        &fixture_analysis(),
        &[hatchback_hit()],
        "yolo@test",
    )
    .expect("map");
    let det = batch.sources[0]
        .segments
        .iter_mut()
        .find(|segment| segment.id.contains("-det-"))
        .expect("det");
    det.content[0].extraction.review_state = ReviewState::Verified;
    let mut store = Store::in_memory().expect("store");
    DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let error = store
        .import_normalized(&batch)
        .expect_err("verified detection");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}

#[test]
fn json_detections_attach_to_the_video_source() {
    let (mut store, case_id) = seeded();
    let identity = identity(case_id.clone());
    let mut batch = scenes_to_batch(&identity, &fixture_analysis()).expect("scenes");
    attach_detections(&mut batch, &identity, &[hatchback_hit()], "from-json").expect("attach");

    assert_eq!(batch.sources[0].id, "hr-src-camera");
    assert!(
        batch.sources[0]
            .segments
            .iter()
            .any(|s| s.id.contains("-det-"))
    );
    let stills = batch
        .sources
        .iter()
        .filter(|source| source.source_kind == SourceKind::Other)
        .count();
    assert_eq!(stills, 1);
    assert!(
        batch.sources[1]
            .segments
            .iter()
            .all(|segment| !segment.id.contains("-det-")),
        "boxes belong on the original, not the still"
    );
    store.import_normalized(&batch).expect("import");
}

#[test]
fn normalized_boxes_scale_to_frame_pixels() {
    let scene = &fixture_analysis().scenes[0];
    let raw = evidence_video::RawDetection {
        label: "car".to_owned(),
        bbox: [0.125, 0.370, 0.250, 0.204],
        normalized: true,
        confidence: Some(0.7),
    };
    let placed = place_on_scene(scene, scene.keyframe.as_ref(), &[raw]).expect("place");
    assert_eq!(placed[0].start_ms, 9_800);
    let box_ = placed[0].bbox;
    assert!((box_[0] - 240.0).abs() < 1.0);
    assert!((box_[2] - 480.0).abs() < 1.0);
}

#[test]
fn parse_yolo_txt_reads_class_and_confidence() {
    let hits = parse_yolo_txt("2 0.5 0.5 0.2 0.1 0.91\n0 0.1 0.2 0.05 0.4\n").expect("parse");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].label, "car");
    assert_eq!(coco_label(0), "person");
    assert!(hits[0].normalized);
    assert_eq!(hits[0].confidence, Some(0.91));
    assert!((hits[0].bbox[2] - 0.2).abs() < f64::EPSILON);
}
