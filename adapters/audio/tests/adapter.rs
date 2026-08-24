//! Adapter tests named as assertions about the domain rules.

#![allow(missing_docs)]

use std::f32::consts::PI;
use std::io::Write;

use evidence_audio::{
    ChannelSide, ChannelSplit, DecodedAudio, EXTRACTOR_CHANNEL, EXTRACTOR_WHISPERX, MappingOptions,
    SourceIdentity, WhisperxSegment, WhisperxTranscript, WhisperxWord, channel_split, decode_wav,
    file_rms, format_locator, level_split, prepare_working_copy, transcript_to_batch, write_wav,
};
use evidence_audio::{MediaClass, classify, ffmpeg_available, media_type, open_media};
use evidence_intake::{
    CaseId, ContentKind, DemoFixture, EdgeKind, ReviewState, SourceKind, Store, TemporalRelation,
};
use hound::{SampleFormat, WavSpec, WavWriter};

fn identity(case_id: CaseId) -> SourceIdentity {
    SourceIdentity {
        case_id,
        production_id: "prod-01".to_owned(),
        source_id: "adapter-911".to_owned(),
        logical_name: "911 call.wav".to_owned(),
        media_type: "audio/wav".to_owned(),
        temporal_relation: TemporalRelation::Contemporaneous,
        source_kind: SourceKind::Audio,
        sha256: "ab".repeat(32),
        byte_length: 3_280_401,
    }
}

fn sample_transcript() -> WhisperxTranscript {
    WhisperxTranscript {
        language: Some("en".to_owned()),
        segments: vec![
            WhisperxSegment {
                start: 8.2,
                end: 31.6,
                text: " It just happened. I was sitting at the light. ".to_owned(),
                words: vec![WhisperxWord {
                    word: "It".to_owned(),
                    start: Some(8.2),
                    end: Some(8.4),
                    score: Some(0.91),
                }],
                speaker: None,
                avg_logprob: None,
            },
            WhisperxSegment {
                start: 34.0,
                end: 38.5,
                text: "No, I do not think I am hurt.".to_owned(),
                words: vec![],
                speaker: Some("SPEAKER_00".to_owned()),
                avg_logprob: Some(-0.223),
            },
        ],
    }
}

fn seeded() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    (store, case_id)
}

#[test]
fn a_transcript_line_enters_suggested_and_names_whisperx() {
    let (mut store, case_id) = seeded();
    let batch = transcript_to_batch(
        &identity(case_id.clone()),
        &sample_transcript(),
        &MappingOptions::new("whisperx@large-v3"),
    )
    .expect("map");

    let source = &batch.sources[0];
    assert_eq!(source.source_kind, SourceKind::Audio);
    assert_eq!(source.media_type, "audio/wav");
    let first = &source.segments[0];
    assert_eq!(first.locator, "00:00:08.200–00:00:31.600");
    assert_eq!(first.start_ms, Some(8_200));
    assert_eq!(first.end_ms, Some(31_600));
    let statement = first
        .content
        .iter()
        .find(|item| item.kind == ContentKind::Statement)
        .expect("statement");
    assert_eq!(statement.extraction.extractor, EXTRACTOR_WHISPERX);
    assert_eq!(statement.extraction.version, "whisperx@large-v3");
    assert!(statement.extraction.machine_generated);
    assert_eq!(statement.extraction.review_state, ReviewState::Suggested);
    assert!(statement.speaker_entity_id.is_none());
    assert!(statement.normalized_start.is_none());
    assert!(statement.time_basis.is_none());

    let speaker = batch.edges.first().expect("diarization relationship");
    assert_eq!(speaker.relation, EdgeKind::SpeakerCandidate);
    assert_eq!(speaker.from_id, "adapter-911-stmt-0001");
    assert_eq!(speaker.to_id, "adapter-911-spk-0001");
    assert_eq!(speaker.extraction.review_state, ReviewState::Suggested);
    assert!(speaker.rationale.contains("not a person identification"));

    store.import_normalized(&batch).expect("import");
    let hits = store
        .search(&case_id, "sitting at the light", 10)
        .expect("search");
    assert!(
        hits.iter()
            .any(|hit| hit.locator == "00:00:08.200–00:00:31.600"),
        "imported words must be findable with the original locator: {hits:?}"
    );
}

#[test]
fn locators_address_the_original_timeline_not_the_working_copy() {
    let locator = format_locator(8_200, 31_600);
    assert_eq!(locator, "00:00:08.200–00:00:31.600");
    // Cleanup resamples but never time-stretches; the mapper copies WhisperX
    // seconds through as original milliseconds.
    let audio = sine(16_000, 1, 0.25, 0.2, false);
    let working = prepare_working_copy(&audio, false).expect("clean");
    assert_eq!(working.sample_rate, 48_000);
    let original_ms = audio.duration_ms();
    let working_ms = working.duration_ms();
    let delta = original_ms.abs_diff(working_ms);
    assert!(
        delta <= 30,
        "working copy duration {working_ms} ms drifted {delta} ms from original {original_ms} ms"
    );
}

#[test]
fn channel_split_is_an_observation_not_a_person() {
    let audio = left_only_burst(48_000, 200);
    let split = channel_split(&audio, 0, audio.duration_ms()).expect("left-dominant");
    assert_eq!(split.side, ChannelSide::Left);
    assert!(split.confidence > 0.0 && split.confidence <= 1.0);

    let (mut store, case_id) = seeded();
    let mut options = MappingOptions::new("whisperx@test");
    options.channel_splits = vec![ChannelSplit {
        start_ms: 8_200,
        end_ms: 31_600,
        ..split
    }];
    let batch = transcript_to_batch(&identity(case_id.clone()), &sample_transcript(), &options)
        .expect("map");
    let observation = batch.sources[0].segments[0]
        .content
        .iter()
        .find(|item| item.kind == ContentKind::Observation)
        .expect("channel observation");
    assert_eq!(observation.extraction.extractor, EXTRACTOR_CHANNEL);
    assert!(observation.speaker_entity_id.is_none());
    assert!(observation.text.contains("left"));
    assert!(!observation.text.to_lowercase().contains("person"));

    store.import_normalized(&batch).expect("import");
}

#[test]
fn dual_mono_does_not_invent_two_speakers() {
    let audio = sine(48_000, 2, 0.3, 0.2, true);
    assert!(
        channel_split(&audio, 0, audio.duration_ms()).is_none(),
        "identical channels are dual-mono, not two speakers"
    );
}

#[test]
fn machine_audio_cannot_arrive_verified() {
    let (_, case_id) = seeded();
    let mut batch = transcript_to_batch(
        &identity(case_id),
        &sample_transcript(),
        &MappingOptions::new("whisperx@test"),
    )
    .expect("map");
    batch.sources[0].segments[0].content[0]
        .extraction
        .review_state = ReviewState::Verified;
    let mut store = Store::in_memory().expect("store");
    DemoFixture::VehicleStop.seed(&mut store).expect("seed");
    let error = store
        .import_normalized(&batch)
        .expect_err("verified machine audio");
    assert!(
        error.to_string().contains("must enter as suggested"),
        "{error}"
    );
}

#[test]
fn an_unaligned_interval_is_not_a_recording_gap() {
    let (_, case_id) = seeded();
    let batch = transcript_to_batch(
        &identity(case_id),
        &sample_transcript(),
        &MappingOptions::new("whisperx@test"),
    )
    .expect("map");
    let interval = batch.sources[0]
        .segments
        .iter()
        .find(|segment| {
            segment
                .content
                .iter()
                .any(|item| item.extraction.extractor == "asr-no-speech")
        })
        .expect("unaligned interval between 31.6s and 34.0s");
    assert_eq!(interval.start_ms, Some(31_600));
    assert_eq!(interval.end_ms, Some(34_000));
    let observation = &interval.content[0];
    assert_eq!(observation.kind, ContentKind::Observation);
    assert!(
        observation
            .text
            .contains("does not establish recording loss")
    );
    assert!(
        batch.sources[0]
            .segments
            .iter()
            .flat_map(|segment| &segment.content)
            .all(|item| item.kind != ContentKind::RecordingGap)
    );
}

#[test]
fn cleaned_audio_is_not_imported_as_the_source() {
    let (mut store, case_id) = seeded();
    let original = sine(16_000, 1, 0.4, 0.15, false);
    let working = prepare_working_copy(&original, true).expect("clean");
    assert_ne!(
        working.samples, original.samples,
        "the working copy must actually be a copy"
    );
    let batch = transcript_to_batch(
        &SourceIdentity {
            sha256: original.sha256.clone(),
            byte_length: original.byte_length,
            ..identity(case_id.clone())
        },
        &sample_transcript(),
        &MappingOptions::new("whisperx@test"),
    )
    .expect("map");
    assert_eq!(batch.sources[0].sha256, original.sha256);
    assert_eq!(batch.sources[0].byte_length, original.byte_length);
    assert_eq!(batch.sources.len(), 1);
    store.import_normalized(&batch).expect("import");
    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    assert!(
        ledger.iter().any(|item| item.source == "911 call.wav"),
        "only the original name is on the ledger: {ledger:?}"
    );
    assert!(
        ledger
            .iter()
            .all(|item| !item.source.contains("working") && !item.source.contains("denois")),
        "cleaned audio must not appear as a source: {ledger:?}"
    );
}

#[test]
fn empty_and_truncated_files_are_refused() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let empty = dir.path().join("empty.wav");
    std::fs::write(&empty, []).expect("write");
    let error = decode_wav(&empty).expect_err("empty");
    assert!(error.to_string().contains("empty"), "{error}");

    let truncated = dir.path().join("truncated.wav");
    let mut file = std::fs::File::create(&truncated).expect("create");
    file.write_all(b"RIFF").expect("write");
    drop(file);
    let error = decode_wav(&truncated).expect_err("truncated");
    assert!(error.to_string().contains("could not decode"), "{error}");
}

#[test]
fn level_split_stays_off_when_levels_are_flat() {
    let audio = sine(48_000, 1, 0.4, 0.2, false);
    let rms = file_rms(&audio);
    assert!(level_split(&audio, 0, audio.duration_ms(), rms).is_none());
}

fn sine(rate: u32, channels: u16, seconds: f32, amplitude: f32, dual: bool) -> DecodedAudio {
    let frames = (seconds * rate as f32) as usize;
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for frame in 0..frames {
        let value = (2.0 * PI * 440.0 * frame as f32 / rate as f32).sin() * amplitude;
        samples.push(value);
        if channels > 1 {
            samples.push(if dual { value } else { 0.0 });
        }
    }
    DecodedAudio {
        sha256: "cd".repeat(32),
        byte_length: 64,
        media_type: "audio/wav".to_owned(),
        sample_rate: rate,
        channels,
        samples,
    }
}

fn left_only_burst(rate: u32, duration_ms: u64) -> DecodedAudio {
    sine(rate, 2, duration_ms as f32 / 1_000.0, 0.4, false)
}

/// Writes and re-reads a wav so decode / hash paths are exercised.
#[test]
fn decode_round_trip_preserves_channel_count() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let path = dir.path().join("round.wav");
    let spec = WavSpec {
        channels: 2,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&path, spec).expect("writer");
    for _ in 0..1_600 {
        writer.write_sample(8_000_i16).expect("L");
        writer.write_sample(0_i16).expect("R");
    }
    writer.finalize().expect("finalize");
    let decoded = decode_wav(&path).expect("decode");
    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.sample_rate, 16_000);
    assert_eq!(decoded.sha256.len(), 64);
    assert!(decoded.byte_length > 0);
    let split = channel_split(&decoded, 0, decoded.duration_ms()).expect("left");
    assert_eq!(split.side, ChannelSide::Left);

    let out = dir.path().join("copy.wav");
    write_wav(&out, &decoded).expect("write");
    let again = decode_wav(&out).expect("redecode");
    assert_eq!(again.channels, 2);
}

#[test]
fn a_wav_and_a_video_are_classified_apart() {
    assert_eq!(classify(std::path::Path::new("911.wav")), MediaClass::Wav);
    assert_eq!(
        classify(std::path::Path::new("bodycam.mp4")),
        MediaClass::Video
    );
    assert_eq!(
        classify(std::path::Path::new("interview.m4a")),
        MediaClass::AudioContainer
    );
    assert_eq!(media_type(std::path::Path::new("bodycam.mp4")), "video/mp4");
}

#[test]
fn a_video_soundtrack_is_transcribed_as_video_not_as_a_new_source() {
    if !ffmpeg_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmpdir");
    let wav = dir.path().join("track.wav");
    let spec = WavSpec {
        channels: 2,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&wav, spec).expect("writer");
    for _ in 0..3_200 {
        writer.write_sample(6_000_i16).expect("L");
        writer.write_sample(0_i16).expect("R");
    }
    writer.finalize().expect("finalize");

    let mp4 = dir.path().join("bodycam.mp4");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("color=c=black:s=64x64:d=0.2")
        .args(["-i"])
        .arg(&wav)
        .args(["-shortest", "-c:v", "mpeg4", "-c:a", "aac"])
        .arg(&mp4)
        .status()
        .expect("ffmpeg remux");
    if !status.success() {
        return;
    }

    let opened = open_media(&mp4).expect("open video");
    assert_eq!(opened.source_kind, SourceKind::Video);
    assert_eq!(opened.media_type, "video/mp4");
    assert_eq!(opened.sha256.len(), 64);
    assert!(opened.audio.frames() > 0);

    let (mut store, case_id) = seeded();
    let batch = transcript_to_batch(
        &SourceIdentity {
            source_kind: SourceKind::Video,
            media_type: "video/mp4".to_owned(),
            logical_name: "bodycam.mp4".to_owned(),
            sha256: opened.sha256.clone(),
            byte_length: opened.byte_length,
            ..identity(case_id.clone())
        },
        &sample_transcript(),
        &MappingOptions::new("whisper.cpp@test"),
    )
    .expect("map");
    assert_eq!(batch.sources[0].source_kind, SourceKind::Video);
    assert_eq!(batch.sources[0].sha256, opened.sha256);
    store.import_normalized(&batch).expect("import");
}
