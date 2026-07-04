//! Length-prefixed bincode framing over a raw `TcpStream`. Same codec
//! is used by the coordinator side (talking to a worker) and the
//! worker side (replying to the coordinator) — no asymmetry, no HTTP.
//!
//! Frame layout:
//!
//! ```text
//! [u32 length, little-endian] [bincode payload]
//! ```
//!
//! `length` is the byte count of the bincode payload only — header is
//! 4 bytes on the wire. A `MAX_FRAME_BYTES` cap protects the reader
//! from a malicious / corrupted peer trying to allocate an absurd
//! buffer up-front.

use bincode::config::Configuration;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 64 MiB hard cap per frame. A batch of ~22 players × ~200 bytes ×
/// many matches plus a healthy padding for any future protocol growth
/// fits comfortably under this. A real run shouldn't get close — the
/// cap is purely a corrupt-input fuse.
pub const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// Static bincode encoder/decoder configuration. Standard (little-endian,
/// variable-length ints) keeps payloads compact and is the default
/// shape `bincode::serde` produces.
fn config() -> Configuration {
    bincode::config::standard()
}

pub struct Frame;

impl Frame {
    /// Encode `msg` with bincode and write it framed to the stream.
    pub async fn write<T>(stream: &mut TcpStream, msg: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let payload = bincode::serde::encode_to_vec(msg, config())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = payload.len();
        if len > MAX_FRAME_BYTES as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame {} bytes exceeds {}", len, MAX_FRAME_BYTES),
            ));
        }
        stream.write_all(&(len as u32).to_le_bytes()).await?;
        stream.write_all(&payload).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Read one framed bincode message off the stream.
    pub async fn read<T>(stream: &mut TcpStream) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await?;
        let len = u32::from_le_bytes(len_bytes);
        if len > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame {} bytes exceeds {}", len, MAX_FRAME_BYTES),
            ));
        }
        let mut buf = vec![0u8; len as usize];
        stream.read_exact(&mut buf).await?;
        let (msg, _) = bincode::serde::decode_from_slice(&buf, config())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(msg)
    }
}
