// GNU GPL v3 License

use crate::{OpData, Raw};
use std::{io::Result, ptr::NonNull};

/// The hidden underlying trait for `Op` that is used to not expose OS-specific
/// details.
#[doc(hidden)]
pub trait OpBase {
    /// Enqueue this function into the completion queue, given an OS-specific
    /// "OpData" object.
    fn run(&mut self, op_data: &mut OpData<'_>) -> Result<()>;
}

/// An operation that can be enqueued into the completion queue.
pub trait Op: OpBase {
    /// The raw file descriptor that this operation is associated with.
    fn source(&self) -> Raw;
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

        if ($res) == SOCKET_ERROR {
            let err = unsafe { WSAGetLastError() };
            if err != ERROR_IO_PENDING as _ {
                return Err(Error::last_os_error());
            }
        }
    }};
}

#[cfg(windows)]
macro_rules! check_win32_error {
    ($res: expr) => {{
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_IO_PENDING};

        if ($res) == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_IO_PENDING {
                return Err(Error::last_os_error());
            }
        }
    }};
}

macro_rules! impl_op {
    (< $($gname: ident: $gbound: ident),* > $name: ident) => {
        impl<$($gname: $gbound),*> $crate::ops::Op for $name<$($gname),*> {
            fn source(&self) -> $crate::Raw {
                self.source
            }
        }

        impl<$($gname: $gbound),*> $crate::ops::OpBase for $name<$($gname),*> {
            fn run(&mut self, op_data: &mut $crate::OpData<'_>) -> Result<()> {
                cfg_if::cfg_if! {
                    if #[cfg(target_os = "linux")] {
                        use $crate::OpData::{Polling, Entry};

                        match op_data {
                            Entry(ref mut entry) => {
                                entry.insert(self.uring_entry());
                            },
                            Polling(ref mut poll) => {
                                poll.slot.insert(self.polling_function());
                                poll.read = Self::READ;
                                poll.write = Self::WRITE;
                            }
                        }
                    } else if #[cfg(unix)] {
                        op_data.slot.insert(self.polling_function());
                        op_data.read = Self::READ;
                        op_data.write = Self::WRITE;
                    } else if #[cfg(windows)] {
                        self.win32_start(op_data.overlapped)?;
                    }
                }

                Ok(())
            }
        }
    }
}

mod read;
