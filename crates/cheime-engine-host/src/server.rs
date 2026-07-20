//! Named pipe server for the engine host.
//!
//! Creates a named pipe at `\\.\pipe\cheime-engine`, listens for TIP client
//! connections, performs the version handshake, and spawns a session runner
//! thread per client.

use crate::pipe_handle::PipeHandle;
use crate::session_runner::run_client_loop;
use cheime_model::{ClientInstanceId, DeploymentGeneration, Revision, SessionEpoch, SessionId};
use cheime_pipeline::DictPipeline;
use cheime_protocol::MessageHeader;
use cheime_tip_core::{PipeReader, PipeWriter};
use cheime_wire::{ClientHello, HelloAck, HelloRejected, MessageCodec, ServerHello, WireError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use thiserror::Error;
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES,
};

/// Default pipe name for the engine.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\cheime-engine";

/// Errors during server or handshake operation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ServerError {
    #[error("handshake timeout")]
    HandshakeTimeout,
    #[error("protocol version mismatch: client sent {client}, server requires {server}")]
    VersionMismatch { client: u16, server: u16 },
    #[error("pipe error: {0}")]
    Pipe(String),
    #[error("wire error: {0}")]
    Wire(String),
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<WireError> for ServerError {
    fn from(e: WireError) -> Self {
        ServerError::Wire(e.to_string())
    }
}

impl From<cheime_tip_core::PipeError> for ServerError {
    fn from(e: cheime_tip_core::PipeError) -> Self {
        ServerError::Pipe(e.to_string())
    }
}

fn poll_accept<F>(stop: &AtomicBool, mut attempt: F) -> Result<bool, ServerError>
where
    F: FnMut() -> Result<bool, ServerError>,
{
    while !stop.load(Ordering::Relaxed) {
        if attempt()? {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(false)
}

/// Run the engine host server loop.
pub fn run_server(
    dict_pipeline: DictPipeline,
    deployment: cheime_model::DeploymentGeneration,
    pipe_name: &str,
) -> Result<(), ServerError> {
    let stop = AtomicBool::new(false);
    run_server_until(dict_pipeline, deployment, pipe_name, &stop)
}

fn run_server_until(
    dict_pipeline: DictPipeline,
    deployment: cheime_model::DeploymentGeneration,
    pipe_name: &str,
    stop: &AtomicBool,
) -> Result<(), ServerError> {
    let connection_counter = Arc::new(AtomicU64::new(1));
    let dict_pipeline = Arc::new(dict_pipeline);

    let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

    eprintln!("[engine] listening on {pipe_name}");

    loop {
        // CreateNamedPipeW returns HANDLE directly (not Result)
        let pipe_handle = unsafe {
            CreateNamedPipeW(
                windows::core::PCWSTR::from_raw(wide_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            )
        };

        if pipe_handle.is_invalid() {
            eprintln!("[engine] CreateNamedPipeW returned invalid handle");
            return Err(ServerError::Pipe(
                "CreateNamedPipeW returned invalid handle".into(),
            ));
        }

        let accepted = poll_accept(stop, || {
            match unsafe { ConnectNamedPipe(pipe_handle, None) } {
                Ok(()) => Ok(true),
                Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(true),
                Err(error)
                    if error.code()
                        == windows::Win32::Foundation::ERROR_PIPE_LISTENING.to_hresult() =>
                {
                    Ok(false)
                }
                Err(error) => Err(ServerError::Pipe(format!(
                    "ConnectNamedPipe failed: {error:?}"
                ))),
            }
        });
        match accepted {
            Ok(true) => {}
            Ok(false) => {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
                }
                return Ok(());
            }
            Err(error) => {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
                }
                eprintln!("[engine] {error}");
                continue;
            }
        }

        let connection_id = connection_counter.fetch_add(1, Ordering::Relaxed);
        let pipeline_clone = Arc::clone(&dict_pipeline);

        // Extract the raw pointer to make it Send-able across threads
        let pipe_ptr: isize = pipe_handle.0 as isize;
        std::thread::spawn(move || {
            let handle = HANDLE(pipe_ptr as *mut std::ffi::c_void);
            eprintln!("[engine] connection {connection_id} connected");
            match handle_client(handle, pipeline_clone, deployment, connection_id) {
                Ok(()) => eprintln!("[engine] connection {connection_id} session ended normally"),
                Err(e) => eprintln!("[engine] connection {connection_id} error: {e}"),
            }
        });
    }
}

/// Handle one client: handshake → session loop → cleanup.
fn handle_client(
    pipe_handle: HANDLE,
    dict_pipeline: Arc<DictPipeline>,
    deployment: cheime_model::DeploymentGeneration,
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
    read_pipe
        .set_blocking(false)
        .map_err(|error| ServerError::Io(error.to_string()))?;
    let write_pipe = unsafe { PipeHandle::from_raw_handle(dup_handle) };
    let codec = MessageCodec::new(MessageCodec::DEFAULT_MAX);
    let mut reader = PipeReader::new(read_pipe);
    let mut writer = PipeWriter::new(write_pipe);

    // 1. Send ServerHello
    let hello = ServerHello {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        supported_caps: vec![],
        deployment_generation: deployment.get(),
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

    if client_hello.client_instance_id == 0
        || client_hello.deployment_generation != deployment.get()
    {
        let rejected = HelloRejected {
            reason: "invalid client or deployment identity".into(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        writer.write_message(&codec, &rejected)?;
        writer.flush()?;
        return Err(ServerError::Pipe(
            "invalid client or deployment identity".into(),
        ));
    }

    let identity = connection_identity(client_hello.client_instance_id, deployment, connection_id);

    // 4. Send HelloAck
    let ack = HelloAck {
        client_instance_id: identity.client.get(),
        session_id: identity.session.get(),
        session_epoch: identity.epoch.get(),
        initial_revision: identity.revision.get(),
        deployment_generation: identity.deployment.get(),
        session_id_base: identity.session.get(),
    };
    writer.write_message(&codec, &ack)?;
    writer.flush()?;

    eprintln!("[engine] connection {connection_id} handshake complete");

    // 5. Use the identity acknowledged to the client.

    reader
        .get_ref()
        .set_blocking(true)
        .map_err(|error| ServerError::Io(error.to_string()))?;

    // 6. Run the session loop
    run_client_loop(reader, writer, codec, (*dict_pipeline).clone(), identity)
        .map_err(|e| ServerError::Pipe(e.to_string()))?;

    Ok(())
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

    #[test]
    fn accept_poll_stops_without_attempting_connection() {
        let stop = AtomicBool::new(true);
        let mut attempts = 0;
        assert!(
            !poll_accept(&stop, || {
                attempts += 1;
                Ok(false)
            })
            .unwrap()
        );
        assert_eq!(attempts, 0);
    }

    #[test]
    fn reconnect_allocates_fresh_nonzero_session_and_epoch() {
        let deployment = DeploymentGeneration::new(7);
        let first = connection_identity(42, deployment, 1);
        let second = connection_identity(42, deployment, 2);
        assert_ne!(first.session, second.session);
        assert_ne!(first.epoch, second.epoch);
        assert_ne!(first.session.get(), 0);
        assert_ne!(first.epoch.get(), 0);
        assert_eq!(first.client, ClientInstanceId::new(42));
        assert_eq!(first.deployment, deployment);
        assert_eq!(first.revision, Revision::new(0));
    }
}
