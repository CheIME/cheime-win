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

    #[error("buffer too small: need {needed}, have {have}")]
    BufferTooSmall { needed: usize, have: usize },
}
