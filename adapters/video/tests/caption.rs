//! Scene-description tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::cell::Cell;
use std::path::{Path, PathBuf};

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ReviewState, SourceKind, Store, TemporalRelation,
};
use evidence_video::{
    CaptionBackend, EXTRACTOR_CAPTION, Keyframe, Scene, SceneAnalysis, SceneCaption, VideoIdentity,
    attach_captions, caption_scene, caption_scenes, scenes_and_captions_to_batch, scenes_to_batch,
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
                sha256: "52".repeat(32),
                byte_length: 2_048,
                width: Some(1920),
                height: Some(1080),
            }),
        }],
        dropout: None,
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
fn a_scene_description_is_an_observation_not_a_finding() {
    let (mut store, case_id) = seeded();
    let batch = scenes_and_captions_to_batch(
        &identity(case_id.clone()),
        &fixture_analysis(),
        &[impact_caption()],
        "vlm@llava",
    )
    .expect("map");

    let video = batch
        .sources
        .iter()
        .find(|source| source.source_kind == SourceKind::Video)
        .expect("video");
    let cap = video
        .segments
        .iter()
        .find(|segment| segment.id.contains("-cap-"))
        .expect("caption segment");
    assert_eq!(
        cap.locator,
        "scene 2, 00:00:09.800–00:00:15.400; scene description"
    );
    assert_eq!(cap.start_ms, Some(9_800));
    assert_eq!(cap.end_ms, Some(15_400));
    assert!(cap.bounding_box.is_none());
    let content = &cap.content[0];
    assert_eq!(content.kind, ContentKind::Observation);
    assert_eq!(content.extraction.extractor, EXTRACTOR_CAPTION);
    assert_eq!(content.extraction.review_state, ReviewState::Suggested);
    assert!(content.speaker_entity_id.is_none());
    assert!(content.text.starts_with("Scene description (machine):"));
    assert!(content.text.contains("Suggested"));
    assert!(!content.text.contains(".. Suggested"));
    assert!(!content.text.to_lowercase().contains("the sedan struck"));
    assert!(!content.text.to_lowercase().contains("this proves"));

    store.import_normalized(&batch).expect("import");
}

#[test]
fn machine_descriptions_cannot_arrive_verified() {
    let (_, case_id) = seeded();
    let mut batch = scenes_and_captions_to_batch(
        &identity(case_id),
        &fixture_analysis(),
        &[impact_caption()],
        "vlm@test",
    )
    .expect("map");
    let cap = batch.sources[0]
        .segments
        .iter_mut()
        .find(|segment| segment.id.contains("-cap-"))
        .expect("cap");
    cap.content[0].extraction.review_state = ReviewState::Verified;
    let mut store = Store::in_memory().expect("store");
    DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let error = store
        .import_normalized(&batch)
        .expect_err("verified caption");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}

#[test]
fn json_descriptions_attach_to_the_video_source() {
    let (mut store, case_id) = seeded();
    let identity = identity(case_id.clone());
    let mut batch = scenes_to_batch(&identity, &fixture_analysis()).expect("scenes");
    attach_captions(&mut batch, &identity, &[impact_caption()], "from-json").expect("attach");

    assert_eq!(batch.sources[0].id, "hr-src-camera");
    assert!(
        batch.sources[0]
            .segments
            .iter()
            .any(|s| s.id.contains("-cap-"))
    );
    assert!(
        batch.sources[1]
            .segments
            .iter()
            .all(|segment| !segment.id.contains("-cap-")),
        "descriptions belong on the original, not the still"
    );
    store.import_normalized(&batch).expect("import");
}

#[test]
fn an_empty_description_is_refused() {
    let scene = &fixture_analysis().scenes[0];
    let error = caption_scene(scene, "   ", None).expect_err("empty");
    assert!(error.to_string().contains("empty"), "{error}");
}

struct AbstainingBatchBackend {
    calls: Cell<u32>,
}

impl CaptionBackend for AbstainingBatchBackend {
    fn describe_still(&self, _still: &Path) -> evidence_video::Result<Option<String>> {
        panic!("caption_scenes should use the batch method")
    }

    fn describe_stills(&self, stills: &[PathBuf]) -> evidence_video::Result<Vec<Option<String>>> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(stills.len(), 2);
        Ok(vec![
            Some("A person stands beside a vehicle.".to_owned()),
            None,
        ])
    }

    fn extractor(&self) -> &str {
        EXTRACTOR_CAPTION
    }

    fn version(&self) -> String {
        "test@pinned".to_owned()
    }
}

#[test]
fn scene_captions_load_once_and_preserve_abstention() {
    let mut analysis = fixture_analysis();
    analysis.scenes.push(Scene {
        index: 3,
        start_ms: 15_400,
        end_ms: 20_000,
        cut_score_millis: None,
        keyframe: Some(Keyframe {
            path: PathBuf::from("scene-0003.jpg"),
            sha256: "53".repeat(32),
            byte_length: 2_048,
            width: Some(1920),
            height: Some(1080),
        }),
    });
    let backend = AbstainingBatchBackend {
        calls: Cell::new(0),
    };

    let captions = caption_scenes(&analysis, &backend).expect("caption batch");

    assert_eq!(backend.calls.get(), 1);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].scene_index, Some(2));
    assert_eq!(captions[0].text, "A person stands beside a vehicle.");
}
