//! Recording-alignment tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::path::PathBuf;

use evidence_audio::DecodedAudio;
use evidence_intake::{
    CaseId, DemoFixture, EdgeKind, NodeKind, ReviewState, Store, TemporalRelation,
};
use evidence_video::{
    EXTRACTOR_SYNC, Scene, SceneAnalysis, SyncMeasurement, SyncOptions, SyncPair, SyncSide,
    VideoIdentity, measure_offset, measurement_to_batch, scenes_to_batch, sync_pair,
};

/// Source rate of every synthesized recording below.
const SOURCE_RATE: u32 = 8_000;
/// Length of every synthesized recording, in seconds.
const SECONDS: usize = 20;
/// The offset the recovery test hides in the second recording.
const OFFSET_MS: i64 = 3_410;

/// A tiny linear congruential generator, so every "noise" here is the same
/// noise on every machine and every run. Nothing in this file is random.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next value in `[-1.0, 1.0)`.
    fn next_unit(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits = (self.0 >> 33) as u32;
        (bits as f32 / (1_u32 << 30) as f32) - 1.0
    }
}

fn samples(count: usize, seed: u64, level: f32) -> Vec<f32> {
    let mut noise = Lcg::new(seed);
    (0..count).map(|_| noise.next_unit() * level).collect()
}

/// One mono recording at the source rate.
fn recording(samples: Vec<f32>) -> DecodedAudio {
    DecodedAudio {
        sha256: "aa".repeat(32),
        byte_length: samples.len() as u64 * 2,
        media_type: "audio/wav".to_owned(),
        sample_rate: SOURCE_RATE,
        channels: 1,
        samples,
    }
}

/// The ambient scene both cameras hear.
fn ambient() -> Vec<f32> {
    samples(SECONDS * SOURCE_RATE as usize, 0x5EED_1234, 0.5)
}

/// The scene, delayed by `lead_ms` of near-silence, under its own mic noise.
fn heard(scene: &[f32], lead_ms: u64, seed: u64) -> DecodedAudio {
    let lead = (lead_ms * u64::from(SOURCE_RATE) / 1_000) as usize;
    let mut track = vec![0.0_f32; lead];
    track.extend_from_slice(scene);
    let mic = samples(track.len(), seed, 0.25);
    for (sample, own) in track.iter_mut().zip(&mic) {
        *sample += own;
    }
    recording(track)
}

/// Half a working sample, plus a millisecond of rounding slack.
fn tolerance_ms(options: &SyncOptions) -> i64 {
    (1_000 / i64::from(options.work_rate_hz)) + 1
}

#[test]
fn a_known_offset_is_recovered_to_within_one_work_sample() {
    let options = SyncOptions::default();
    let scene = ambient();
    let first = heard(&scene, 0, 0x1111_2222);
    let second = heard(&scene, OFFSET_MS as u64, 0x3333_4444);

    let forward = measure_offset(&first, &second, &options)
        .expect("measure")
        .expect("a confident peak");
    assert!(
        (forward.offset_ms - OFFSET_MS).abs() <= tolerance_ms(&options),
        "measured {} ms, expected {OFFSET_MS} ms",
        forward.offset_ms
    );
    assert!(forward.peak > 0.5, "peak was {}", forward.peak);
    assert!(forward.prominence >= options.min_prominence);
    assert_eq!(forward.work_rate_hz, options.work_rate_hz);
    assert_eq!(forward.a_duration_ms, 20_000);
    assert_eq!(forward.b_duration_ms, 20_000 + OFFSET_MS as u64);

    // The other direction is the same measurement with the other sign: the
    // sound on the delayed recording arrives *before* the same sound on the
    // one it is compared against.
    let backward = measure_offset(&second, &first, &options)
        .expect("measure")
        .expect("a confident peak");
    assert!(
        (backward.offset_ms + OFFSET_MS).abs() <= tolerance_ms(&options),
        "measured {} ms, expected {} ms",
        backward.offset_ms,
        -OFFSET_MS
    );
    assert!(backward.peak > 0.5, "peak was {}", backward.peak);
}

#[test]
fn unrelated_recordings_yield_no_edge() {
    let count = SECONDS * SOURCE_RATE as usize;
    let first = recording(samples(count, 0xA1A1_A1A1, 0.5));
    let second = recording(samples(count, 0xB2B2_B2B2, 0.5));

    let measured = measure_offset(&first, &second, &SyncOptions::default()).expect("measure");
    assert!(
        measured.is_none(),
        "two unrelated recordings were aligned anyway: {measured:?}"
    );
}

#[test]
fn silence_yields_no_edge() {
    let count = SECONDS * SOURCE_RATE as usize;
    let quiet = recording(vec![0.0_f32; count]);
    let scene = recording(ambient());

    assert!(
        measure_offset(&quiet, &quiet, &SyncOptions::default())
            .expect("measure")
            .is_none()
    );
    assert!(
        measure_offset(&scene, &quiet, &SyncOptions::default())
            .expect("measure")
            .is_none()
    );
    assert!(
        measure_offset(&quiet, &scene, &SyncOptions::default())
            .expect("measure")
            .is_none()
    );
}

fn side(source_id: &str, logical_name: &str) -> SyncSide {
    SyncSide {
        source_id: source_id.to_owned(),
        logical_name: logical_name.to_owned(),
        path: PathBuf::from(format!("{source_id}.mp4")),
    }
}

fn pair(case_id: CaseId) -> SyncPair {
    SyncPair {
        case_id,
        a: side("hr-src-bwc-mendez", "Mendez body camera.mp4"),
        b: side("hr-src-bwc-okafor", "Okafor body camera.mp4"),
    }
}

fn measurement(offset_ms: i64) -> SyncMeasurement {
    SyncMeasurement {
        offset_ms,
        peak: 0.83,
        prominence: 41.5,
        work_rate_hz: 4_000,
        searched_lag_ms: 16_000,
        a_duration_ms: 20_000,
        b_duration_ms: 23_410,
    }
}

#[test]
fn the_sync_edge_names_both_originals_and_is_suggested() {
    let pair = pair(CaseId("case-hit-run-001".to_owned()));
    let batch = measurement_to_batch(&pair, &measurement(OFFSET_MS)).expect("map");

    assert!(
        batch.sources.is_empty(),
        "an offset is not a new original; the batch must carry no sources"
    );
    assert_eq!(batch.edges.len(), 1);
    let edge = &batch.edges[0];
    assert_eq!(edge.from_kind, NodeKind::Source);
    assert_eq!(edge.from_id, "hr-src-bwc-mendez");
    assert_eq!(edge.to_kind, NodeKind::Source);
    assert_eq!(edge.to_id, "hr-src-bwc-okafor");
    assert_eq!(edge.relation, EdgeKind::TemporallyOverlaps);
    assert_eq!(edge.extraction.extractor, EXTRACTOR_SYNC);
    assert!(edge.extraction.machine_generated);
    assert_eq!(edge.extraction.review_state, ReviewState::Suggested);
    assert!(edge.extraction.confidence.is_none());

    assert!(
        edge.rationale.starts_with("audio cross-correlation:"),
        "{}",
        edge.rationale
    );
    assert!(edge.rationale.contains("`Okafor body camera.mp4` lags"));
    assert!(edge.rationale.contains("`Mendez body camera.mp4`"));
    assert!(edge.rationale.contains("by 3410 ms"));
    assert!(edge.rationale.contains("peak 0.83"));
    assert!(edge.rationale.contains("4000 Hz mono"));
    // Nothing about who or what is on either recording.
    assert!(!edge.rationale.to_lowercase().contains("same event"));

    // A negative offset reads the other way round, with the sign in the word.
    let leading = measurement_to_batch(&pair, &measurement(-OFFSET_MS)).expect("map");
    assert!(
        leading.edges[0]
            .rationale
            .contains("`Okafor body camera.mp4` leads")
    );
    assert!(leading.edges[0].rationale.contains("by 3410 ms"));
    // The identifier is a function of the pair, not of the measurement.
    assert_eq!(leading.edges[0].id, edge.id);
}

#[test]
fn a_pair_with_one_source_id_is_refused() {
    let case_id = CaseId("case-hit-run-001".to_owned());
    let mut same = pair(case_id);
    same.b = side("hr-src-bwc-mendez", "The very same camera.mp4");

    let error = measurement_to_batch(&same, &measurement(OFFSET_MS))
        .expect_err("a recording cannot be synchronised against itself");
    assert!(error.to_string().contains("itself"), "{error}");

    let error = sync_pair(&same, &SyncOptions::default()).expect_err("refused before any decode");
    assert!(error.to_string().contains("itself"), "{error}");
}

fn identity(case_id: CaseId, source_id: &str, logical_name: &str, hash: &str) -> VideoIdentity {
    VideoIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: source_id.to_owned(),
        logical_name: logical_name.to_owned(),
        media_type: "video/mp4".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: hash.repeat(32),
        byte_length: 8_120_004,
    }
}

fn one_scene(duration_ms: u64) -> SceneAnalysis {
    SceneAnalysis {
        duration_ms,
        last_video_pts_ms: Some(duration_ms),
        scenes: vec![Scene {
            index: 1,
            start_ms: 0,
            end_ms: duration_ms,
            cut_score_millis: None,
            keyframe: None,
        }],
        dropout: None,
    }
}

#[test]
fn an_offset_edge_imports_between_two_ingested_videos() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let pair = pair(case_id.clone());

    for (side, hash, duration) in [(&pair.a, "51", 20_000_u64), (&pair.b, "52", 23_410)] {
        let batch = scenes_to_batch(
            &identity(case_id.clone(), &side.source_id, &side.logical_name, hash),
            &one_scene(duration),
        )
        .expect("map");
        store.import_normalized(&batch).expect("import the video");
    }

    let batch = measurement_to_batch(&pair, &measurement(OFFSET_MS)).expect("map");
    let edge_id = batch.edges[0].id.clone();
    store.import_normalized(&batch).expect("import the offset");

    let queue = store.review_queue(&case_id).expect("queue");
    let edge = queue
        .iter()
        .find(|item| item.target_kind == "edge" && item.target_id == edge_id)
        .expect("the offset edge waits in the review queue");
    assert_eq!(edge.review_state, "suggested");
    assert!(edge.summary.contains("temporally_overlaps"), "{edge:?}");
    assert!(edge.summary.contains(&pair.a.source_id), "{edge:?}");
    assert!(edge.summary.contains(&pair.b.source_id), "{edge:?}");
}
