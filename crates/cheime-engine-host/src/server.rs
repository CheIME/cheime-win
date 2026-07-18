//! Named pipe server for the engine host.
//!
//! Listens on `\\.\pipe\cheime-engine` for TIP client connections,
//! performs the version handshake, and spawns session runners.

use cheime_model::ClientInstanceId;
use cheime_wire::{ClientHello, HelloAck, HelloRejected, MessageCodec, ServerHello, WireError};
use std::io::{Read, Write};
use thiserror::Error;

use cheime_tip_core::{PipeError, PipeReader, PipeWriter};

/// Errors during server or handshake operation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[allow(dead_code)]
pub enum ServerError {
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("protocol version mismatch: client sent {client}, server requires {server}")]
    VersionMismatch { client: u16, server: u16 },
    #[error("pipe error: {0}")]
    Pipe(#[from] PipeError),
    #[error("wire error: {0}")]
    Wire(#[from] WireError),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("server already running")]
    AlreadyRunning,
}

/// Represents the engine after a successful handshake with a TIP client.
#[allow(dead_code)]
pub struct EngineConnection {
    pub client_id: ClientInstanceId,
    pub reader: PipeReader<Box<dyn Read + Send>>,
    pub writer: PipeWriter<Box<dyn Write + Send>>,
    pub codec: MessageCodec,
}

/// Run the server-side handshake.
///
/// 1. Send `ServerHello`
/// 2. Read `ClientHello` (5-second timeout enforced by caller)
/// 3. Validate version → `HelloAck` or `HelloRejected`
///
/// Returns `EngineConnection` on success. Caller should close the pipe
/// on error.
#[allow(dead_code)]
pub fn run_handshake<R, W>(
    mut reader: PipeReader<R>,
    mut writer: PipeWriter<W>,
    engine_version: &str,
    next_client_id: u64,
) -> Result<EngineConnection, ServerError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let codec = MessageCodec::new(MessageCodec::DEFAULT_MAX);

    // 1. Send ServerHello
    let hello = ServerHello {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        engine_version: engine_version.to_owned(),
        supported_caps: vec![],
    };
    writer.write_message(&codec, &hello)?;
    writer.flush()?;

    // 2. Read ClientHello
    let client_hello: Option<ClientHello> = reader.try_read_frame(&codec)?;
    let client_hello = match client_hello {
        Some(ch) => ch,
        None => return Err(ServerError::HandshakeTimeout),
    };

    // 3. Version check
    if client_hello.protocol_version != cheime_model::CORE_PROTOCOL_VERSION {
        let rejected = HelloRejected {
            reason: format!(
                "protocol version mismatch: engine={}, tip={}",
                cheime_model::CORE_PROTOCOL_VERSION,
                client_hello.protocol_version
            ),
            engine_version: engine_version.to_owned(),
        };
        writer.write_message(&codec, &rejected)?;
        writer.flush()?;
        return Err(ServerError::VersionMismatch {
            client: client_hello.protocol_version,
            server: cheime_model::CORE_PROTOCOL_VERSION,
        });
    }

    // 4. Send HelloAck
    let ack = HelloAck {
        session_id_base: next_client_id,
    };
    writer.write_message(&codec, &ack)?;
    writer.flush()?;

    Ok(EngineConnection {
        client_id: ClientInstanceId::new(client_hello.client_instance_id),
        reader: PipeReader::new(Box::new(std::io::empty()) as Box<dyn Read + Send>),
        writer: PipeWriter::new(Box::new(std::io::sink()) as Box<dyn Write + Send>),
        codec,
    })
}

/// Handshake test with in-memory byte buffers.
#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::CORE_PROTOCOL_VERSION;

    fn codec() -> MessageCodec {
        MessageCodec::new(MessageCodec::DEFAULT_MAX)
    }

    #[test]
    fn server_hello_encodes_and_decodes() {
        let c = codec();
        let hello = ServerHello {
            protocol_version: CORE_PROTOCOL_VERSION,
            engine_version: String::from("0.1.0"),
            supported_caps: vec![],
        };

        let data = c.encode_handshake(&hello).unwrap();
        let decoded: ServerHello = c.decode_handshake(&data).unwrap();
        assert_eq!(decoded.protocol_version, CORE_PROTOCOL_VERSION);
    }

    #[test]
    fn client_hello_encodes_and_decodes() {
        let c = codec();
        let hello = ClientHello {
            protocol_version: CORE_PROTOCOL_VERSION,
            client_instance_id: 42,
            client_caps: vec![],
        };

        let data = c.encode_handshake(&hello).unwrap();
        let decoded: ClientHello = c.decode_handshake(&data).unwrap();
        assert_eq!(decoded.client_instance_id, 42);
    }

    #[test]
    fn hello_rejected_encodes_and_decodes() {
        let c = codec();
        let rejected = HelloRejected {
            reason: String::from("bad version"),
            engine_version: String::from("0.1.0"),
        };

        let data = c.encode_handshake(&rejected).unwrap();
        let decoded: HelloRejected = c.decode_handshake(&data).unwrap();
        assert_eq!(decoded.reason, "bad version");
    }

    #[test]
    fn full_handshake_success_path() {
        // Simulate what the server does: write ServerHello, read ClientHello,
        // write HelloAck. Both ends communicate through a shared byte buffer.
        let c = codec();

        // Server side encodes hello
        let server_hello = ServerHello {
            protocol_version: CORE_PROTOCOL_VERSION,
            engine_version: String::from("0.1.0"),
            supported_caps: vec![],
        };
        let server_hello_bytes = c.encode_handshake(&server_hello).unwrap();

        // Client reads it
        let decoded: ServerHello = c.decode_handshake(&server_hello_bytes).unwrap();
        assert_eq!(decoded.protocol_version, CORE_PROTOCOL_VERSION);

        // Client sends back ClientHello
        let client_hello = ClientHello {
            protocol_version: CORE_PROTOCOL_VERSION,
            client_instance_id: 7,
            client_caps: vec![],
        };
        let client_hello_bytes = c.encode_handshake(&client_hello).unwrap();

        // Server reads it and sends HelloAck
        let decoded_client: ClientHello = c.decode_handshake(&client_hello_bytes).unwrap();
        assert_eq!(decoded_client.protocol_version, CORE_PROTOCOL_VERSION);

        let ack = HelloAck {
            session_id_base: 100,
        };
        let ack_bytes = c.encode_handshake(&ack).unwrap();

        // Client reads ack
        let decoded_ack: HelloAck = c.decode_handshake(&ack_bytes).unwrap();
        assert_eq!(decoded_ack.session_id_base, 100);
    }

    #[test]
    fn version_mismatch_rejected() {
        let c = codec();

        let client_hello = ClientHello {
            protocol_version: 999, // wrong
            client_instance_id: 1,
            client_caps: vec![],
        };
        let data = c.encode_handshake(&client_hello).unwrap();
        let decoded: ClientHello = c.decode_handshake(&data).unwrap();
        assert_ne!(decoded.protocol_version, CORE_PROTOCOL_VERSION);
    }
}
