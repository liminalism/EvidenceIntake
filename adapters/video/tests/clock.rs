//! Clock-overlay tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::path::{Path, PathBuf};

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, NormalizedContent, ReviewState, SourceKind, Store,
    TemporalRelation,
};
use evidence_video::{
    ClockBackend, ClockReading, EXTRACTOR_CLOCK, Keyframe, OverlayBand, RawClockText, Result,
    Scene, SceneAnalysis, TesseractCliBackend, VideoIdentity, attach_clock_readings,
    parse_clock_text, read_clocks, scenes_to_batch,
};

/// A backend that answers with fixed OCR text and never opens the still.
struct StubClockBackend {
    texts: Vec<RawClockText>,
}

impl ClockBackend for StubClockBackend {
    fn read(&self, _still: &Path) -> Result<Vec<RawClockText>> {
        Ok(self.texts.clone())
    }

    fn version(&self) -> String {
        "stub@1".to_owned()
    }
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

fn keyframe(sha: &str) -> Keyframe {
    Keyframe {
        path: PathBuf::from("no-such-still.jpg"),
        sha256: sha.repeat(32),
        byte_length: 2_048,
        width: Some(1920),
        height: Some(1080),
    }
}

fn one_scene() -> SceneAnalysis {
    SceneAnalysis {
        duration_ms: 28_000,
        last_video_pts_ms: Some(28_000),
        scenes: vec![Scene {
            index: 2,
            start_ms: 9_800,
            end_ms: 15_400,
            cut_score_millis: None,
            keyframe: Some(keyframe("53")),
        }],
        dropout: None,
    }
}

fn two_scenes() -> SceneAnalysis {
    SceneAnalysis {
        duration_ms: 28_000,
        last_video_pts_ms: Some(28_000),
        scenes: vec![
            Scene {
                index: 1,
                start_ms: 0,
                end_ms: 9_800,
                cut_score_millis: None,
                keyframe: Some(keyframe("54")),
            },
            Scene {
                index: 2,
                start_ms: 9_800,
                end_ms: 15_400,
                cut_score_millis: None,
                keyframe: Some(keyframe("55")),
            },
        ],
        dropout: None,
    }
}

fn reading(scene_index: u32, start_ms: u64, end_ms: u64, raw: &str, parsed: &str) -> ClockReading {
    ClockReading {
        scene_index,
        start_ms,
        end_ms,
        band: OverlayBand::Bottom,
        raw_text: raw.to_owned(),
        reading: parsed.to_owned(),
    }
}

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

fn clock_content(batch: &evidence_intake::NormalizedBatch) -> Vec<&NormalizedContent> {
    batch
        .sources
        .iter()
        .filter(|source| source.source_kind == SourceKind::Video)
        .flat_map(|source| source.segments.iter())
        .filter(|segment| segment.id.contains("-clk-"))
        .flat_map(|segment| segment.content.iter())
        .collect()
}

#[test]
fn a_clock_reading_is_raw_time_not_a_normalized_interval() {
    let (mut store, case_id) = seeded();
    let identity = identity(case_id);
    let mut batch = scenes_to_batch(&identity, &one_scene()).expect("scenes");
    attach_clock_readings(
        &mut batch,
        &identity,
        &[reading(
            2,
            9_800,
            15_400,
            "2026-02-27 21:07:00",
            "2026-02-27 21:07:00",
        )],
        "tesseract@eng/psm7",
    )
    .expect("attach");

    let video = &batch.sources[0];
    let segment = video
        .segments
        .iter()
        .find(|segment| segment.id.contains("-clk-"))
        .expect("clock segment");
    assert_eq!(
        segment.locator,
        "scene 2, 00:00:09.800–00:00:15.400; clock overlay (bottom)"
    );
    assert_eq!(segment.start_ms, Some(9_800));
    assert_eq!(segment.end_ms, Some(15_400));

    let content = &segment.content[0];
    assert_eq!(content.kind, ContentKind::Observation);
    assert_eq!(
        content.raw_time.as_deref(),
        Some("overlay 2026-02-27 21:07:00")
    );
    assert!(content.normalized_start.is_none());
    assert!(content.normalized_end.is_none());
    assert!(content.time_basis.is_none());
    assert!(content.content_created_at.is_none());
    assert!(content.asserted_time.is_none());
    assert!(content.speaker_entity_id.is_none());
    assert!(content.attributed_to_entity_id.is_none());
    assert_eq!(content.extraction.extractor, EXTRACTOR_CLOCK);
    assert_eq!(content.extraction.review_state, ReviewState::Suggested);
    assert!(content.extraction.machine_generated);
    assert!(
        content
            .text
            .contains("read by OCR as `2026-02-27 21:07:00`")
    );
    assert!(content.text.contains("verify against the original frame"));

    store.import_normalized(&batch).expect("import");
}

#[test]
fn an_unparseable_overlay_yields_no_reading() {
    assert_eq!(parse_clock_text("BWC ###:##"), None);
    let backend = StubClockBackend {
        texts: vec![
            RawClockText {
                band: OverlayBand::Top,
                text: "BWC ###:## AXON".to_owned(),
            },
            RawClockText {
                band: OverlayBand::Bottom,
                text: "CAM 04 --:--:--".to_owned(),
            },
        ],
    };
    let readings = read_clocks(&one_scene(), &backend).expect("read");
    assert!(readings.is_empty(), "{readings:?}");
}

#[test]
fn an_ocr_substitution_keeps_both_the_reading_and_the_original_text() {
    let backend = StubClockBackend {
        texts: vec![RawClockText {
            band: OverlayBand::Top,
            text: "AXON 2O26-02-27 2l:07:00".to_owned(),
        }],
    };
    let readings = read_clocks(&one_scene(), &backend).expect("read");
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].reading, "2026-02-27 21:07:00");
    assert_eq!(readings[0].raw_text, "AXON 2O26-02-27 2l:07:00");
    assert_eq!(readings[0].band, OverlayBand::Top);
    assert_eq!(readings[0].scene_index, 2);
    assert_eq!(readings[0].start_ms, 9_800);

    let (_, case_id) = seeded();
    let identity = identity(case_id);
    let mut batch = scenes_to_batch(&identity, &one_scene()).expect("scenes");
    attach_clock_readings(&mut batch, &identity, &readings, "stub@1").expect("attach");
    let content = clock_content(&batch);
    assert!(
        content[0]
            .text
            .contains("OCR text was `AXON 2O26-02-27 2l:07:00`"),
        "{}",
        content[0].text
    );
}

#[test]
fn parse_clock_text_accepts_device_shapes_and_refuses_ranges() {
    let accepted = [
        ("2026-02-27 21:07:00", "2026-02-27 21:07:00"),
        ("2026-02-27 21:07:00.800", "2026-02-27 21:07:00.800"),
        ("2026-02-27 21:07:00Z", "2026-02-27 21:07:00Z"),
        (
            "2026-02-27 21:07:00.800-05:00",
            "2026-02-27 21:07:00.800-05:00",
        ),
        ("2026-02-27 21:07:00-0500", "2026-02-27 21:07:00-0500"),
        ("2026/02/27 21:07:00", "2026/02/27 21:07:00"),
        ("2026/02/27 21:07:00.800", "2026/02/27 21:07:00.800"),
        ("02/27/2026 21:07:00", "02/27/2026 21:07:00"),
        ("02-27-2026 21:07:00", "02-27-2026 21:07:00"),
        ("27.02.2026 21:07:00", "27.02.2026 21:07:00"),
        ("21:07:00", "21:07:00"),
        ("21:07:06.400", "21:07:06.400"),
        ("09:07:06.4 PM", "09:07:06.4 PM"),
        ("27.02.2026 21:07:00.80", "27.02.2026 21:07:00.80"),
        ("09:07:00 PM", "09:07:00 PM"),
        ("CAM 04  02/27/2026 21:07:00  REC", "02/27/2026 21:07:00"),
        ("2O26-O2-27 2l:07:00", "2026-02-27 21:07:00"),
        ("AXON BWC 2l:07:00", "21:07:00"),
    ];
    for (text, expected) in accepted {
        assert_eq!(
            parse_clock_text(text).as_deref(),
            Some(expected),
            "reading `{text}`"
        );
    }

    let refused = [
        "21:07:06.400–21:07:19.000",
        "21:07:06-21:07:19",
        "2026-02-27 21:07:00 - 2026-02-27 21:07:19",
        "25:07:00",
        "21:60:00",
        "21:07:60",
        "2026-13-27 21:07:00",
        "2026-02-32 21:07:00",
        "13/27/2026 21:07:00",
        "REC 00 BATT 87%",
        "scene 2",
        "",
        "2107",
        "21:07",
    ];
    for text in refused {
        assert_eq!(parse_clock_text(text), None, "refusing `{text}`");
    }
}

#[test]
fn machine_clock_readings_cannot_arrive_verified() {
    let (_, case_id) = seeded();
    let identity = identity(case_id);
    let mut batch = scenes_to_batch(&identity, &one_scene()).expect("scenes");
    attach_clock_readings(
        &mut batch,
        &identity,
        &[reading(
            2,
            9_800,
            15_400,
            "2026-02-27 21:07:00",
            "2026-02-27 21:07:00",
        )],
        "tesseract@test",
    )
    .expect("attach");
    let segment = batch.sources[0]
        .segments
        .iter_mut()
        .find(|segment| segment.id.contains("-clk-"))
        .expect("clock segment");
    segment.content[0].extraction.review_state = ReviewState::Verified;

    let mut store = Store::in_memory().expect("store");
    DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let error = store
        .import_normalized(&batch)
        .expect_err("verified clock reading");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}

#[test]
fn two_scenes_can_carry_disagreeing_overlays() {
    let (mut store, case_id) = seeded();
    let identity = identity(case_id);
    let mut batch = scenes_to_batch(&identity, &two_scenes()).expect("scenes");
    attach_clock_readings(
        &mut batch,
        &identity,
        &[
            reading(1, 0, 9_800, "21:07:00", "21:07:00"),
            reading(2, 9_800, 15_400, "21:06:58", "21:06:58"),
        ],
        "tesseract@eng/psm7",
    )
    .expect("attach");

    let raw_times: Vec<Option<&str>> = clock_content(&batch)
        .iter()
        .map(|content| content.raw_time.as_deref())
        .collect();
    assert_eq!(
        raw_times,
        vec![Some("overlay 21:07:00"), Some("overlay 21:06:58")],
        "nothing reconciles two device clocks"
    );
    store.import_normalized(&batch).expect("import");
}

#[test]
fn a_reading_that_ends_before_it_starts_is_refused() {
    let (_, case_id) = seeded();
    let identity = identity(case_id);
    let mut batch = scenes_to_batch(&identity, &one_scene()).expect("scenes");
    let error = attach_clock_readings(
        &mut batch,
        &identity,
        &[reading(2, 15_400, 9_800, "21:07:00", "21:07:00")],
        "tesseract@test",
    )
    .expect_err("backwards reading");
    assert!(
        error.to_string().contains("ends before it starts"),
        "{error}"
    );

    let error = attach_clock_readings(
        &mut batch,
        &identity,
        &[reading(2, 9_800, 15_400, "REC", "  ")],
        "tesseract@test",
    )
    .expect_err("empty reading");
    assert!(error.to_string().contains("no timestamp"), "{error}");
}

#[test]
fn a_missing_ocr_binary_names_what_to_install() {
    let backend = TesseractCliBackend {
        binary: PathBuf::from("tesseract-does-not-exist-here"),
        ..TesseractCliBackend::default_local()
    };
    assert_eq!(backend.version(), "tesseract@eng/psm7");
    assert_eq!(OverlayBand::Top.crop_expression(), "crop=iw:ih*0.15:0:0");
    assert_eq!(
        OverlayBand::Bottom.crop_expression(),
        "crop=iw:ih*0.15:0:ih*0.85"
    );
    let error = backend
        .read(Path::new("no-such-still.jpg"))
        .expect_err("missing tesseract");
    assert!(error.to_string().contains("Install Tesseract"), "{error}");
}
