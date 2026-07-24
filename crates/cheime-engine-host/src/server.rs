//! Named pipe server for the engine host.
//!
//! Creates a named pipe at `\\.\pipe\cheime-engine`, listens for TIP client
//! connections, performs the version handshake, and spawns a session runner
//! thread per client.
//!
//! Each connection gets its own `ComposablePipeline` (per-processor state)
//! but all share the same dictionary index and user store.

use crate::pipe_handle::PipeHandle;
use crate::session_runner;
use cheime_config::schema::SchemaConfig;
use cheime_dictionary::CompiledIndex;
use cheime_model::{ClientInstanceId, DeploymentGeneration, Revision, SessionEpoch, SessionId};
use cheime_pipeline::factory::PipelineFactory;
use cheime_pipeline::learning::LearningService;
use cheime_protocol::MessageHeader;
use cheime_tip_core::{PipeReader, PipeWriter};
use cheime_wire::{ClientHello, HelloAck, HelloRejected, MessageCodec, ServerHello, WireError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::PCWSTR;

/// Default pipe name for the engine.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\cheime-engine";

/// Errors during server or handshake operation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ServerError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("version mismatch: client={client}, server={server}")]
    VersionMismatch { client: u16, server: u16 },
    #[error("pipe error: {0}")]
    Pipe(String),
}

impl From<WireError> for ServerError {
    fn from(e: WireError) -> Self {
        ServerError::Pipe(e.to_string())
    }
}

impl From<cheime_tip_core::PipeError> for ServerError {
    fn from(e: cheime_tip_core::PipeError) -> Self {
        ServerError::Pipe(e.to_string())
    }
}

/// Run the engine host server loop.
pub fn run_server(
    config: &SchemaConfig,
    index: Arc<CompiledIndex>,
    learning: Arc<LearningService>,
    pipe_name: &str,
) -> Result<(), ServerError> {
    let expiry_service = Arc::clone(&learning);
    if let Err(error) = std::thread::Builder::new()
        .name(String::from("cheime-learning-expiry"))
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            confirm_expired_tick(&expiry_service);
        })
    {
        eprintln!("[engine] failed to start learning expiry worker: {error}");
    }
    let stop = AtomicBool::new(false);
    run_server_until(config, index, learning, pipe_name, &stop)
}

fn run_server_until(
    config: &SchemaConfig,
    index: Arc<CompiledIndex>,
    learning: Arc<LearningService>,
    pipe_name: &str,
    stop: &AtomicBool,
) -> Result<(), ServerError> {
    let deployment = DeploymentGeneration::new(1);
    let mut connection_id: u64 = 0;

    let pipe_name_wide: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR::from_raw(pipe_name_wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            eprintln!("CreateNamedPipeW failed");
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }

        // Wait for a client to connect before spawning handler thread
        let connected = unsafe { ConnectNamedPipe(handle, None) };
        let client_ready = match connected {
            Ok(()) => true,
            Err(e) if e.code() == windows::Win32::Foundation::ERROR_PIPE_CONNECTED.into() => true,
            _ => {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                continue;
            }
        };
        if !client_ready {
            continue;
        }

        connection_id = connection_id.wrapping_add(1);
        let conn_id = connection_id;

        let cfg = config.clone();
        let idx = Arc::clone(&index);
        let learning = Arc::clone(&learning);
        let raw_handle = handle.0 as usize; // raw handle value; safe across threads
        std::thread::spawn(move || {
            let h = HANDLE(raw_handle as *mut std::ffi::c_void);
            if let Err(e) = handle_client(h, &cfg, idx, learning, deployment, conn_id) {
                eprintln!("[engine] connection {} error: {e}", conn_id);
            }
        });
    }

    Ok(())
}
/// Handle one client: handshake → session loop → cleanup.
fn handle_client(
    pipe_handle: HANDLE,
    config: &SchemaConfig,
    index: Arc<CompiledIndex>,
    learning: Arc<LearningService>,
    deployment: DeploymentGeneration,
    connection_id: u64,
) -> Result<(), ServerError> {
    // Duplicate the handle so we can have separate reader/writer cursors
    let mut dup_handle = HANDLE::default();
    let dup_ok = unsafe {
        windows::Win32::Foundation::DuplicateHandle(
            windows::Win32::System::Threading::GetCurrentProcess(),
            pipe_handle,
            windows::Win32::System::Threading::GetCurrentProcess(),
            &mut dup_handle,
            0,
            false,
            windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
        )
    };
    if dup_ok.is_err() {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
        }
        return Err(ServerError::Io("DuplicateHandle failed".into()));
    }

    let read_pipe = unsafe { PipeHandle::from_raw_handle(pipe_handle) };
    let write_pipe = unsafe { PipeHandle::from_raw_handle(dup_handle) };
    read_pipe
        .set_blocking(false)
        .map_err(|error| ServerError::Io(error.to_string()))?;
    let codec = MessageCodec::new(MessageCodec::DEFAULT_MAX);
    let mut reader = PipeReader::new(read_pipe);
    let mut writer = PipeWriter::new(write_pipe);

    // 1. Send ServerHello
    let hello = ServerHello {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        supported_caps: vec![],
    };
    writer.write_message(&codec, &hello)?;
    writer.flush()?;

    // 2. Read ClientHello
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let client_hello: ClientHello = reader
        .read_message_until(&codec, deadline)
        .map_err(|error| match error {
            cheime_tip_core::PipeError::TimedOut => ServerError::HandshakeTimeout,
            other => ServerError::from(other),
        })?
        .ok_or(ServerError::HandshakeTimeout)?;

    // 3. Version check
    if client_hello.protocol_version != cheime_model::CORE_PROTOCOL_VERSION {
        let rejected = HelloRejected {
            reason: format!(
                "version mismatch: engine={}, tip={}",
                cheime_model::CORE_PROTOCOL_VERSION,
                client_hello.protocol_version
            ),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        writer.write_message(&codec, &rejected)?;
        writer.flush()?;
        return Err(ServerError::VersionMismatch {
            client: client_hello.protocol_version,
            server: cheime_model::CORE_PROTOCOL_VERSION,
        });
    }

    if client_hello.client_instance_id == 0 {
        let rejected = HelloRejected {
            reason: "invalid client identity".into(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        writer.write_message(&codec, &rejected)?;
        writer.flush()?;
        return Err(ServerError::Pipe("invalid client identity".into()));
    }

    let identity = connection_identity(client_hello.client_instance_id, deployment, connection_id);

    // 4. Send HelloAck (simplified protocol: only session_id_base)
    let ack = HelloAck {
        session_id_base: identity.session.get(),
    };
    writer.write_message(&codec, &ack)?;
    writer.flush()?;

    eprintln!("[engine] connection {connection_id} handshake complete");

    // 5. Build pipeline for this connection (per-session processor state)
    let pipeline = PipelineFactory::build_with_learning(config, Some(learning), Some(index), None)
        .map_err(|e| ServerError::Pipe(format!("pipeline build failed: {e}")))?;

    reader
        .get_ref()
        .set_blocking(true)
        .map_err(|error| ServerError::Io(error.to_string()))?;

    // 6. Run the session loop
    session_runner::run_client_loop(reader, writer, codec, pipeline, identity)
        .map_err(|e| ServerError::Pipe(e.to_string()))?;

    Ok(())
}

fn confirm_expired_tick(service: &LearningService) {
    service.confirm_expired();
}

fn connection_identity(
    client_id: u64,
    deployment: DeploymentGeneration,
    connection_id: u64,
) -> MessageHeader {
    let session = connection_id.saturating_mul(2).max(1);
    let epoch = session.saturating_add(1);
    MessageHeader {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client: ClientInstanceId::new(client_id),
        session: SessionId::new(session),
        epoch: SessionEpoch::new(epoch),
        sequence: cheime_model::Sequence::new(0),
        revision: Revision::new(0),
        deployment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{ActionId, CommitToken};
    use cheime_pipeline::learning::{
        CommitRecord, FakeClock, LearningService, LEARNING_DELAY_MS,
    };
    use cheime_user_data::UserStore;
    use parking_lot::Mutex;

    #[test]
    fn connection_identity_allocates_unique_sessions() {
        let d = DeploymentGeneration::new(1);
        let id1 = connection_identity(100, d, 1);
        let id2 = connection_identity(100, d, 2);
        assert_eq!(id1.session.get(), 2); // 1*2.max(1) = 2
        assert_eq!(id2.session.get(), 4); // 2*2.max(1) = 4
        assert!(id1.session != id2.session);
    }

    #[test]
    fn expiry_tick_confirms_shared_store() {
        let store = Arc::new(Mutex::new(UserStore::new("test")));
        let clock = Arc::new(FakeClock::new(0));
        let service = Arc::new(LearningService::new(store.clone(), clock.clone()));
        service.commit_applied(
            CommitToken {
                session: SessionId::new(1),
                epoch: SessionEpoch::new(1),
                action_id: ActionId::new(1),
            },
            CommitRecord {
                text: String::from("旎皓"),
                canonical_code: String::from("ni hao"),
                schema: String::from("qp"),
                lexemes: Vec::new(),
                exact_phrase: false,
            },
        );
        clock.set(LEARNING_DELAY_MS);
        confirm_expired_tick(&service);
        assert_eq!(store.lock().query("ni hao")[0].text, "旎皓");
    }
}
