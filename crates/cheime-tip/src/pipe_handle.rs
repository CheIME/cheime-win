//! Safe wrapper around a Windows pipe HANDLE for use as `Read` + `Write` via std::io.
//!
//! Used by both the engine host (server) and the TIP (client).
//! Converts from a raw `HANDLE` obtained from `CreateFileW` or `CreateNamedPipeW`.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use windows::Win32::Foundation::HANDLE;

pub struct PipeHandle {
    inner: OwnedHandle,
}

impl PipeHandle {
    pub unsafe fn from_raw_handle(handle: HANDLE) -> Self {
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.0 as RawHandle) };
        Self { inner: owned }
    }

    pub fn raw_handle(&self) -> RawHandle {
        self.inner.as_raw_handle()
    }

    pub fn set_blocking(&self, blocking: bool) -> io::Result<()> {
        use windows::Win32::System::Pipes::{PIPE_NOWAIT, PIPE_WAIT, SetNamedPipeHandleState};
        let mode = if blocking { PIPE_WAIT } else { PIPE_NOWAIT };
        unsafe { SetNamedPipeHandleState(self.as_hvalue(), Some(&mode), None, None) }
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn as_hvalue(&self) -> HANDLE {
        HANDLE(self.inner.as_raw_handle())
    }
}

impl io::Read for PipeHandle {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes_read = 0u32;
        match unsafe {
            windows::Win32::Storage::FileSystem::ReadFile(
                self.as_hvalue(),
                Some(buf),
                Some(&mut bytes_read),
                None,
            )
        } {
            Ok(()) if bytes_read == 0 => Ok(0),
            Ok(()) => Ok(bytes_read as usize),
            Err(e) => {
                let hresult = e.code();
                if hresult
                    == windows::core::HRESULT::from(windows::Win32::Foundation::ERROR_BROKEN_PIPE)
                    || hresult
                        == windows::core::HRESULT::from(windows::Win32::Foundation::ERROR_NO_DATA)
                {
                    if hresult
                        == windows::core::HRESULT::from(windows::Win32::Foundation::ERROR_NO_DATA)
                    {
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    } else {
                        Ok(0)
                    }
                } else {
                    Err(io::Error::other(e.to_string()))
                }
            }
        }
    }
}

impl io::Write for PipeHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes_written = 0u32;
        match unsafe {
            windows::Win32::Storage::FileSystem::WriteFile(
                self.as_hvalue(),
                Some(buf),
                Some(&mut bytes_written),
                None,
            )
        } {
            Ok(()) => Ok(bytes_written as usize),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        unsafe { windows::Win32::Storage::FileSystem::FlushFileBuffers(self.as_hvalue()) }
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

unsafe impl Send for PipeHandle {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::FromRawHandle;

    #[test]
    fn flush_propagates_invalid_handle_error() {
        let invalid = unsafe {
            OwnedHandle::from_raw_handle(windows::Win32::Foundation::INVALID_HANDLE_VALUE.0)
        };
        let mut pipe = PipeHandle { inner: invalid };
        assert!(io::Write::flush(&mut pipe).is_err());
    }
}
