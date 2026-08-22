//! Length-framed JSON control messages with one binary attachment.

use std::io::{Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Error, Result};

const MAX_HEADER_BYTES: usize = 4 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Write one protocol frame.
pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    header: &T,
    payload: &[u8],
) -> Result<()> {
    let json = serde_json::to_vec(header)?;
    if json.len() > MAX_HEADER_BYTES || payload.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::FrameTooLarge);
    }
    let header_len = u32::try_from(json.len()).map_err(|_| Error::FrameTooLarge)?;
    let payload_len = u64::try_from(payload.len()).map_err(|_| Error::FrameTooLarge)?;
    writer.write_all(&header_len.to_le_bytes())?;
    writer.write_all(&payload_len.to_le_bytes())?;
    writer.write_all(&json)?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

/// Read one protocol frame. EOF before a new header returns `Ok(None)`.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<Option<(T, Vec<u8>)>> {
    let mut header_len = [0_u8; 4];
    if !read_exact_or_eof(reader, &mut header_len)? {
        return Ok(None);
    }
    let mut payload_len = [0_u8; 8];
    reader.read_exact(&mut payload_len)?;
    let header_len = u32::from_le_bytes(header_len) as usize;
    let payload_len =
        usize::try_from(u64::from_le_bytes(payload_len)).map_err(|_| Error::FrameTooLarge)?;
    if header_len > MAX_HEADER_BYTES || payload_len > MAX_PAYLOAD_BYTES {
        return Err(Error::FrameTooLarge);
    }
    let mut json = vec![0_u8; header_len];
    reader.read_exact(&mut json)?;
    let header = serde_json::from_slice(&json)?;
    let mut payload = vec![0_u8; payload_len];
    reader.read_exact(&mut payload)?;
    Ok(Some((header, payload)))
}

fn read_exact_or_eof(reader: &mut impl Read, buffer: &mut [u8]) -> std::io::Result<bool> {
    let mut offset = 0;
    while offset < buffer.len() {
        match reader.read(&mut buffer[offset..])? {
            0 if offset == 0 => return Ok(false),
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "partial frame header",
                ));
            }
            read => offset += read,
        }
    }
    Ok(true)
}
