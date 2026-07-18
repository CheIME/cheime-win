//! CheIME Engine Host.
//!
//! The user-level x64 process that hosts all CheIME engine logic:
//! - Named pipe listener (\\\\?\\pipe\\cheime-engine)
//! - Client connection management (one pipe per client instance)
//! - Session Actor lifecycle
//! - Dictionary deployment and query
//! - Lua extension runtime
//! - User data persistence

fn main() {
    println!("CheIME Engine Host v0.1.0");
}
