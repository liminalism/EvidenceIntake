//! Scene-cut tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::path::PathBuf;
use std::process::Command;

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, EdgeKind, NodeKind, ReviewState, SourceKind, Store,
    TemporalRelation,
};
use evidence_video::{
    EXTRACTOR_SCENE, Keyframe, Scene, SceneAnalysis, SceneRequest, VideoIdentity, cut_scenes,
    detect_scenes, dropout_span, ffprobe_available, parse_scene_report, scenes_to_batch,
};

fn ffmpeg_available() -> bool {
    evidence_audio::ffmpeg_available() && ffprobe_available()
}

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

/// The three camera beats from the hit-and-run fixture.
fn fixture_scenes(with_stills: bool) -> SceneAnalysis {
    let spans = [(0_u64, 9_800), (9_800, 15_400), (15_400, 28_000)];
    let scenes = spans
        .into_iter()
        .enumerate()
        .map(|(index, (start_ms, end_ms))| Scene {
            index: u32::try_from(index + 1).unwrap(),
            start_ms,
            end_ms,
            cut_score_millis: None,
            keyframe: with_stills.then(|| Keyframe {
                path: PathBuf::from(format!("scene-{}.jpg", index + 1)),
                sha256: format!("{:02x}", index + 1).repeat(32),
                byte_length: 1_024,
                width: Some(1920),
                height: Some(1080),
            }),
        })
        .collect();
    SceneAnalysis {
        duration_ms: 28_000,
        last_video_pts_ms: Some(28_000),
        scenes,
        dropout: None,
    }
}

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

#[test]
fn a_scene_observation_enters_suggested_on_the_original_timeline() {
    let (mut store, case_id) = seeded();
    let batch = scenes_to_batch(&identity(case_id.clone()), &fixture_scenes(false)).expect("map");

    assert_eq!(batch.sources.len(), 1);
    let video = &batch.sources[0];
    assert_eq!(video.source_kind, SourceKind::Video);
    assert_eq!(video.sha256, "50".repeat(32));
    assert_eq!(video.segments.len(), 3);
    assert_eq!(
        video.segments[1].locator,
        "scene 2, 00:00:09.800–00:00:15.400"
    );
    assert_eq!(video.segments[1].start_ms, Some(9_800));
    assert_eq!(video.segments[1].end_ms, Some(15_400));
    let observation = &video.segments[1].content[0];
    assert_eq!(observation.kind, ContentKind::Observation);
    assert_eq!(observation.extraction.extractor, EXTRACTOR_SCENE);
    assert!(observation.extraction.machine_generated);
    assert_eq!(observation.extraction.review_state, ReviewState::Suggested);
    assert!(observation.normalized_start.is_none());
    assert!(observation.time_basis.is_none());

    store.import_normalized(&batch).expect("import");
}

#[test]
fn a_keyframe_is_a_derived_source_not_the_original() {
    let (mut store, case_id) = seeded();
    let analysis = fixture_scenes(true);
    let original_hash = "50".repeat(32);
    let batch = scenes_to_batch(&identity(case_id.clone()), &analysis).expect("map");

    assert_eq!(batch.sources.len(), 4);
    assert_eq!(batch.sources[0].source_kind, SourceKind::Video);
    assert_eq!(batch.sources[0].sha256, original_hash);
    for still in &batch.sources[1..] {
        assert_eq!(still.source_kind, SourceKind::Other);
        assert_eq!(still.media_type, "image/jpeg");
        assert_ne!(still.sha256, original_hash);
        assert!(still.logical_name.contains('@'));
        let text = &still.segments[0].content[0].text;
        assert!(text.contains("Derived working copy"));
        assert!(!text.to_lowercase().contains("original frame is this jpeg"));
    }

    store.import_normalized(&batch).expect("import");
    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    assert!(
        ledger
            .iter()
            .any(|item| item.source.contains("Oak and Third"))
    );
    assert!(ledger.iter().any(|item| item.source.contains('@')));
}

#[test]
fn a_keyframe_says_structurally_which_video_it_was_cut_from() {
    let (mut store, case_id) = seeded();
    let batch = scenes_to_batch(&identity(case_id.clone()), &fixture_scenes(true)).expect("map");

    assert_eq!(batch.edges.len(), 3, "one derived_from edge per still");
    for (still, edge) in batch.sources[1..].iter().zip(&batch.edges) {
        assert_eq!(edge.from_kind, NodeKind::Source);
        assert_eq!(edge.from_id, still.id);
        assert_eq!(edge.relation, EdgeKind::DerivedFrom);
        assert_eq!(edge.to_kind, NodeKind::Source);
        assert_eq!(edge.to_id, batch.sources[0].id);
        assert!(edge.extraction.machine_generated);
        assert_eq!(edge.extraction.review_state, ReviewState::Suggested);
        assert!(edge.rationale.contains("derived working copy"));
    }

    store.import_normalized(&batch).expect("import");
    let queue = store.review_queue(&case_id).expect("queue");
    let edges: Vec<_> = queue
        .iter()
        .filter(|item| item.target_kind == "edge" && item.target_id.contains("-stilledge-"))
        .collect();
    assert_eq!(edges.len(), 3);
    assert!(edges.iter().all(|item| item.machine_generated));
    assert!(edges.iter().all(|item| item.review_state == "suggested"));
}

#[test]
fn machine_scenes_cannot_arrive_verified() {
    let (_, case_id) = seeded();
    let mut batch = scenes_to_batch(&identity(case_id), &fixture_scenes(false)).expect("map");
    batch.sources[0].segments[0].content[0]
        .extraction
        .review_state = ReviewState::Verified;
    let mut store = Store::in_memory().expect("store");
    DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let error = store
        .import_normalized(&batch)
        .expect_err("verified machine scene");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}

#[test]
fn a_tail_shorter_than_claimed_duration_is_a_recording_gap() {
    assert_eq!(dropout_span(10_000, 12_500, 2_000), Some((10_000, 12_500)));
    assert_eq!(dropout_span(10_000, 11_000, 2_000), None);

    let (_, case_id) = seeded();
    let mut analysis = fixture_scenes(false);
    analysis.last_video_pts_ms = Some(10_000);
    analysis.dropout = dropout_span(10_000, 28_000, 2_000);
    let batch = scenes_to_batch(&identity(case_id), &analysis).expect("map");
    let gap = batch.sources[0]
        .segments
        .iter()
        .find(|segment| {
            segment
                .content
                .iter()
                .any(|item| item.kind == ContentKind::RecordingGap)
        })
        .expect("gap");
    assert_eq!(gap.start_ms, Some(10_000));
    assert_eq!(gap.end_ms, Some(28_000));
}

#[test]
fn parse_scene_report_reads_pts_time_lines() {
    let report = "\
frame:0    pts:0       pts_time:0.000000
lavfi.scene_score=0.412
frame:12   pts:294     pts_time:9.800000
lavfi.scene_score=0.880
";
    assert_eq!(parse_scene_report(report), vec![0, 9_800]);
}

#[test]
fn a_two_color_clip_is_cut_into_more_than_one_scene() {
    if !ffmpeg_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmpdir");
    let mp4 = dir.path().join("two_scenes.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:d=1:r=10",
            "-f",
            "lavfi",
            "-i",
            "color=c=white:s=64x64:d=1:r=10",
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0",
            "-an",
        ])
        .arg(&mp4)
        .status()
        .expect("ffmpeg concat");
    if !status.success() {
        return;
    }

    let analysis = detect_scenes(&mp4, 0.2, 2_000, Some(dir.path())).expect("detect");
    assert!(
        analysis.scenes.len() >= 2,
        "black-then-white should cut: {:?}",
        analysis.scenes
    );
    assert!(analysis.scenes.iter().any(|scene| scene.keyframe.is_some()));
    assert_eq!(analysis.dropout, None);

    let (mut store, case_id) = seeded();
    let batch = cut_scenes(&SceneRequest {
        case_id: case_id.clone(),
        production_id: "prod-01".to_owned(),
        source_id: "clip-two-scenes".to_owned(),
        path: mp4,
        logical_name: Some("two_scenes.mp4".to_owned()),
        temporal_relation: TemporalRelation::Contemporaneous,
        threshold: 0.2,
        gap_ms: 2_000,
        stills_dir: Some(dir.path().to_path_buf()),
    })
    .expect("cut");
    assert_eq!(batch.sources[0].source_kind, SourceKind::Video);
    assert!(batch.sources.len() > 1);
    store.import_normalized(&batch).expect("import");
}
