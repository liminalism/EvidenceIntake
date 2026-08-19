//! Soundtrack-merge tests named as assertions about the domain rules.

#![allow(missing_docs)]

use evidence_audio::{
    EXTRACTOR_DIARIZE, MappingOptions, SourceIdentity, WhisperxTranscript, format_locator,
    transcript_to_batch,
};
use evidence_intake::{
    CaseId, ContentKind, DemoFixture, NormalizedBatch, ReviewState, SourceKind, Store,
    TemporalRelation,
};
use evidence_video::{Scene, SceneAnalysis, VideoIdentity, merge_soundtrack, scenes_to_batch};

const ORIGINAL_HASH: &str = "5050505050505050505050505050505050505050505050505050505050505050";
const ORIGINAL_BYTES: u64 = 12_280_334;
const LOGICAL_NAME: &str = "Oak and Third camera prosecution clip.mp4";

fn identity(case_id: CaseId) -> VideoIdentity {
    VideoIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: "hr-src-camera".to_owned(),
        logical_name: LOGICAL_NAME.to_owned(),
        media_type: "video/mp4".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: ORIGINAL_HASH.to_owned(),
        byte_length: ORIGINAL_BYTES,
    }
}

/// The same original, as the audio adapter would have hashed it.
fn heard(case_id: CaseId, sha256: &str, byte_length: u64) -> SourceIdentity {
    SourceIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: "hr-src-camera".to_owned(),
        logical_name: LOGICAL_NAME.to_owned(),
        media_type: "video/mp4".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        source_kind: SourceKind::Video,
        sha256: sha256.to_owned(),
        byte_length,
    }
}

fn fixture_scenes() -> SceneAnalysis {
    let spans = [(0_u64, 9_800), (9_800, 15_400), (15_400, 28_000)];
    let scenes = spans
        .into_iter()
        .enumerate()
        .map(|(index, (start_ms, end_ms))| Scene {
            index: u32::try_from(index + 1).unwrap(),
            start_ms,
            end_ms,
            cut_score_millis: None,
            keyframe: None,
        })
        .collect();
    SceneAnalysis {
        duration_ms: 28_000,
        last_video_pts_ms: Some(28_000),
        scenes,
        dropout: None,
    }
}

/// Three spoken lines on the original video timeline, one of them diarized.
fn transcript() -> WhisperxTranscript {
    WhisperxTranscript::from_json(
        r#"{
          "language": "en",
          "segments": [
            {"start": 0.5, "end": 3.0,
             "text": "Watch the kerbside lane, he is not slowing down.",
             "words": [], "speaker": null, "avg_logprob": -0.2},
            {"start": 3.2, "end": 6.5,
             "text": "The white sedan just clipped that hatchback and kept going.",
             "words": [], "speaker": "SPEAKER_01", "avg_logprob": -0.15},
            {"start": 6.8, "end": 9.0,
             "text": "Somebody get the plate before it turns.",
             "words": [], "speaker": null, "avg_logprob": -0.3}
          ]
        }"#,
    )
    .expect("whisperx json")
}

fn soundtrack(case_id: CaseId, sha256: &str, byte_length: u64) -> NormalizedBatch {
    transcript_to_batch(
        &heard(case_id, sha256, byte_length),
        &transcript(),
        &MappingOptions::new("whisperx@test"),
    )
    .expect("map transcript")
}

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

/// Scenes plus the soundtrack of the same original, already merged.
fn merged(case_id: &CaseId) -> (VideoIdentity, NormalizedBatch) {
    let identity = identity(case_id.clone());
    let mut batch = scenes_to_batch(&identity, &fixture_scenes()).expect("scenes");
    let spoken = soundtrack(case_id.clone(), ORIGINAL_HASH, ORIGINAL_BYTES);
    merge_soundtrack(&mut batch, &identity, spoken).expect("merge");
    (identity, batch)
}

#[test]
fn a_spoken_line_lands_on_the_video_at_its_second() {
    let (mut store, case_id) = seeded();
    let (identity, batch) = merged(&case_id);

    let video = batch
        .sources
        .iter()
        .find(|source| source.id == identity.source_id)
        .expect("video source");
    assert_eq!(video.source_kind, SourceKind::Video);
    let line = video
        .segments
        .iter()
        .find(|segment| segment.id == "hr-src-camera-seg-0001")
        .expect("the second spoken line");
    assert_eq!(line.start_ms, Some(3_200));
    assert_eq!(line.end_ms, Some(6_500));
    assert_eq!(line.locator, format_locator(3_200, 6_500));
    assert_eq!(line.content[0].kind, ContentKind::Statement);
    assert_eq!(
        line.content[0].extraction.review_state,
        ReviewState::Suggested
    );

    store.import_normalized(&batch).expect("import");

    let hits = store.search(&case_id, "kerbside", 25).expect("search");
    let hit = hits.first().expect("the spoken sentence is searchable");
    assert_eq!(hit.source, LOGICAL_NAME);
    assert_eq!(hit.locator, format_locator(500, 3_000));
    assert!(hit.machine_generated);
    assert_eq!(hit.review_state, "suggested");
}

#[test]
fn soundtrack_and_scenes_share_one_source_and_hash() {
    let (mut store, case_id) = seeded();
    let (_, batch) = merged(&case_id);

    let videos: Vec<_> = batch
        .sources
        .iter()
        .filter(|source| source.source_kind == SourceKind::Video)
        .collect();
    assert_eq!(videos.len(), 1, "scenes and speech share one source");
    let video = videos[0];
    assert_eq!(video.sha256, ORIGINAL_HASH);
    assert_eq!(video.byte_length, ORIGINAL_BYTES);
    assert!(video.segments.iter().any(|s| s.id.contains("-scene-")));
    assert!(video.segments.iter().any(|s| s.id.contains("-seg-")));

    store.import_normalized(&batch).expect("import");
    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    assert_eq!(
        ledger
            .iter()
            .filter(|item| item.source == LOGICAL_NAME)
            .count(),
        1,
        "the original is listed once: {ledger:?}"
    );
}

#[test]
fn a_soundtrack_hashed_from_a_different_original_is_refused() {
    let (_, case_id) = seeded();
    let identity = identity(case_id.clone());
    let mut batch = scenes_to_batch(&identity, &fixture_scenes()).expect("scenes");

    let elsewhere = soundtrack(case_id.clone(), &"77".repeat(32), ORIGINAL_BYTES);
    let error = merge_soundtrack(&mut batch, &identity, elsewhere)
        .expect_err("a soundtrack from another file");
    assert!(error.to_string().contains("different original"), "{error}");

    let mut batch = scenes_to_batch(&identity, &fixture_scenes()).expect("scenes");
    let resized = soundtrack(case_id, ORIGINAL_HASH, ORIGINAL_BYTES + 1);
    let error =
        merge_soundtrack(&mut batch, &identity, resized).expect_err("a different byte length");
    assert!(error.to_string().contains("different original"), "{error}");
}

#[test]
fn soundtrack_speakers_stay_anonymous() {
    let (_, case_id) = seeded();
    let (identity, batch) = merged(&case_id);

    let video = batch
        .sources
        .iter()
        .find(|source| source.id == identity.source_id)
        .expect("video source");
    let label = video
        .segments
        .iter()
        .flat_map(|segment| &segment.content)
        .find(|content| content.extraction.extractor == EXTRACTOR_DIARIZE)
        .expect("a diarization observation");
    assert!(label.speaker_entity_id.is_none());
    assert!(label.attributed_to_entity_id.is_none());
    assert!(label.text.contains("SPEAKER_01"), "{}", label.text);
    assert!(
        video
            .segments
            .iter()
            .flat_map(|segment| &segment.content)
            .all(|content| content.speaker_entity_id.is_none())
    );
}

#[test]
fn machine_soundtrack_cannot_arrive_verified() {
    let (mut store, case_id) = seeded();
    let (_, mut batch) = merged(&case_id);

    let line = batch.sources[0]
        .segments
        .iter_mut()
        .find(|segment| segment.id.contains("-seg-"))
        .expect("a spoken line");
    line.content[0].extraction.review_state = ReviewState::Verified;

    let error = store
        .import_normalized(&batch)
        .expect_err("verified machine statement");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}
