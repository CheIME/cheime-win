//! Named pipe server for the engine host.
//!
//! Creates a named pipe at `\\.\pipe\cheime-engine`, listens for TIP client
//! connections, performs the version handshake, and spawns a session runner
//! thread per client.

use crate::pipe_handle::PipeHandle;
use crate::session_runner::run_client_loop;
use cheime_model::ClientInstanceId;
use cheime_pipeline::DictPipeline;
use cheime_protocol::MessageHeader;
use cheime_tip_core::{PipeReader, PipeWriter};
use cheime_wire::{ClientHello, HelloAck, HelloRejected, MessageCodec, ServerHello, WireError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use windows::Win32::Foundation::{
    ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    PIPE_UNLIMITED_INSTANCES,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};

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

/// Run the engine host server loop.
pub fn run_server(
    dict_pipeline: DictPipeline,
    deployment: cheime_model::DeploymentGeneration,
    pipe_name: &str,
) -> Result<(), ServerError> {
    let client_counter = Arc::new(AtomicU64::new(1));
    let dict_pipeline = Arc::new(dict_pipeline);

    let wide_name: Vec<u16> = pipe_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    eprintln!("[engine] listening on {pipe_name}");

    loop {
        // CreateNamedPipeW returns HANDLE directly (not Result)
        let pipe_handle = unsafe {
            CreateNamedPipeW(
                windows::core::PCWSTR::from_raw(wide_name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            )
        };

        if pipe_handle.is_invalid() {
            eprintln!("[engine] CreateNamedPipeW returned invalid handle");
            return Err(ServerError::Pipe("CreateNamedPipeW returned invalid handle".into()));
        }

        let connect_result = unsafe { ConnectNamedPipe(pipe_handle, None) };
        match connect_result {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
            Err(e) => {
                eprintln!("[engine] ConnectNamedPipe failed: {e:?}");
                unsafe { let _ = windows::Win32::Foundation::CloseHandle(pipe_handle); }
                continue;
            }
        }

        let client_id = client_counter.fetch_add(1, Ordering::Relaxed);
        let pipeline_clone = Arc::clone(&dict_pipeline);

        // Extract the raw pointer to make it Send-able across threads
        let pipe_ptr: isize = pipe_handle.0 as isize;
        drop(pipe_handle); // don't drop the handle — ownership transfers to thread
        std::thread::spawn(move || {
            let handle = HANDLE(pipe_ptr as *mut std::ffi::c_void);
            eprintln!("[engine] client {client_id} connected");
            match handle_client(handle, pipeline_clone, deployment, client_id) {
                Ok(()) => eprintln!("[engine] client {client_id} session ended normally"),
                Err(e) => eprintln!("[engine] client {client_id} error: {e}"),
            }
        });
    }
}

/// Handle one client: handshake → session loop → cleanup.
fn handle_client(
    pipe_handle: HANDLE,
    dict_pipeline: Arc<DictPipeline>,
    deployment: cheime_model::DeploymentGeneration,
    client_id: u64,
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
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(pipe_handle); }
        return Err(ServerError::Io("DuplicateHandle failed".into()));
    }

    let read_pipe = unsafe { PipeHandle::from_raw_handle(pipe_handle) };
    let write_pipe = unsafe { PipeHandle::from_raw_handle(dup_handle) };
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
    let client_hello: Option<ClientHello> = reader.try_read_frame(&codec)?;
    let client_hello = match client_hello {
        Some(ch) => ch,
        None => return Err(ServerError::HandshakeTimeout),
    };

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

    // 4. Send HelloAck
    let ack = HelloAck {
        session_id_base: client_id,
    };
    writer.write_message(&codec, &ack)?;
    writer.flush()?;

    eprintln!("[engine] client {client_id} handshake complete");

    // 5. Build session identity
    let identity = MessageHeader {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client: ClientInstanceId::new(client_hello.client_instance_id),
        session: cheime_model::SessionId::new(client_id),
        epoch: cheime_model::SessionEpoch::new(1),
        sequence: cheime_model::Sequence::new(0),
        revision: cheime_model::Revision::new(0),
        deployment,
    };

    // 6. Run the session loop
    run_client_loop(reader, writer, codec, (*dict_pipeline).clone(), identity)
        .map_err(|e| ServerError::Pipe(e.to_string()))?;

    Ok(())
}
