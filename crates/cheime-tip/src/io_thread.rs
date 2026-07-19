//! I/O thread for the TIP — the dedicated background thread that:
//! 1. Connects to the engine named pipe via `CreateFileW`
//! 2. Performs the client handshake
//! 3. Runs a read/write loop

use cheime_model::{CandidateSnapshot, PlatformAction};
use cheime_protocol::{EngineMessage, FrontendMessage};
use cheime_tip_core::{PipeError, PipeReader, PipeWriter};
use cheime_wire::{ClientHello, HelloAck, MessageCodec, ServerHello};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::Foundation::HWND;
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
    pub fn spawn(
        receiver: Receiver<FrontendMessage>,
        hwnd: HWND,
        pipe_name: &str,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stop_flag);

        let wide_name: Vec<u16> = pipe_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let hwnd_raw = hwnd.0 as isize;
        let handle = std::thread::Builder::new()
            .name("cheime-io".into())
            .spawn(move || {
                io_thread_main(receiver, HWND(hwnd_raw as *mut std::ffi::c_void), &wide_name, &stop);
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
            let _ = handle.join();
        }
    }
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
            if stop.load(Ordering::Relaxed) { return; }
            match try_connect(pipe_name) {
                Ok(h) => break h,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        };

        // 2. Run handshake
        let handshake_result = run_client_handshake(pipe, &codec, 1);
        let (mut reader, mut writer) = match handshake_result {
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
            if stop.load(Ordering::Relaxed) { return; }

            // Send pending frontend messages
            while let Ok(msg) = receiver.try_recv() {
                if writer.write_message(&codec, &msg).is_err() {
                    post_status(hwnd, false, "write error");
                    continue 'reconnect;
                }
            }
            let _ = writer.flush();

            // Read responses
            match reader.try_read_frame(&codec) {
                Ok(Some(msg)) => match msg {
                    EngineMessage::CandidateSnapshot { snapshot, .. } => {
                        post_snapshot(hwnd, &snapshot);
                    }
                    EngineMessage::PlatformAction { action, .. } => {
                        post_action(hwnd, &action);
                    }
                    _ => {}
                },
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(PipeError::Disconnected) => {
                    post_status(hwnd, false, "engine disconnected");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue 'reconnect;
                }
                Err(_) => {}
            }
        }
    }
}

fn try_connect(pipe_name: &[u16]) -> Result<crate::pipe_handle::PipeHandle, String> {
    use windows::Win32::System::Pipes::WaitNamedPipeW;
    use windows::core::PCWSTR;

    loop {
        let handle = unsafe {
            CreateFileW(
                PCWSTR::from_raw(pipe_name.as_ptr()),
                (FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0),
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED,
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
                        let _ = WaitNamedPipeW(PCWSTR::from_raw(pipe_name.as_ptr()), 5000);
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
) -> Result<(PipeReader<crate::pipe_handle::PipeHandle>, PipeWriter<crate::pipe_handle::PipeHandle>), String>
{
    let dup = duplicate_handle(pipe.raw_handle()).ok_or("DuplicateHandle failed")?;
    let mut reader = PipeReader::new(pipe);
    let mut writer = PipeWriter::new(dup);

    let server_hello: Option<ServerHello> = reader
        .try_read_frame(codec)
        .map_err(|e| format!("read ServerHello: {e}"))?;
    let server_hello = server_hello.ok_or("no ServerHello")?;

    if server_hello.protocol_version != cheime_model::CORE_PROTOCOL_VERSION {
        return Err(format!("version mismatch"));
    }

    let client_hello = ClientHello {
        protocol_version: cheime_model::CORE_PROTOCOL_VERSION,
        client_instance_id: client_id,
        client_caps: vec![],
    };
    writer.write_message(codec, &client_hello).map_err(|e| format!("write ClientHello: {e}"))?;
    writer.flush().map_err(|e| format!("flush: {e}"))?;

    let _ack: Option<HelloAck> = reader
        .try_read_frame(codec)
        .map_err(|e| format!("read HelloAck: {e}"))?;
    Ok((reader, writer))
}

fn duplicate_handle(raw: std::os::windows::io::RawHandle) -> Option<crate::pipe_handle::PipeHandle> {
    use windows::Win32::Foundation::DUPLICATE_SAME_ACCESS;
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut dup = HANDLE::default();
    unsafe {
        windows::Win32::Foundation::DuplicateHandle(
            GetCurrentProcess(),
            HANDLE(raw as *mut std::ffi::c_void),
            GetCurrentProcess(),
            &mut dup, 0, false, DUPLICATE_SAME_ACCESS,
        )
    }.ok()?;
    Some(unsafe { crate::pipe_handle::PipeHandle::from_raw_handle(dup) })
}

fn post_snapshot(hwnd: HWND, snapshot: &CandidateSnapshot) {
    let b = Box::new(snapshot.clone());
    unsafe {
        let _ = PostMessageW(
            hwnd, WM_CHEIME_SNAPSHOT,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}

fn post_action(hwnd: HWND, action: &PlatformAction) {
    let b = Box::new(action.clone());
    unsafe {
        let _ = PostMessageW(
            hwnd, WM_CHEIME_ACTION,
            windows::Win32::Foundation::WPARAM(0),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}

fn post_status(hwnd: HWND, connected: bool, detail: &str) {
    let b = Box::new((connected, detail.to_owned()));
    unsafe {
        let _ = PostMessageW(
            hwnd, WM_CHEIME_STATUS,
            windows::Win32::Foundation::WPARAM(connected as usize),
            windows::Win32::Foundation::LPARAM(Box::into_raw(b) as isize),
        );
    }
}
