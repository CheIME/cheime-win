use crate::error::PipeError;
use cheime_wire::{FramedReader, FramedWriter, MessageCodec, WireError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{Read, Write};

/// Writes length-prefixed framed messages through a codec.
pub struct PipeWriter<W: Write + Send> {
    inner: W,
}

impl<W: Write + Send> PipeWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn write_message<M: Serialize>(
        &mut self,
        codec: &MessageCodec,
        msg: &M,
    ) -> Result<(), PipeError> {
        let payload = codec.encode_handshake(msg)?;

        // Allocate frame buffer: 4-byte header + payload
        let mut frame = vec![0u8; 4 + payload.len()];

        // Use FramedWriter for the actual framing
        FramedWriter::write_frame(&mut frame, codec, msg)?;

        self.inner
            .write_all(&frame[..4 + payload.len()])
            .map_err(|e| PipeError::Io(e.to_string()))?;

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), PipeError> {
        self.inner.flush().map_err(|e| PipeError::Io(e.to_string()))
    }
}

/// Reads length-prefixed framed messages through a codec.
pub struct PipeReader<R: Read + Send> {
    inner: R,
    buffer: Vec<u8>,
}

impl<R: Read + Send> PipeReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
        }
    }

    /// Read available bytes from the underlying handle and try to parse a frame.
    ///
    /// Returns:
    /// - `Ok(Some(decoded))` when a complete message was parsed
    /// - `Ok(None)` when more data is needed
    /// - `Err(...)` on I/O or framing errors
    pub fn try_read_frame<M: DeserializeOwned>(
        &mut self,
        codec: &MessageCodec,
    ) -> Result<Option<M>, PipeError> {
        let mut chunk = [0u8; 4096];
        match self.inner.read(&mut chunk) {
            Ok(0) => {
                // Source exhausted. Only treat as disconnected if buffer is also empty.
                if self.buffer.is_empty() {
                    return Err(PipeError::Disconnected);
                }
                // Otherwise, try to parse remaining buffered data on this call.
                // The next call will get Ok(0) again with empty buffer → disconnect.
            }
            Ok(n) => {
                self.buffer.extend_from_slice(&chunk[..n]);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(PipeError::Io(e.to_string())),
        }

        let max_size = codec.max_size();
        match FramedReader::read_frame(&self.buffer, max_size) {
            Ok(Some((payload_start, payload_len))) => {
                let payload = self.buffer[payload_start..payload_start + payload_len].to_vec();
                let consumed = payload_start + payload_len;
                self.buffer.drain(..consumed);
                let msg: M = codec.decode_handshake(&payload)?;
                Ok(Some(msg))
            }
            Ok(None) => Ok(None),
            Err(WireError::InvalidFrameLength) => {
                Err(PipeError::Wire(WireError::InvalidFrameLength))
            }
            Err(WireError::SizeExceeded { actual, max }) => {
                Err(PipeError::Wire(WireError::SizeExceeded { actual, max }))
            }
            Err(e) => Err(PipeError::Wire(e)),
        }
    }

    /// Consume self and return the accumulated buffer for inspection.
    #[cfg(test)]
    fn into_buffer(self) -> Vec<u8> {
        self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_protocol::{FrontendMessage, MessageHeader};
    use std::io::Cursor;

    fn codec() -> MessageCodec {
        MessageCodec::new(MessageCodec::DEFAULT_MAX)
    }

    fn key_message(c: char) -> FrontendMessage {
        FrontendMessage::KeyCommand {
            header: MessageHeader {
                protocol_version: CORE_PROTOCOL_VERSION,
                client: ClientInstanceId::new(1),
                session: SessionId::new(2),
                epoch: SessionEpoch::new(3),
                sequence: Sequence::new(4),
                revision: Revision::new(5),
                deployment: DeploymentGeneration::new(6),
            },
            event: KeyEvent {
                key: Key::Character(c),
                state: KeyState::default(),
            },
        }
    }

    #[test]
    fn write_then_read_single_message() {
        let msg = key_message('n');
        let c = codec();

        let buffer = Vec::new();
        let cursor = Cursor::new(buffer);

        let mut writer = PipeWriter::new(cursor);
        writer.write_message(&c, &msg).unwrap();

        let written_data = writer.inner.into_inner();
        let read_cursor = Cursor::new(written_data);
        let mut reader = PipeReader::new(read_cursor);

        let decoded: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
        assert!(decoded.is_some());
        assert_eq!(decoded.unwrap(), msg);
    }

    #[test]
    fn read_multiple_messages_from_backlog() {
        let c = codec();
        let msg1 = key_message('n');
        let msg2 = key_message('i');

        let buffer = Vec::new();
        let cursor = Cursor::new(buffer);
        let mut writer = PipeWriter::new(cursor);

        writer.write_message(&c, &msg1).unwrap();
        writer.write_message(&c, &msg2).unwrap();

        let written_data = writer.inner.into_inner();
        let read_cursor = Cursor::new(written_data);
        let mut reader = PipeReader::new(read_cursor);

        let decoded1: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
        assert_eq!(decoded1, Some(msg1));

        let decoded2: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
        assert_eq!(decoded2, Some(msg2));
    }

    #[test]
    fn empty_buffer_returns_none() {
        let c = codec();
        // Feed only 2 bytes of data — not enough for a complete frame header
        let read_cursor = Cursor::new(vec![0x00, 0x01]);
        let mut reader = PipeReader::new(read_cursor);

        let result: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
        assert!(result.is_none());
        // The 2 partial bytes should be buffered internally
        assert_eq!(reader.into_buffer().len(), 2);
    }

    #[test]
    fn disconnection_detected() {
        let c = codec();
        // Empty cursor: first read returns 0 (like a closed pipe)
        let read_cursor = Cursor::new(vec![]);
        let mut reader = PipeReader::new(read_cursor);

        let result: Result<Option<FrontendMessage>, PipeError> = reader.try_read_frame(&c);
        assert!(matches!(result, Err(PipeError::Disconnected)));
    }

    #[test]
    fn partial_write_not_visible_to_reader() {
        let msg = key_message('x');
        let c = codec();

        let buffer = Vec::new();
        let cursor = Cursor::new(buffer);
        let mut writer = PipeWriter::new(cursor);
        writer.write_message(&c, &msg).unwrap();
        let written_data = writer.inner.into_inner();

        // Feed only partial data (first 2 bytes)
        let partial = &written_data[..2];
        let read_cursor = Cursor::new(partial.to_vec());
        let mut reader = PipeReader::new(read_cursor);

        let result: Option<FrontendMessage> = reader.try_read_frame(&c).unwrap();
        assert!(result.is_none());
        // Buffer should have accumulated the partial data
        assert_eq!(reader.into_buffer().len(), 2);
    }
}
