//! Local named-pipe/socket server loop.

use std::io::{BufReader, BufWriter};
use std::sync::{Arc, Mutex};

use interprocess::TryClone;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions};

use crate::broker::Broker;
use crate::frame::{read_frame, write_frame};
use crate::protocol::RequestEnvelope;
use crate::{Error, Result};

/// Serve a broker on a namespaced endpoint until the listener fails.
pub fn serve(endpoint: &str, broker: Broker) -> Result<()> {
    let name = endpoint
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| Error::Request(format!("invalid broker endpoint: {error}")))?;
    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .map_err(|error| Error::Request(format!("create broker endpoint: {error}")))?;
    let broker = Arc::new(Mutex::new(broker));
    loop {
        let stream = listener
            .accept()
            .map_err(|error| Error::Request(format!("accept broker client: {error}")))?;
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || {
            let Ok(reader_stream) = stream.try_clone() else {
                return;
            };
            let mut reader = BufReader::new(reader_stream);
            let mut writer = BufWriter::new(stream);
            loop {
                let frame: Result<Option<(RequestEnvelope, Vec<u8>)>> = read_frame(&mut reader);
                let Ok(Some((request, payload))) = frame else {
                    break;
                };
                let response = match broker.lock() {
                    Ok(mut broker) => broker.handle(&request, &payload),
                    Err(_) => break,
                };
                if write_frame(&mut writer, &response, &[]).is_err() {
                    break;
                }
            }
        });
    }
}
