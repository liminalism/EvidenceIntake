//! Local-socket broker client.

use std::io::{BufReader, BufWriter};
use std::sync::atomic::{AtomicU64, Ordering};

use interprocess::TryClone;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericNamespaced, Stream};

use crate::frame::{read_frame, write_frame};
use crate::protocol::{Request, RequestEnvelope, Response, ResponseEnvelope, ResultBody};
use crate::{Error, Result};

/// Synchronous client for the per-user local broker.
pub struct Client {
    reader: BufReader<Stream>,
    writer: BufWriter<Stream>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    /// Connect to a namespaced local endpoint. Windows uses a named pipe.
    pub fn connect(endpoint: &str) -> Result<Self> {
        let name = endpoint
            .to_ns_name::<GenericNamespaced>()
            .map_err(|error| Error::Request(format!("invalid broker endpoint: {error}")))?;
        let stream = Stream::connect(name)
            .map_err(|error| Error::Request(format!("connect to broker: {error}")))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            reader,
            writer: BufWriter::new(stream),
            next_id: AtomicU64::new(1),
        })
    }

    /// Send one request and return its typed successful body.
    pub fn request(&mut self, request: Request, payload: &[u8]) -> Result<ResultBody> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).max(1);
        let envelope = RequestEnvelope::new(id, request);
        write_frame(&mut self.writer, &envelope, payload)?;
        let Some((response, response_payload)) = read_frame(&mut self.reader)? else {
            return Err(Error::Request(
                "broker closed the connection before responding".to_owned(),
            ));
        };
        if !response_payload.is_empty() {
            return Err(Error::IncompatibleProtocol(
                "broker response carried an unexpected binary payload".to_owned(),
            ));
        }
        let response: ResponseEnvelope = response;
        response.validate(id)?;
        match response.response {
            Response::Ok { result } => Ok(result),
            Response::Error { code, message } => Err(Error::Request(format!("{code}: {message}"))),
        }
    }
}
