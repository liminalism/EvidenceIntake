//! Background process coordinator for durable adapter jobs.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use evidence_adapter_protocol::{AdapterArtifact, AdapterEvent, AdapterResultManifest};

use crate::{Error, IntakeJob, Result, Store};

enum CoordinatorCommand {
    Wake,
}

/// Cloneable signal handle for one database's local queue worker.
#[derive(Clone)]
pub struct IntakeCoordinator {
    sender: Sender<CoordinatorCommand>,
}

impl std::fmt::Debug for IntakeCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("IntakeCoordinator").finish()
    }
}

impl IntakeCoordinator {
    /// Recover stale states and start the one-job-at-a-time coordinator.
    pub fn start(database: impl AsRef<Path>) -> Result<Self> {
        let database = database.as_ref().to_path_buf();
        let mut store = Store::open(&database)?;
        store.recover_interrupted_intake_jobs()?;
        drop(store);

        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("evidence-intake-coordinator".to_owned())
            .spawn(move || {
                let Ok(mut store) = Store::open(&database) else {
                    return;
                };
                loop {
                    match drain_queue(&mut store) {
                        Ok(()) => {}
                        Err(error) => eprintln!("intake coordinator: {error}"),
                    }
                    match receiver.recv_timeout(Duration::from_secs(2)) {
                        Ok(CoordinatorCommand::Wake) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(Error::Io)?;
        Ok(Self { sender })
    }

    /// Wake the worker after one or more jobs are queued.
    pub fn wake(&self) {
        let _ = self.sender.send(CoordinatorCommand::Wake);
    }
}

fn drain_queue(store: &mut Store) -> Result<()> {
    while let Some(job) = store.claim_next_intake_job()? {
        if let Err(error) = execute_job(store, &job) {
            let log = job.artifact_dir.join("adapter.log");
            if log.is_file() {
                let _ = store.register_intake_artifact(&job.id, "log", &log, None);
            }
            let _ = store.fail_intake_job(&job.id, &error.to_string());
        }
    }
    Ok(())
}

fn execute_job(store: &mut Store, job: &IntakeJob) -> Result<()> {
    std::fs::create_dir_all(&job.artifact_dir)?;
    let request_path = job.artifact_dir.join("request.json");
    write_atomic(&request_path, job.request_json.as_bytes())?;
    let log_path = job.artifact_dir.join("adapter.log");
    let log = File::create(&log_path)?;
    let executable = adapter_executable(&job.modality)?;
    let mut child = Command::new(&executable)
        .arg("job")
        .arg("--request")
        .arg(&request_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| {
            Error::InvalidIntake(format!(
                "could not start {} for job `{}`: {error}",
                executable.display(),
                job.id
            ))
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        Error::InvalidIntake(format!(
            "adapter stdout was unavailable for job `{}`",
            job.id
        ))
    })?;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let event: AdapterEvent = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::InvalidIntake(format!(
                    "adapter emitted malformed progress JSON: {error}"
                )));
            }
        };
        if event.job_id != job.id {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidIntake(format!(
                "adapter progress named job `{}`, expected `{}`",
                event.job_id, job.id
            )));
        }
        if let Err(error) = store.update_intake_progress(&event) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    let status = child.wait()?;
    store.register_intake_artifact(&job.id, "log", &log_path, None)?;
    if !status.success() {
        let diagnostic = std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("adapter exited without a diagnostic")
            .to_owned();
        return Err(Error::InvalidIntake(format!(
            "adapter exited with {status}: {diagnostic}"
        )));
    }
    let result_path = job.artifact_dir.join("result.json");
    let mut manifest: AdapterResultManifest =
        serde_json::from_slice(&std::fs::read(&result_path)?)?;
    manifest.artifacts.push(AdapterArtifact {
        kind: "log".to_owned(),
        path: log_path,
        sha256: None,
    });
    store.mark_intake_importing(&job.id)?;
    store.commit_intake_result(&job.id, &manifest)
}

fn adapter_executable(modality: &str) -> Result<PathBuf> {
    let (environment, filename) = match modality {
        "document" => ("EVIDENCE_DOCUMENT_ADAPTER", "evidence-document"),
        "audio" => ("EVIDENCE_AUDIO_ADAPTER", "evidence-audio"),
        "video" => ("EVIDENCE_VIDEO_ADAPTER", "evidence-video"),
        other => {
            return Err(Error::InvalidIntake(format!(
                "unknown intake modality `{other}`"
            )));
        }
    };
    if let Some(path) = std::env::var_os(environment).map(PathBuf::from) {
        return Ok(path);
    }
    let directory = std::env::current_exe()?
        .parent()
        .ok_or_else(|| Error::InvalidIntake("application executable has no directory".to_owned()))?
        .to_path_buf();
    Ok(directory.join(if cfg!(windows) {
        format!("{filename}.exe")
    } else {
        filename.to_owned()
    }))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
