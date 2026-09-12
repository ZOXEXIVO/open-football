//! Length-prefixed wincode framing over a raw `TcpStream`. Same codec
//! is used by the coordinator side (talking to a worker) and the
//! worker side (replying to the coordinator) — no asymmetry, no HTTP.
//!
//! Frame layout:
//!
//! ```text
//! [u32 length, little-endian] [wincode payload]
//! ```
//!
//! `length` is the byte count of the wincode payload only — header is
//! 4 bytes on the wire. A `MAX_FRAME_BYTES` cap protects the reader
//! from a malicious / corrupted peer trying to allocate an absurd
//! buffer up-front.
//!
//! The payload is produced by `wincode` through its serde bridge
//! (`serde_wincode::SerdeCompat`), so every message type keeps its plain
//! `Serialize` / `Deserialize` derives — the same ones `core` already
//! carries for JSON — and nothing in the protocol needs a second schema.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_wincode::SerdeCompat;
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use wincode::config::{
    Configuration, Deserialize as WincodeDeserialize, Serialize as WincodeSerialize,
};
use wincode::int_encoding::{LittleEndian, VarInt};
use wincode::len::BincodeLen;

/// 64 MiB hard cap per frame. A batch of ~22 players × ~200 bytes ×
/// many matches plus a healthy padding for any future protocol growth
/// fits comfortably under this. A real run shouldn't get close — the
/// cap is purely a corrupt-input fuse.
pub const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// The codec every frame is encoded with. Little-endian, variable-length
/// integers — ids, lengths and enum tags are small numbers, so this keeps
/// payloads compact — and a preallocation fuse set to the frame cap: a
/// sequence header claiming more bytes than a whole frame may carry is
/// corruption, and wincode refuses it before reserving the memory.
type Codec =
    Configuration<true, { MAX_FRAME_BYTES as usize }, BincodeLen, LittleEndian, VarInt, u32>;

const CODEC: Codec = Configuration::default()
    .with_varint_encoding()
    .with_preallocation_size_limit::<{ MAX_FRAME_BYTES as usize }>();

pub struct Frame;

impl Frame {
    /// Encode `msg` to the bytes that travel inside a frame — no length
    /// prefix, no cap check. Exposed so the pieces of the protocol that
    /// are baked elsewhere can prove they survive the codec.
    pub fn encode<T>(msg: &T) -> io::Result<Vec<u8>>
    where
        T: Serialize,
    {
        <SerdeCompat<T> as WincodeSerialize<Codec>>::serialize(msg, CODEC)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Decode one message from the bytes of a frame — the inverse of
    /// [`Frame::encode`].
    pub fn decode<T>(payload: &[u8]) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        <SerdeCompat<T> as WincodeDeserialize<'_, Codec>>::deserialize(payload, CODEC)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Encode `msg` with wincode and write it framed to the stream.
    pub async fn write<T>(stream: &mut TcpStream, msg: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let payload = Self::encode(msg)?;
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

    /// Read one framed wincode message off the stream.
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
        Self::decode(&buf)
    }
}

/// The protocol's envelopes are enums with struct, unit and payload-carrying
/// variants, strings, options and nested structs — every corner of the serde
/// data model the bridge has to get right. A codec that mishandles one of
/// them fails the handshake on every connection, so the shapes go through it
/// here first, on the bytes and then over a real socket.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::protocol::{PROTOCOL_VERSION, RecordingSettings, Request, Response};
    use tokio::net::TcpListener;

    fn handshake() -> Request {
        Request::Handshake {
            coordinator_version: "1.2.3".to_string(),
            protocol_version: PROTOCOL_VERSION,
            recording: RecordingSettings {
                positions: true,
                events: false,
                full_scope: true,
            },
        }
    }

    #[test]
    fn envelopes_round_trip_through_the_codec() {
        let back: Request = Frame::decode(&Frame::encode(&handshake()).unwrap()).unwrap();
        match back {
            Request::Handshake {
                coordinator_version,
                protocol_version,
                recording,
            } => {
                assert_eq!(coordinator_version, "1.2.3");
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                assert!(recording.positions && !recording.events && recording.full_scope);
            }
            other => panic!("decoded {:?}", other),
        }

        let ping: Request = Frame::decode(&Frame::encode(&Request::Ping).unwrap()).unwrap();
        assert!(matches!(ping, Request::Ping));

        let reply = Response::Handshake {
            version: "1.2.3".to_string(),
            protocol_version: PROTOCOL_VERSION,
            threads: 16,
            computer_name: "worker-01".to_string(),
            cpu_brand: "Apple M4".to_string(),
        };
        let back: Response = Frame::decode(&Frame::encode(&reply).unwrap()).unwrap();
        match back {
            Response::Handshake {
                version,
                protocol_version,
                threads,
                computer_name,
                cpu_brand,
            } => {
                assert_eq!(version, "1.2.3");
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                assert_eq!(threads, 16);
                assert_eq!(computer_name, "worker-01");
                assert_eq!(cpu_brand, "Apple M4");
            }
            other => panic!("decoded {:?}", other),
        }

        let rejected = Response::HandshakeRejected {
            reason: "version mismatch".to_string(),
        };
        let back: Response = Frame::decode(&Frame::encode(&rejected).unwrap()).unwrap();
        assert!(
            matches!(back, Response::HandshakeRejected { reason } if reason == "version mismatch")
        );
    }

    #[test]
    fn a_truncated_payload_is_an_error_not_a_panic() {
        let bytes = Frame::encode(&handshake()).unwrap();
        let err = Frame::decode::<Request>(&bytes[..bytes.len() / 2]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    // `#[tokio::test]` expands to `::core::future`, and this workspace has its
    // own `core` crate, so the runtime is built by hand.
    #[test]
    fn frames_cross_a_socket() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();

                let server = tokio::spawn(async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let req: Request = Frame::read(&mut stream).await.unwrap();
                    assert!(matches!(req, Request::Handshake { .. }));
                    Frame::write(&mut stream, &Response::Pong).await.unwrap();
                    let req: Request = Frame::read(&mut stream).await.unwrap();
                    assert!(matches!(req, Request::Ping));
                });

                let mut stream = TcpStream::connect(addr).await.unwrap();
                Frame::write(&mut stream, &handshake()).await.unwrap();
                let reply: Response = Frame::read(&mut stream).await.unwrap();
                assert!(matches!(reply, Response::Pong));
                Frame::write(&mut stream, &Request::Ping).await.unwrap();

                server.await.unwrap();
            });
    }
}
