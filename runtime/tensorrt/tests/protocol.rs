#![allow(missing_docs)]

use std::io::Cursor;

use evidence_trt::frame::{read_frame, write_frame};
use evidence_trt::{
    InputMetadata, Operation, Request, RequestEnvelope, Response, ResponseEnvelope, ResultBody,
    TranscriptSegment, TranscriptWord,
};

#[test]
fn frame_round_trip_preserves_typed_request_and_binary_input() {
    let request = RequestEnvelope::new(
        42,
        Request::Infer {
            model: "ocr-v1".to_owned(),
            revision: "export-1".to_owned(),
            operation: Operation::PageOcr,
            input: InputMetadata {
                width: Some(2),
                height: Some(2),
                channels: Some(1),
                language: Some("eng".to_owned()),
                ..InputMetadata::default()
            },
        },
    );
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &request, &[1, 2, 3, 4]).unwrap();
    let (decoded, payload): (RequestEnvelope, Vec<u8>) =
        read_frame(&mut Cursor::new(bytes)).unwrap().unwrap();
    assert_eq!(decoded, request);
    assert_eq!(payload, vec![1, 2, 3, 4]);
}

#[test]
fn incompatible_protocol_is_refused_before_dispatch() {
    let mut request = RequestEnvelope::new(1, Request::Health);
    request.version += 1;
    assert!(request.validate().is_err());
}

#[test]
fn v2_transcript_round_trip_preserves_words_and_anonymous_speakers() {
    let response = ResponseEnvelope::new(
        7,
        Response::Ok {
            result: ResultBody::Transcript {
                segments: vec![TranscriptSegment {
                    start_ms: 100,
                    end_ms: 900,
                    text: "hello".to_owned(),
                    confidence: Some(0.9),
                    words: vec![TranscriptWord {
                        start_ms: 120,
                        end_ms: 480,
                        text: "hello".to_owned(),
                        confidence: Some(0.95),
                    }],
                    speaker: Some("SPEAKER_01".to_owned()),
                }],
            },
        },
    );
    let encoded = serde_json::to_vec(&response).unwrap();
    let decoded: ResponseEnvelope = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, response);
    decoded.validate(7).unwrap();
}
