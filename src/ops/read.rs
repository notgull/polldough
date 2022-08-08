// GNU GPL v3 License

use super::{split_nonnull, Op, OpBase};
use crate::{BufMut, PollingFn, Raw, Source, SourceType};
use std::io::{Error, Result};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::ERROR_IO_PENDING,
    Networking::WinSock::{WSAGetLastError, SOCKET_ERROR, WSABUF},
    System::IO::OVERLAPPED,
};

/// Read in data from a source to a buffer.
pub struct Read<B> {
    source: Raw,
    ty: SourceType,
    buf: B,
    offset: i64,
}

impl<B: BufMut> Read<B> {
    /// Create a new `Read` from the source and a buffer to read into.
    pub fn new<S: Source>(source: &S, buf: B) -> Self {
        Read {
            source: source.as_raw(),
            ty: S::SOURCE_TYPE,
            buf,
            offset: 0,
        }
    }

    /// Set the offset to read from.
    ///
    /// This has no effect for sockets. For files, this indicates the
    /// offset to start reading at.
    pub fn offset(&mut self, offset: i64) -> &mut Self {
        self.offset = offset;
        self
    }

    /// Retrieve the inner buffer.
    ///
    /// # Unsafe
    ///
    /// The operation must be complete before the buffer is retrieved.
    pub unsafe fn into_buf(self) -> B {
        self.buf
    }

    #[cfg(unix)]
    fn polling_function(&mut self) -> PollingFn {
        let (ptr, len) = split_nonnull(self.buf.pointer());
        let source = self.source;
        let offset = self.offset;
        let mut seeked = false;
        let ptr = super::TsPtr(ptr);

        // if we're a file, use seeking
        match self.ty {
            SourceType::File => Box::new(move || {
                if !seeked {
                    syscall!(lseek(source, offset, libc::SEEK_SET))?;
                    seeked = true;
                }

                let n = syscall!(read(source, ptr.0.as_ptr().cast(), len))?;
                Ok(n as _)
            }),
            SourceType::Socket => Box::new(move || {
                let n = syscall!(read(source, ptr.0.as_ptr().cast(), len))?;
                Ok(n as _)
            }),
        }
    }

    #[cfg(unix)]
    const READ: bool = true;
    #[cfg(unix)]
    const WRITE: bool = false;

    #[cfg(target_os = "linux")]
    fn uring_entry(&mut self) -> io_uring::squeue::Entry {
        use io_uring::types::Fd;

        let (ptr, len) = split_nonnull(self.buf.pointer());
        let mut read = io_uring::opcode::Read::new(Fd(self.source), ptr.as_ptr().cast(), len as _);

        if matches!(self.ty, SourceType::File) {
            read = read.offset(self.offset);
        }

        read.build()
    }

    #[cfg(windows)]
    fn win32_start(&mut self, overlapped: *mut OVERLAPPED) -> Result<()> {
        use std::mem::MaybeUninit;

        let (ptr, len) = split_nonnull(self.buf.pointer());
        match self.ty {
            SourceType::Socket => {
                let buf = WSABUF {
                    len: len as _,
                    buf: ptr as _,
                };
                let mut recv_bytes = 0;
                let mut flags = 0;

                check_socket_error!(unsafe {
                    windows_sys::Win32::Networking::WinSock::WSARecv(
                        self.source as _,
                        &buf,
                        1,
                        &mut recv_bytes,
                        &mut flags,
                        overlapped,
                        None,
                    )
                });
            }
            SourceType::File => {
                let mut recv_bytes = 0;

                check_win32_error!(unsafe {
                    windows_sys::Win32::Storage::FileSystem::ReadFile(
                        self.source,
                        ptr as _,
                        len as _,
                        &mut recv_bytes,
                        overlapped,
                    )
                });
            }
        }

        Ok(())
    }
}

impl_op! {
    <B: BufMut> Read
}
