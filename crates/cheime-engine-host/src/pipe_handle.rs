//! Safe wrapper around Windows pipe HANDLE for use with std::io traits.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use windows::Win32::Foundation::HANDLE;

/// A safe owning wrapper around a Windows `HANDLE` for use as `Read` + `Write`.
pub struct PipeHandle {
    inner: OwnedHandle,
}

impl PipeHandle {
    /// Create a `PipeHandle` from a raw Windows `HANDLE`.
    ///
    /// # Safety
    /// The caller must ensure `handle` is valid and not already closed.
    pub unsafe fn from_raw_handle(handle: HANDLE) -> Self {
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.0 as RawHandle) };
        Self { inner: owned }
    }

    pub fn raw_handle(&self) -> RawHandle {
        self.inner.as_raw_handle()
    }

    fn as_hvalue(&self) -> HANDLE {
        HANDLE(self.inner.as_raw_handle() as *mut std::ffi::c_void)
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
            Ok(()) if bytes_read == 0 => Ok(0), // EOF (pipe closed)
            Ok(()) => Ok(bytes_read as usize),
            Err(e) => {
                let code = windows::core::HRESULT::from(e.code());
                if code == windows::core::HRESULT::from(windows::Win32::Foundation::ERROR_BROKEN_PIPE)
                    || code == windows::core::HRESULT::from(windows::Win32::Foundation::ERROR_NO_DATA)
                {
                    Ok(0) // EOF
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, e.to_string()))
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
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = unsafe {
            windows::Win32::Storage::FileSystem::FlushFileBuffers(self.as_hvalue())
        };
        Ok(())
    }
}

unsafe impl Send for PipeHandle {}
