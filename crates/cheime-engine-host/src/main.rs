//! CheIME Engine Host.
//!
//! The user-level x64 process that hosts all CheIME engine logic:
//! - Named pipe listener
//! - Client connection management
//! - Session Actor lifecycle
//! - Dictionary deployment and query
//! - Lua extension runtime
//! - User data persistence

mod server;

fn main() {
    println!("CheIME Engine Host v0.1.0");
    println!("Protocol version: {}", cheime_model::CORE_PROTOCOL_VERSION);
}
