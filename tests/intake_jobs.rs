//! Persistent intake queue and atomic manifest import.

#![allow(missing_docs)]

use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterArtifact, AdapterJobRequest, AdapterProfile,
    AdapterResultManifest, AdapterSourceLocation, ModelRef, TemporalRelationArg,
};
use evidence_intake::{
    CaseId, IntakeJobState, NewIntakeJob, NormalizedBatch, NormalizedSource, ProposedCase,
    SourceKind, Store, TemporalRelation,
};
use sha2::{Digest, Sha256};

fn open_case(store: &mut Store) -> (CaseId, String) {
    let opened = store
        .open_case(&ProposedCase {
            id: Some("case-intake".to_owned()),
            name: "State v. Queue".to_owned(),
            reference: None,
            jurisdiction: None,
            production: Some("Production 1".to_owned()),
        })
        .expect("open case");
    (CaseId(opened.id), opened.production.id)
}

fn request(root: &std::path::Path, production: &str, job: &str) -> AdapterJobRequest {
    let original = root.join("original.wav");
    std::fs::write(&original, b"synthetic original").expect("write original");
    let bytes = std::fs::read(&original).expect("read original");
    AdapterJobRequest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: job.to_owned(),
        case_id: "case-intake".to_owned(),
        production_id: production.to_owned(),
        source_id: format!("source-{job}"),
        original_path: original,
        original_sha256: format!("{:x}", Sha256::digest(&bytes)),
        original_byte_length: bytes.len() as u64,
        logical_name: "original.wav".to_owned(),
        temporal_relation: TemporalRelationArg::Unknown,
        artifacts_dir: root.join(job).join("attempt-0001"),
        profile: AdapterProfile::Audio {
            broker_endpoint: "evidence-trt".to_owned(),
            whisper: ModelRef {
                id: "whisper".to_owned(),
                revision: "revision".to_owned(),
            },
            language: "en".to_owned(),
            phone_band: false,
            level_split: false,
            gap_ms: 2_000,
        },
    }
}

fn batch(request: &AdapterJobRequest) -> NormalizedBatch {
    NormalizedBatch {
        case_id: CaseId(request.case_id.clone()),
        sources: vec![NormalizedSource {
            id: request.source_id.clone(),
            production_id: request.production_id.clone(),
            logical_name: request.logical_name.clone(),
            media_type: "audio/wav".to_owned(),
            source_kind: SourceKind::Audio,
            temporal_relation: TemporalRelation::Unknown,
            sha256: request.original_sha256.clone(),
            byte_length: request.original_byte_length,
            segments: Vec::new(),
        }],
        edges: Vec::new(),
    }
}

fn enqueue(store: &mut Store, request: &AdapterJobRequest) {
    store
        .enqueue_intake_job(&NewIntakeJob {
            request_json: serde_json::to_string_pretty(request).expect("request JSON"),
        })
        .expect("enqueue");
}

#[test]
fn result_manifest_commits_source_location_artifacts_and_terminal_state_together() {
    let temp = tempfile::tempdir().expect("temp");
    let database = temp.path().join("case.sqlite");
    let mut store = Store::open(&database).expect("store");
    let (_, production) = open_case(&mut store);
    let request = request(temp.path(), &production, "job-complete");
    enqueue(&mut store, &request);
    assert_eq!(
        store
            .claim_next_intake_job()
            .expect("claim")
            .expect("job")
            .state,
        IntakeJobState::Running
    );
    store
        .mark_intake_importing(&request.job_id)
        .expect("importing");
    std::fs::create_dir_all(&request.artifacts_dir).expect("artifact directory");
    let batch_path = request.artifacts_dir.join("normalized-batch.json");
    std::fs::write(
        &batch_path,
        serde_json::to_vec_pretty(&batch(&request)).expect("batch JSON"),
    )
    .expect("write batch");
    let manifest = AdapterResultManifest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        batch_path: batch_path.clone(),
        keyframe_index_path: None,
        source_locations: vec![AdapterSourceLocation {
            source_id: request.source_id.clone(),
            path: request.original_path.clone(),
            sha256: request.original_sha256.clone(),
            byte_length: request.original_byte_length,
        }],
        artifacts: vec![AdapterArtifact {
            kind: "normalized_batch".to_owned(),
            path: batch_path,
            sha256: None,
        }],
    };
    store
        .commit_intake_result(&request.job_id, &manifest)
        .expect("atomic commit");

    assert_eq!(
        store.intake_job(&request.job_id).expect("job").state,
        IntakeJobState::Completed
    );
    assert_eq!(
        store
            .source_location(&request.source_id)
            .expect("location")
            .path,
        request.original_path
    );
    assert!(
        !store
            .intake_artifacts(&request.job_id)
            .expect("artifacts")
            .is_empty()
    );
}

#[test]
fn duplicate_preflight_and_restart_recovery_happen_before_model_work() {
    let temp = tempfile::tempdir().expect("temp");
    let database = temp.path().join("case.sqlite");
    let mut store = Store::open(&database).expect("store");
    let (_, production) = open_case(&mut store);
    let first = request(temp.path(), &production, "job-first");
    enqueue(&mut store, &first);

    let mut duplicate = first.clone();
    duplicate.job_id = "job-duplicate".to_owned();
    duplicate.source_id = "source-duplicate".to_owned();
    duplicate.artifacts_dir = temp.path().join("job-duplicate").join("attempt-0001");
    let error = store
        .enqueue_intake_job(&NewIntakeJob {
            request_json: serde_json::to_string(&duplicate).expect("JSON"),
        })
        .expect_err("duplicate original");
    assert!(error.to_string().contains("source hash"), "{error}");

    store.claim_next_intake_job().expect("claim").expect("job");
    drop(store);
    let mut store = Store::open(&database).expect("reopen");
    assert_eq!(store.recover_interrupted_intake_jobs().expect("recover"), 1);
    assert_eq!(
        store.intake_job(&first.job_id).expect("job").state,
        IntakeJobState::Interrupted
    );
}

#[test]
fn retry_uses_a_new_attempt_directory_and_relink_requires_the_same_bytes() {
    let temp = tempfile::tempdir().expect("temp");
    let mut store = Store::open(temp.path().join("case.sqlite")).expect("store");
    let (_, production) = open_case(&mut store);
    let request = request(temp.path(), &production, "job-retry");
    enqueue(&mut store, &request);
    store.claim_next_intake_job().expect("claim").expect("job");
    store
        .fail_intake_job(&request.job_id, "synthetic crash")
        .expect("fail");

    let mut retry = request.clone();
    retry.artifacts_dir = temp.path().join("job-retry").join("attempt-0002");
    let retried = store
        .retry_intake_job(
            &request.job_id,
            &serde_json::to_string_pretty(&retry).expect("retry JSON"),
        )
        .expect("retry");
    assert_eq!(retried.attempt, 2);
    assert_eq!(retried.artifact_dir, retry.artifacts_dir);

    // Relinking is exercised after a direct import, independent of queue state.
    let mut direct = request.clone();
    direct.source_id = "source-direct".to_owned();
    store
        .import_normalized(&batch(&direct))
        .expect("direct import");
    let moved = temp.path().join("moved.wav");
    std::fs::copy(&direct.original_path, &moved).expect("move copy");
    assert_eq!(
        store
            .relink_source(&direct.source_id, &moved)
            .expect("matching relink")
            .path,
        moved
    );
    let changed = temp.path().join("changed.wav");
    std::fs::write(&changed, b"changed").expect("changed file");
    assert!(store.relink_source(&direct.source_id, &changed).is_err());
}

#[test]
fn escaped_artifact_path_is_rejected_without_partial_import() {
    let temp = tempfile::tempdir().expect("temp");
    let mut store = Store::open(temp.path().join("case.sqlite")).expect("store");
    let (_, production) = open_case(&mut store);
    let request = request(temp.path(), &production, "job-escape");
    enqueue(&mut store, &request);
    store.claim_next_intake_job().expect("claim").expect("job");
    store
        .mark_intake_importing(&request.job_id)
        .expect("importing");
    std::fs::create_dir_all(&request.artifacts_dir).expect("artifact directory");
    let escaped = temp.path().join("escaped.json");
    std::fs::write(
        &escaped,
        serde_json::to_vec(&batch(&request)).expect("batch JSON"),
    )
    .expect("write batch");
    let manifest = AdapterResultManifest {
        schema_version: ADAPTER_PROTOCOL_VERSION,
        job_id: request.job_id.clone(),
        batch_path: escaped,
        keyframe_index_path: None,
        source_locations: vec![AdapterSourceLocation {
            source_id: request.source_id.clone(),
            path: request.original_path.clone(),
            sha256: request.original_sha256.clone(),
            byte_length: request.original_byte_length,
        }],
        artifacts: Vec::new(),
    };
    assert!(
        store
            .commit_intake_result(&request.job_id, &manifest)
            .is_err()
    );
    assert!(store.source_location(&request.source_id).is_err());
    assert_eq!(
        store.intake_job(&request.job_id).expect("job").state,
        IntakeJobState::Importing
    );
}
