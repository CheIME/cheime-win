//! I/O thread for the TIP — the dedicated background thread that:
//! 1. Connects to the engine named pipe via `CreateFileW`
//! 2. Performs the client handshake
//! 3. Runs a read/write loop

use crate::tsf_interfaces::tsf_log;
use cheime_model::{CandidateSnapshot, PlatformAction};
use cheime_protocol::{EngineMessage, FrontendMessage};
use cheime_tip_core::{PipeError, PipeReader, PipeWriter};
use cheime_wire::{ClientHello, HelloAck, MessageCodec, ServerHello};
#[cfg(test)]
use serde::Deserialize;
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

/// Custom window messages for TIP dispatch.
pub const WM_CHEIME_SNAPSHOT: u32 = WM_USER + 100;
pub const WM_CHEIME_ACTION: u32 = WM_USER + 101;
pub const WM_CHEIME_STATUS: u32 = WM_USER + 102;

pub struct IoThread {
    handle: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
}

impl IoThread {
    pub fn spawn(receiver: Receiver<FrontendMessage>, hwnd: HWND, pipe_name: &str) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd_raw = hwnd.0 as isize;
        let handle = std::thread::Builder::new()
            .name("cheime-io".into())
            .spawn(move || {
                io_thread_main(
                    receiver,
                    HWND(hwnd_raw as *mut std::ffi::c_void),
                    &wide_name,
                    &stop,
                );
            })
            .expect("spawn I/O thread");

        Self {
            handle: Some(handle),
            stop_flag,
        }
    }

    pub fn shutdown(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let thread = HANDLE(handle.as_raw_handle());
            unsafe {
                let _ = windows::Win32::System::IO::CancelSynchronousIo(thread);
            }
            let _ = handle.join();
        }
    }
}

fn next_client_instance_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let counter = NEXT.fetch_add(1, Ordering::Relaxed);
    let process = u64::from(std::process::id());
    (process << 32 | counter).max(1)
}

fn io_thread_main(
    receiver: Receiver<FrontendMessage>,
    hwnd: HWND,
    pipe_name: &[u16],
    stop: &AtomicBool,
) {
    let codec = MessageCodec::new(MessageCodec::DEFAULT_MAX);

    'reconnect: loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }

        // 1. Connect to the pipe
        let pipe = loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match try_connect(pipe_name, stop) {
                Ok(h) => break h,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        };

        // 2. Run handshake
        let handshake_result = run_client_handshake(pipe, &codec, next_client_instance_id());
        let (mut reader, mut writer, mut session) = match handshake_result {
            Ok(pair) => {
                post_status(hwnd, true, "connected");
                pair
            }
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue 'reconnect;
            }
        };

        // 3. Message loop
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }

            // Send pending frontend messages
            while let Ok(msg) = receiver.try_recv() {
                let msg = session.prepare(msg);
                if writer.write_message(&codec, &msg).is_err() {
                    post_status(hwnd, false, "write error");
                    continue 'reconnect;
                }
            }
            let _ = writer.flush();

            // Read one complete response, bounded so outbound work and shutdown stay responsive.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(25);
            match reader.read_message_until(&codec, deadline) {
                Ok(Some(msg)) => {
                    if session.observe_engine(&msg).is_err() {
                        tsf_log("[CheIME] IO: engine identity mismatch, reconnecting");
                        continue 'reconnect;
                    }
                    match msg {
                        EngineMessage::CandidateSnapshot { snapshot, .. } => {
                            tsf_log(&format!(
                                "[CheIME] IO snapshot preedit={} candidates={}",
                                snapshot.preedit,
                                snapshot.candidates.len()
                            ));
                            post_snapshot(hwnd, &snapshot);
                        }
                        EngineMessage::PlatformAction { action, .. } => {
                            tsf_log(&format!("[CheIME] IO action={action:?}"));
                            post_action(hwnd, &action);
                        }
                        _ => {}
                    }
                }
                Err(PipeError::TimedOut) => {}
                Ok(None) | Err(PipeError::Disconnected) => {
                    post_status(hwnd, false, "engine disconnected");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue 'reconnect;
                }
                Err(_) => continue 'reconnect,
            }
        }
    }
}

fn try_connect(
    pipe_name: &[u16],
    stop: &AtomicBool,
) -> Result<crate::pipe_handle::PipeHandle, String> {
    use windows::Win32::System::Pipes::WaitNamedPipeW;
    use windows::core::PCWSTR;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let handle = unsafe {
            CreateFileW(
                PCWSTR::from_raw(pipe_name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
                HANDLE::default(),
            )
        };
        match handle {
            Ok(h) if !h.is_invalid() => {
                return Ok(unsafe { crate::pipe_handle::PipeHandle::from_raw_handle(h) });
            }
            Ok(_) => return Err("invalid handle".into()),
            Err(e) => {
                if e.code() == windows::Win32::Foundation::ERROR_PIPE_BUSY.into() {
                    unsafe {
                        let _ = WaitNamedPipeW(PCWSTR::from_raw(pipe_name.as_ptr()), 200);
                    }
                    continue;
                }
                return Err(format!("CreateFileW: {e:?}"));
            }
        }
    }
}

fn run_client_handshake(
    pipe: crate::pipe_handle::PipeHandle,
    codec: &MessageCodec,
    client_id: u64,
) -> Result<
    (
        PipeReader<crate::pipe_handle::PipeHandle>,
        PipeWriter<crate::pipe_handle::PipeHandle>,
        FrontendSession,
    ),
    String,
> {
    let dup = duplicate_handle(pipe.raw_handle()).ok_or("DuplicateHandle failed")?;
    dup.set_blocking(true)
        .map_err(|error| format!("set writer blocking: {error}"))?;
    pipe.set_blocking(false)
        .map_err(|error| format!("set reader nonblocking: {error}"))?;
    let mut reader = PipeReader::new(pipe);
    let mut writer = PipeWriter::new(dup);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let server_hello: ServerHello = reader
        .read_message_until(codec, deadline)
        .map_err(|e| format!("read ServerHello: {e}"))?
        .ok_or("no ServerHello")?;

    if server_hello.protocol_version != cheime_model::CORE_PROTOCOL_VERSION {
        return Err("version mismatch".into());
    }

    let client_hello = ClientHello {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client_instance_id: client_id,
        client_caps: vec![],
        deployment_generation: server_hello.deployment_generation,
    };
    writer
        .write_message(codec, &client_hello)
        .map_err(|e| format!("write ClientHello: {e}"))?;
    writer.flush().map_err(|e| format!("flush: {e}"))?;

    let ack: HelloAck = reader
        .read_message_until(codec, deadline)
        .map_err(|e| format!("read HelloAck: {e}"))?
        .ok_or("no HelloAck")?;
    let state = validate_handshake(&server_hello, client_id, &ack)?;
    Ok((reader, writer, FrontendSession::new(state)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AcknowledgedState {
    client: cheime_model::ClientInstanceId,
    session: cheime_model::SessionId,
    epoch: cheime_model::SessionEpoch,
    revision: cheime_model::Revision,
    deployment: cheime_model::DeploymentGeneration,
}

struct FrontendSession {
    state: AcknowledgedState,
    next_sequence: u64,
}

impl FrontendSession {
    fn new(state: AcknowledgedState) -> Self {
        Self {
            state,
            next_sequence: 1,
        }
    }

    fn observe_engine(&mut self, message: &EngineMessage) -> Result<(), String> {
        let header = match message {
            EngineMessage::SessionOpened { header }
            | EngineMessage::CandidateSnapshot { header, .. }
            | EngineMessage::PlatformAction { header, .. }
            | EngineMessage::SessionClosed { header } => header,
            EngineMessage::ProtocolRejected { .. } => return Err("protocol rejected".into()),
        };
        if header.protocol_version != cheime_model::CORE_PROTOCOL_VERSION
            || header.client != self.state.client
            || header.session != self.state.session
            || header.epoch != self.state.epoch
            || header.deployment != self.state.deployment
        {
            return Err("engine identity mismatch".into());
        }
        if header.revision < self.state.revision {
            return Err("stale engine revision".into());
        }
        self.state.revision = header.revision;
        Ok(())
    }

    fn prepare(&mut self, message: FrontendMessage) -> FrontendMessage {
        let header = cheime_protocol::MessageHeader {
            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
            client: self.state.client,
            session: self.state.session,
            epoch: self.state.epoch,
            sequence: cheime_model::Sequence::new(self.next_sequence),
            revision: self.state.revision,
            deployment: self.state.deployment,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        match message {
            FrontendMessage::OpenSession { .. } => FrontendMessage::OpenSession { header },
            FrontendMessage::CloseSession { .. } => FrontendMessage::CloseSession { header },
            FrontendMessage::KeyCommand { event, .. } => {
                FrontendMessage::KeyCommand { header, event }
            }
            FrontendMessage::UiCommand { command, .. } => {
                FrontendMessage::UiCommand { header, command }
            }
            FrontendMessage::PlatformActionResult { result, .. } => {
                FrontendMessage::PlatformActionResult { header, result }
            }
        }
    }
}

fn validate_handshake(
    server: &ServerHello,
    requested_client: u64,
    ack: &HelloAck,
) -> Result<AcknowledgedState, String> {
    if server.protocol_version != cheime_model::CORE_PROTOCOL_VERSION {
        return Err("protocol mismatch".into());
    }
    if requested_client == 0 || ack.client_instance_id != requested_client {
        return Err("client identity mismatch".into());
    }
    if ack.session_id == 0 || ack.session_epoch == 0 {
        return Err("zero session identity".into());
    }
    if server.deployment_generation == 0
        || ack.deployment_generation != server.deployment_generation
    {
        return Err("deployment mismatch".into());
    }
    Ok(AcknowledgedState {
        client: cheime_model::ClientInstanceId::new(ack.client_instance_id),
        session: cheime_model::SessionId::new(ack.session_id),
        epoch: cheime_model::SessionEpoch::new(ack.session_epoch),
        revision: cheime_model::Revision::new(ack.initial_revision),
        deployment: cheime_model::DeploymentGeneration::new(ack.deployment_generation),
    })
}

#[cfg(test)]
#[derive(Deserialize)]
struct AckEnvelope {
    client_instance_id: u64,
    session_id: u64,
    session_epoch: u64,
    initial_revision: u64,
    deployment_generation: u64,
    #[serde(default)]
    session_id_base: u64,
}

#[cfg(test)]
fn decode_ack_response(codec: &MessageCodec, payload: &[u8]) -> Result<HelloAck, String> {
    if payload.is_empty() {
        return Err("missing HelloAck".into());
    }
    let value: AckEnvelope = codec
        .decode_handshake(payload)
        .map_err(|e| format!("malformed or rejected handshake response: {e}"))?;
    Ok(HelloAck {
        client_instance_id: value.client_instance_id,
        session_id: value.session_id,
        session_epoch: value.session_epoch,
        initial_revision: value.initial_revision,
        deployment_generation: value.deployment_generation,
        session_id_base: value.session_id_base,
    })
}

fn duplicate_handle(
    raw: std::os::windows::io::RawHandle,
) -> Option<crate::pipe_handle::PipeHandle> {
    use windows::Win32::Foundation::DUPLICATE_SAME_ACCESS;
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut dup = HANDLE::default();
    unsafe {
        windows::Win32::Foundation::DuplicateHandle(
            GetCurrentProcess(),
            HANDLE(raw),
            GetCurrentProcess(),
            &mut dup,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .ok()?;
    Some(unsafe { crate::pipe_handle::PipeHandle::from_raw_handle(dup) })
}

fn post_snapshot(hwnd: HWND, snapshot: &CandidateSnapshot) {
    let b = Box::new(snapshot.clone());
    unsafe {
        let _ = PostMessageW(
            hwnd,
            WM_CHEIME_SNAPSHOT,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}

fn post_action(hwnd: HWND, action: &PlatformAction) {
    let b = Box::new(action.clone());
    unsafe {
        let _ = PostMessageW(
            hwnd,
            WM_CHEIME_ACTION,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}

fn post_status(hwnd: HWND, connected: bool, detail: &str) {
    let b = Box::new((connected, detail.to_owned()));
    unsafe {
        let _ = PostMessageW(
            hwnd,
            WM_CHEIME_STATUS,
            windows::Win32::Foundation::WPARAM(connected as usize),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use cheime_model::{ClientInstanceId, DeploymentGeneration, Revision, SessionEpoch, SessionId};

    fn hello(deployment: u64) -> ServerHello {
        ServerHello {
            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
            engine_version: "test".into(),
            supported_caps: vec![],
            deployment_generation: deployment,
        }
    }

    fn ack(client: u64, deployment: u64) -> HelloAck {
        HelloAck {
            client_instance_id: client,
            session_id: 7,
            session_epoch: 8,
            initial_revision: 9,
            deployment_generation: deployment,
            session_id_base: 7,
        }
    }

    #[test]
    fn matching_ack_installs_negotiated_identity() {
        let state = validate_handshake(&hello(6), 42, &ack(42, 6)).unwrap();
        assert_eq!(state.client, ClientInstanceId::new(42));
        assert_eq!(state.session, SessionId::new(7));
        assert_eq!(state.epoch, SessionEpoch::new(8));
        assert_eq!(state.revision, Revision::new(9));
        assert_eq!(state.deployment, DeploymentGeneration::new(6));
    }

    #[test]
    fn validation_rejects_wrong_protocol_client_deployment_and_zero_identity() {
        let mut wrong_protocol = hello(6);
        wrong_protocol.protocol_version += 1;
        assert!(validate_handshake(&wrong_protocol, 42, &ack(42, 6)).is_err());
        assert!(validate_handshake(&hello(6), 42, &ack(43, 6)).is_err());
        assert!(validate_handshake(&hello(6), 42, &ack(42, 5)).is_err());
        let mut zero = ack(42, 6);
        zero.session_epoch = 0;
        assert!(validate_handshake(&hello(6), 42, &zero).is_err());
    }

    #[test]
    fn connect_stops_before_touching_invalid_pipe_name() {
        let stop = AtomicBool::new(true);
        assert!(matches!(try_connect(&[], &stop), Err(error) if error == "stopped"));
    }

    #[test]
    fn generated_client_identity_is_nonzero_and_fresh() {
        let first = next_client_instance_id();
        let second = next_client_instance_id();
        assert_ne!(first, 0);
        assert_ne!(second, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn frontend_session_rewrites_outbound_header_with_acknowledged_state() {
        let state = validate_handshake(&hello(6), 42, &ack(42, 6)).unwrap();
        let mut session = FrontendSession::new(state);
        let input = FrontendMessage::OpenSession {
            header: cheime_protocol::MessageHeader {
                protocol_version: 999,
                client: ClientInstanceId::new(1),
                session: SessionId::new(1),
                epoch: SessionEpoch::new(1),
                sequence: cheime_model::Sequence::new(77),
                revision: Revision::new(1),
                deployment: DeploymentGeneration::new(1),
            },
        };
        let rewritten = session.prepare(input);
        let header = rewritten.header();
        assert_eq!(header.protocol_version, cheime_model::CORE_PROTOCOL_VERSION);
        assert_eq!(header.client, ClientInstanceId::new(42));
        assert_eq!(header.session, SessionId::new(7));
        assert_eq!(header.epoch, SessionEpoch::new(8));
        assert_eq!(header.sequence, cheime_model::Sequence::new(1));
        assert_eq!(header.revision, Revision::new(9));
        assert_eq!(header.deployment, DeploymentGeneration::new(6));
    }

    #[test]
    fn frontend_session_observes_engine_revision_and_rejects_wrong_identity() {
        let state = validate_handshake(&hello(6), 42, &ack(42, 6)).unwrap();
        let mut session = FrontendSession::new(state);
        let header = cheime_protocol::MessageHeader {
            protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
            client: ClientInstanceId::new(42),
            session: SessionId::new(7),
            epoch: SessionEpoch::new(8),
            sequence: cheime_model::Sequence::new(1),
            revision: Revision::new(12),
            deployment: DeploymentGeneration::new(6),
        };
        session
            .observe_engine(&EngineMessage::SessionOpened {
                header: header.clone(),
            })
            .unwrap();
        let outbound = session.prepare(FrontendMessage::CloseSession {
            header: header.clone(),
        });
        assert_eq!(outbound.header().revision, Revision::new(12));
        assert_eq!(outbound.header().sequence, cheime_model::Sequence::new(1));

        let mut wrong = header;
        wrong.session = SessionId::new(99);
        assert!(
            session
                .observe_engine(&EngineMessage::SessionOpened { header: wrong })
                .is_err()
        );
    }

    #[test]
    fn handshake_response_rejects_rejected_malformed_and_missing_ack() {
        let codec = MessageCodec::new(MessageCodec::DEFAULT_MAX);
        let rejected = cheime_wire::HelloRejected {
            reason: "no".into(),
            engine_version: "test".into(),
        };
        assert!(decode_ack_response(&codec, &codec.encode_handshake(&rejected).unwrap()).is_err());
        assert!(decode_ack_response(&codec, &[0xff]).is_err());
        assert!(decode_ack_response(&codec, &[]).is_err());
    }
}
