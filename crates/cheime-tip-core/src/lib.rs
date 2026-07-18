#![forbid(unsafe_code)]

//! CheIME TIP Core: platform adapter utilities shared by the TIP DLL and engine host.
//!
//! This crate provides:
//! - GDI candidate window rendering
//! - Message channel dispatch (mpsc + PostMessage)
//! - Platform action application (TSF edit session helpers — abstracted)
//! - Named pipe I/O wrappers (pure framing, no platform handles)

pub mod error;
pub mod pipe;

pub use error::PipeError;
pub use pipe::{PipeReader, PipeWriter};
