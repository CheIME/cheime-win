use cheime_wire::WireError;
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PipeError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("wire error: {0}")]
    Wire(#[from] WireError),

    #[error("pipe disconnected")]
    Disconnected,

    #[error("pipe read timed out")]
    TimedOut,

    #[error("truncated frame header: read {available} of 4 bytes")]
    TruncatedHeader { available: usize },

    #[error("truncated frame payload: expected {expected} bytes, read {available}")]
    TruncatedPayload { expected: usize, available: usize },

    #[error("buffer too small: need {needed}, have {have}")]
    BufferTooSmall { needed: usize, have: usize },
}
