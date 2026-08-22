//! Supervised native worker process.

use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::frame::{read_frame, write_frame};
use crate::manifest::ModelManifest;
use crate::protocol::{RequestEnvelope, Response, ResponseEnvelope};
use crate::{Error, Result};

/// Resident worker used by broker scheduling.
pub trait Worker: Send {
    /// Run one request with one binary payload.
    fn request(&mut self, request: &RequestEnvelope, payload: &[u8]) -> Result<ResponseEnvelope>;
}

/// A fail-closed child process speaking the same framed protocol on stdio.
pub struct ProcessWorker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    failed: bool,
}

impl std::fmt::Debug for ProcessWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessWorker")
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl ProcessWorker {
    /// Spawn a manifest worker and require a successful real load/probe request.
    pub fn start(pack_root: &Path, manifest: &ModelManifest, request_id: u64) -> Result<Self> {
        let executable = manifest.worker_path(pack_root)?;
        let mut child = Command::new(&executable)
            .args(&manifest.worker_args)
            .current_dir(pack_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::Worker(format!("could not start {}: {error}", executable.display()))
            })?;
        let streams = (|| {
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| Error::Worker("worker stdin was not created".to_owned()))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| Error::Worker("worker stdout was not created".to_owned()))?;
            Ok((stdin, stdout))
        })();
        let (stdin, stdout) = match streams {
            Ok(streams) => streams,
            Err(error) => {
                terminate(&mut child);
                return Err(error);
            }
        };
        let mut worker = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            failed: false,
        };
        let load = RequestEnvelope::new(
            request_id,
            crate::protocol::Request::Load {
                model: manifest.id.clone(),
            },
        );
        let response = worker.request(&load, &[])?;
        match response.response {
            Response::Ok { .. } => Ok(worker),
            Response::Error { code, message } => Err(Error::Worker(format!(
                "load probe failed ({code}): {message}"
            ))),
        }
    }
}

impl Worker for ProcessWorker {
    fn request(&mut self, request: &RequestEnvelope, payload: &[u8]) -> Result<ResponseEnvelope> {
        if self.failed {
            return Err(Error::Worker(
                "worker is unavailable after a previous failure".to_owned(),
            ));
        }
        if let Err(error) = write_frame(&mut self.stdin, request, payload) {
            self.failed = true;
            return Err(error);
        }
        let framed = match read_frame(&mut self.stdout) {
            Ok(framed) => framed,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        let Some((response, response_payload)) = framed else {
            self.failed = true;
            let status = self.child.try_wait().ok().flatten();
            return Err(Error::Worker(format!(
                "worker exited during request {} ({status:?})",
                request.id
            )));
        };
        if !response_payload.is_empty() {
            self.failed = true;
            return Err(Error::Worker(
                "worker response unexpectedly carried a binary payload".to_owned(),
            ));
        }
        let response: ResponseEnvelope = response;
        if let Err(error) = response.validate(request.id) {
            self.failed = true;
            return Err(error);
        }
        Ok(response)
    }
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}

fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}
