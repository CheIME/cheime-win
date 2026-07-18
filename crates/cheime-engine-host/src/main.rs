//! CheIME Engine Host.
//!
//! The user-level x64 process that hosts all CheIME engine logic.

mod server;
mod session_runner;

fn main() {
    println!("CheIME Engine Host v0.1.0");
    println!("Protocol version: {}", cheime_model::CORE_PROTOCOL_VERSION);
}
