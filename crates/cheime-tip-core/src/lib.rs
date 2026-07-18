//! CheIME TIP Core: platform adapter utilities shared by the TIP DLL and engine host.
//!
//! This crate provides:
//! - Candidate layout computation (platform-independent)
//! - Message channel dispatch (mpsc + PostMessage)
//! - Platform action application (TSF edit session helpers — abstracted)
//! - Named pipe I/O wrappers (pure framing, no platform handles)

pub mod channel;
pub mod error;
pub mod layout;
pub mod pipe;

pub use channel::{DispatchMessage, TipChannel};
pub use error::PipeError;
pub use layout::{
    CandidateLayout, LayoutRow, compute_window_size, hit_test_candidate, layout_snapshot,
};
pub use pipe::{PipeReader, PipeWriter};
