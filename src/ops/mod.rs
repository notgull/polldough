// GNU GPL v3 License

use crate::{OpData, Raw, SourceType};
use std::{io::Result, ptr::NonNull};

/// The hidden underlying trait for `Op` that is used to not expose OS-specific
/// details.
/// 
/// This trait is semver-exempt.
/// 
/// # Safety
/// 
/// This trait is never implemented by any item outside of this crate.
#[doc(hidden)]
pub unsafe trait OpBase {
    /// Enqueue this function into the completion queue, given an OS-specific
    /// "OpData" object.
    fn run(&mut self, op_data: &mut OpData<'_>) -> Result<()>;
}

/// An operation that can be enqueued into the completion queue.
/// 
/// # Safety
/// 
/// This is a sealed trait, only implemented on crate-specific types.
pub unsafe trait Op: OpBase {
    /// The variables "captured" by this operation, returned at the
    /// very end.
    type Captured;

    /// The raw file descriptor that this operation is associated with.
    fn source(&self) -> Raw;
    /// The variant of the source.
    fn variant(&self) -> SourceType;
    /// Get the captured variables.
    /// 
    /// # Safety
    /// 
    /// The operation must be complete at this point.
    unsafe fn into_captured(self) -> Self::Captured;
}

// split a NonNull<[u8]> into ptr and len
#[inline]
fn split_nonnull(ptr: NonNull<[u8]>) -> (NonNull<u8>, usize) {
    let len = unsafe { &*ptr.as_ptr() }.len();
    let ptr = ptr.as_ptr() as *mut u8;
    (unsafe { NonNull::new_unchecked(ptr) }, len)
}

/// Thread-safe container for `NonNull<T>`
struct TsPtr<T: ?Sized>(NonNull<T>);

unsafe impl<T: ?Sized> Send for TsPtr<T> {}
unsafe impl<T: ?Sized> Sync for TsPtr<T> {}

#[cfg(windows)]
macro_rules! check_socket_error {
    ($res: expr) => {{
        use windows_sys::Win32::{
            Foundation::ERROR_IO_PENDING,
            Networking::WinSock::{WSAGetLastError, SOCKET_ERROR},
            System::IO::OVERLAPPED,
        };

        let res = ($res);

        if res == SOCKET_ERROR {
            let err = unsafe { windows_sys::Win32::Networking::WinSock::WSAGetLastError() };
            if err == ERROR_IO_PENDING as _ {
                Ok(None)
            } else {
                Err(std::io::Error::last_os_error())
            }
        } else {
            Ok(Some(res as usize))
        }
    }};
}

#[cfg(windows)]
macro_rules! check_win32_error {
    ($res: expr) => {{
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_IO_PENDING};

        let res = ($res);

        if res == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if err == ERROR_IO_PENDING {
                Ok(None)
            } else {
                Err(std::io::Error::last_os_error())
            }
        } else {
            Ok(Some(res as usize))
        }
    }};
}

#[cfg(windows)]
macro_rules! install_offset {
    ($overlapped: expr, $offset: expr) => {{
        let (lo, hi) = crate::ops::split_into_offsets($offset);
        unsafe {
            (&mut *$overlapped).Anonymous.Anonymous.Offset = lo;
            (&mut *$overlapped).Anonymous.Anonymous.OffsetHigh = hi;
        }
    }};
}

macro_rules! impl_op {
    (< $($gname: ident: $gbound: ident),* > $name: ident: $cap: ty) => {
        unsafe impl<$($gname: $gbound),*> $crate::ops::Op for $name<$($gname),*> {
            type Captured = $cap;

            fn source(&self) -> $crate::Raw {
                self.source
            }

            fn variant(&self) -> $crate::SourceType {
                self.variant
            }

            unsafe fn into_captured(self) -> $cap {
                self.into_buf()
            }
        }

        unsafe impl<$($gname: $gbound),*> $crate::ops::OpBase for $name<$($gname),*> {
            fn run(&mut self, op_data: &mut $crate::OpData<'_>) -> Result<()> {
                cfg_if::cfg_if! {
                    if #[cfg(target_os = "linux")] {
                        use $crate::OpData::{Polling, Entry};

                        match op_data {
                            Entry(ref mut entry) => {
                                *entry = Some(self.uring_entry());
                            },
                            Polling(ref mut poll) => {
                                poll.slot = Some(self.polling_function());
                                poll.read = Self::READ;
                                poll.write = Self::WRITE;
                            }
                        }
                    } else if #[cfg(unix)] {
                        op_data.slot.insert(self.polling_function());
                        op_data.read = Self::READ;
                        op_data.write = Self::WRITE;
                    } else if #[cfg(windows)] {
                        let res = self.win32_start(op_data.overlapped);
                        op_data.immediate_result = res.transpose();
                    }
                }

                Ok(())
            }
        }
    }
}

/// Split into Offset and OffsetHigh
#[cfg(windows)]
#[inline]
fn split_into_offsets(offset: isize) -> (u32, u32) {
    let offset = offset as u64;
    let offset_high = (offset >> 32) as u32;
    let offset_low = (offset & 0xffffffff) as u32;
    (offset_low, offset_high)
}

mod read;
pub use read::Read;

mod write;
pub use write::Write;
