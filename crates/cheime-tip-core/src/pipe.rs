use crate::error::PipeError;
use cheime_wire::{MessageCodec, WireError};
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

        // Blocking pipe framing is a little-endian u32 length followed by payload.
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);

        self.inner
            .write_all(&frame)
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

    /// Block until one complete frame is read. Clean EOF before a header returns `None`.
    pub fn read_message<M: DeserializeOwned>(
        &mut self,
        codec: &MessageCodec,
    ) -> Result<Option<M>, PipeError> {
        let mut header = [0u8; 4];
        let header_read = read_exact_count(&mut self.inner, &mut header)?;
        if header_read == 0 {
            return Ok(None);
        }
        if header_read != header.len() {
            return Err(PipeError::TruncatedHeader {
                available: header_read,
            });
        }

        let payload_len = u32::from_le_bytes(header) as usize;
        if payload_len == 0 {
            return Err(PipeError::Wire(WireError::InvalidFrameLength));
        }
        if payload_len > codec.max_size() {
            return Err(PipeError::Wire(WireError::SizeExceeded {
                actual: payload_len,
                max: codec.max_size(),
            }));
        }

        let mut payload = vec![0u8; payload_len];
        let payload_read = read_exact_count(&mut self.inner, &mut payload)?;
        if payload_read != payload_len {
            return Err(PipeError::TruncatedPayload {
                expected: payload_len,
                available: payload_read,
            });
        }

        codec
            .decode_handshake(&payload)
            .map(Some)
            .map_err(Into::into)
    }

    /// Read one complete frame, preserving partial bytes when the deadline expires.
    pub fn read_message_until<M: DeserializeOwned>(
        &mut self,
        codec: &MessageCodec,
        deadline: std::time::Instant,
    ) -> Result<Option<M>, PipeError> {
        loop {
            if let Some(message) = decode_buffered(&mut self.buffer, codec)? {
                return Ok(Some(message));
            }

            let mut chunk = [0u8; 4096];
            match self.inner.read(&mut chunk) {
                Ok(0) => return eof_result(&self.buffer),
                Ok(count) => self.buffer.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        return Err(PipeError::TimedOut);
                    }
                    std::thread::sleep(
                        deadline
                            .saturating_duration_since(now)
                            .min(std::time::Duration::from_millis(1)),
                    );
                }
                Err(error) => return Err(PipeError::Io(error.to_string())),
            }
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
        loop {
            if let Some(message) = decode_buffered(&mut self.buffer, codec)? {
                return Ok(Some(message));
            }
            let mut chunk = [0u8; 4096];
            match self.inner.read(&mut chunk) {
                Ok(0) if self.buffer.is_empty() => return Err(PipeError::Disconnected),
                Ok(0) => return eof_result(&self.buffer),
                Ok(count) => self.buffer.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(PipeError::Io(error.to_string())),
            }
        }
    }

    pub fn get_ref(&self) -> &R {
        &self.inner
    }
}

fn decode_buffered<M: DeserializeOwned>(
    buffer: &mut Vec<u8>,
    codec: &MessageCodec,
) -> Result<Option<M>, PipeError> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let header: [u8; 4] = buffer[..4].try_into().expect("four-byte header");
    let payload_len = validate_length(header, codec)?;
    if buffer.len() < 4 + payload_len {
        return Ok(None);
    }
    let message = codec.decode_handshake(&buffer[4..4 + payload_len])?;
    buffer.drain(..4 + payload_len);
    Ok(Some(message))
}

fn eof_result<M>(buffer: &[u8]) -> Result<Option<M>, PipeError> {
    match buffer.len() {
        0 => Ok(None),
        available @ 1..=3 => Err(PipeError::TruncatedHeader { available }),
        available => {
            let expected =
                u32::from_le_bytes(buffer[..4].try_into().expect("four-byte header")) as usize;
            Err(PipeError::TruncatedPayload {
                expected,
                available: available - 4,
            })
        }
    }
}

fn validate_length(header: [u8; 4], codec: &MessageCodec) -> Result<usize, PipeError> {
    let payload_len = u32::from_le_bytes(header) as usize;
    if payload_len == 0 {
        return Err(PipeError::Wire(WireError::InvalidFrameLength));
    }
    if payload_len > codec.max_size() {
        return Err(PipeError::Wire(WireError::SizeExceeded {
            actual: payload_len,
            max: codec.max_size(),
        }));
    }
    Ok(payload_len)
}

fn read_exact_count<R: Read>(reader: &mut R, mut buffer: &mut [u8]) -> Result<usize, PipeError> {
    let expected = buffer.len();
    while !buffer.is_empty() {
        match reader.read(buffer) {
            Ok(0) => break,
            Ok(count) => {
                let (_, remainder) = buffer.split_at_mut(count);
                buffer = remainder;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(PipeError::Io(error.to_string())),
        }
    }
    Ok(expected - buffer.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{
        CORE_PROTOCOL_VERSION, ClientInstanceId, DeploymentGeneration, Key, KeyEvent, KeyState,
        Revision, Sequence, SessionEpoch, SessionId,
    };
    use cheime_protocol::{FrontendMessage, MessageHeader};
    use std::io::{self, Cursor, Read, Write};

    struct ChunkedReader {
        inner: Cursor<Vec<u8>>,
        chunks: Vec<usize>,
        index: usize,
    }

    impl ChunkedReader {
        fn new(data: Vec<u8>, chunks: Vec<usize>) -> Self {
            Self {
                inner: Cursor::new(data),
                chunks,
                index: 0,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let limit = self.chunks.get(self.index).copied().unwrap_or(buf.len());
            self.index += 1;
            let count = buf.len().min(limit);
            self.inner.read(&mut buf[..count])
        }
    }

    struct ScriptedReader {
        steps: std::collections::VecDeque<io::Result<Vec<u8>>>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or(Ok(Vec::new())) {
                Ok(bytes) => {
                    let count = bytes.len().min(buf.len());
                    buf[..count].copy_from_slice(&bytes[..count]);
                    if count < bytes.len() {
                        self.steps.push_front(Ok(bytes[count..].to_vec()));
                    }
                    Ok(count)
                }
                Err(error) => Err(error),
            }
        }
    }

    struct WouldBlockReader;

    impl Read for WouldBlockReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        }
    }

    struct PartialWriter {
        bytes: Vec<u8>,
        max_chunk: usize,
    }

    impl Write for PartialWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let count = buf.len().min(self.max_chunk);
            self.bytes.extend_from_slice(&buf[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

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

        assert!(matches!(
            reader.try_read_frame::<FrontendMessage>(&c),
            Err(PipeError::TruncatedHeader { available: 2 })
        ));
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

        assert!(matches!(
            reader.try_read_frame::<FrontendMessage>(&c),
            Err(PipeError::TruncatedHeader { available: 2 })
        ));
    }

    #[test]
    fn blocking_read_accepts_chunked_header_and_byte_payload() {
        let c = codec();
        let msg = key_message('n');
        let mut writer = PipeWriter::new(Cursor::new(Vec::new()));
        writer.write_message(&c, &msg).unwrap();
        let data = writer.inner.into_inner();
        let payload_len = data.len() - 4;
        let mut chunks = vec![1, 1, 2];
        chunks.extend(std::iter::repeat_n(1, payload_len));
        let mut reader = PipeReader::new(ChunkedReader::new(data, chunks));

        assert_eq!(reader.read_message(&c).unwrap(), Some(msg));
    }

    #[test]
    fn blocking_read_preserves_multiple_frames() {
        let c = codec();
        let first = key_message('n');
        let second = key_message('i');
        let mut writer = PipeWriter::new(Cursor::new(Vec::new()));
        writer.write_message(&c, &first).unwrap();
        writer.write_message(&c, &second).unwrap();
        let mut reader = PipeReader::new(Cursor::new(writer.inner.into_inner()));

        assert_eq!(reader.read_message(&c).unwrap(), Some(first));
        assert_eq!(reader.read_message(&c).unwrap(), Some(second));
        assert_eq!(reader.read_message::<FrontendMessage>(&c).unwrap(), None);
    }

    #[test]
    fn blocking_read_distinguishes_clean_eof_from_truncated_header() {
        let c = codec();
        let mut clean = PipeReader::new(Cursor::new(Vec::<u8>::new()));
        assert_eq!(clean.read_message::<FrontendMessage>(&c).unwrap(), None);

        for bytes in [vec![1], vec![1, 0], vec![1, 0, 0]] {
            let mut truncated = PipeReader::new(Cursor::new(bytes));
            assert!(matches!(
                truncated.read_message::<FrontendMessage>(&c),
                Err(PipeError::TruncatedHeader { .. })
            ));
        }
    }

    #[test]
    fn blocking_read_rejects_truncated_payload() {
        let c = codec();
        let mut data = 5u32.to_le_bytes().to_vec();
        data.extend([1, 2, 3]);
        let mut reader = PipeReader::new(Cursor::new(data));

        assert!(matches!(
            reader.read_message::<FrontendMessage>(&c),
            Err(PipeError::TruncatedPayload {
                expected: 5,
                available: 3
            })
        ));
    }

    #[test]
    fn blocking_read_rejects_zero_and_oversize_before_payload_read() {
        let c = MessageCodec::new(8);
        for length in [0u32, 9u32] {
            let mut reader =
                PipeReader::new(ChunkedReader::new(length.to_le_bytes().to_vec(), vec![4]));
            assert!(reader.read_message::<FrontendMessage>(&c).is_err());
            assert_eq!(reader.inner.index, 1, "payload read attempted for {length}");
        }
    }

    #[test]
    fn deadline_read_is_bounded_and_preserves_partial_header() {
        let c = codec();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(5);
        let mut blocked = PipeReader::new(WouldBlockReader);
        assert!(matches!(
            blocked.read_message_until::<FrontendMessage>(&c, deadline),
            Err(PipeError::TimedOut)
        ));

        let mut partial = PipeReader::new(ChunkedReader::new(vec![1, 0], vec![2]));
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(5);
        assert!(matches!(
            partial.read_message_until::<FrontendMessage>(&c, deadline),
            Err(PipeError::TruncatedHeader { available: 2 })
        ));
    }

    #[test]
    fn deadline_timeout_preserves_partial_frame_for_next_call() {
        let c = codec();
        let msg = key_message('q');
        let mut encoded = PipeWriter::new(Cursor::new(Vec::new()));
        encoded.write_message(&c, &msg).unwrap();
        let bytes = encoded.inner.into_inner();
        let steps = std::collections::VecDeque::from([
            Ok(bytes[..2].to_vec()),
            Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Ok(bytes[2..4].to_vec()),
            Ok(bytes[4..].to_vec()),
        ]);
        let mut reader = PipeReader::new(ScriptedReader { steps });
        assert!(matches!(
            reader.read_message_until::<FrontendMessage>(&c, std::time::Instant::now()),
            Err(PipeError::TimedOut)
        ));
        assert_eq!(
            reader
                .read_message_until::<FrontendMessage>(
                    &c,
                    std::time::Instant::now() + std::time::Duration::from_millis(50)
                )
                .unwrap(),
            Some(msg)
        );
    }

    #[test]
    fn buffered_reader_reports_truncated_eof() {
        let c = codec();
        let mut header = PipeReader::new(Cursor::new(vec![1, 0]));
        assert!(matches!(
            header.try_read_frame::<FrontendMessage>(&c),
            Err(PipeError::TruncatedHeader { available: 2 })
        ));

        let mut payload = 5u32.to_le_bytes().to_vec();
        payload.extend([1, 2]);
        let mut reader = PipeReader::new(Cursor::new(payload));
        assert!(matches!(
            reader.try_read_frame::<FrontendMessage>(&c),
            Err(PipeError::TruncatedPayload {
                expected: 5,
                available: 2
            })
        ));
    }

    #[test]
    fn writer_retries_partial_writes() {
        let c = codec();
        let msg = key_message('x');
        let mut expected_writer = PipeWriter::new(Cursor::new(Vec::new()));
        expected_writer.write_message(&c, &msg).unwrap();
        let expected = expected_writer.inner.into_inner();
        let mut writer = PipeWriter::new(PartialWriter {
            bytes: Vec::new(),
            max_chunk: 1,
        });

        writer.write_message(&c, &msg).unwrap();

        assert_eq!(writer.inner.bytes, expected);
    }
}
