// GNU GPL v3 License

use cfg_if::cfg_if;
use std::{
    borrow::{Borrow, BorrowMut},
    io::{IoSlice, IoSliceMut},
    ops::{Deref, DerefMut},
};

#[cfg(any(unix, windows))]
use std::{mem, ptr, slice};

#[cfg(unix)]
use libc::iovec;
#[cfg(windows)]
use std::convert::TryInto;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::WSABUF;

/// An owned version of an `IoVec`.
#[repr(transparent)]
pub struct OwnedIoSlice(Sys);

#[cfg(windows)]
#[repr(transparent)]
struct Sys {
    // SAFETY: the backing storage is a Box<[u8]>
    buf: WSABUF,
}

#[cfg(unix)]
#[repr(transparent)]
struct Sys {
    // SAFETY: the backing storage is a Box<[u8]>
    buf: iovec,
}

#[cfg(not(any(unix, windows)))]
#[repr(transparent)]
struct Sys {
    buf: Box<[u8]>,
}

impl OwnedIoSlice {
    /// Create a new `OwnedIoVec` from a boxed slice.
    pub fn from_boxed_slice(slice: Box<[u8]>) -> Self {
        cfg_if! {
            if #[cfg(windows)] {
                Self(Sys {
                    buf: WSABUF {
                        len: slice.len().try_into().expect("OwnedIoVec len too large"),
                        buf: Box::into_raw(slice) as *mut _,
                    }
                })
            } else if #[cfg(unix)] {
                Self(Sys {
                    buf: iovec {
                        iov_len: slice.len(),
                        iov_base: Box::into_raw(slice) as *mut _,
                    }
                })
            } else {
                Self(Sys {
                    buf: slice,
                })
            }
        }
    }

    /// Get the `IoSlice` used in this `IoVec`.
    pub fn io_slice(&self) -> IoSlice<'_> {
        cfg_if! {
            if #[cfg(any(unix, windows))] {
                // SAFETY: iovec is ABI-compatible with WSABUF/iovec, so we
                // can do a transmute here
                unsafe { mem::transmute(self.0.buf) }
            } else {
                // just borrow the inside
                IoSlice::new(&self.0.buf)
            }
        }
    }

    /// Get the `IoSliceMut` used in this `IoVec`.
    pub fn io_slice_mut(&mut self) -> IoSliceMut<'_> {
        cfg_if! {
            if #[cfg(any(unix, windows))] {
                // SAFETY: iovec is ABI-compatible with WSABUF/iovec, so we
                // can do a transmute here
                unsafe { mem::transmute(self.0.buf) }
            } else {
                // just borrow the inside
                IoSliceMut::new(&mut self.0.buf)
            }
        }
    }
}

impl From<Box<[u8]>> for OwnedIoSlice {
    fn from(b: Box<[u8]>) -> Self {
        Self::from_boxed_slice(b)
    }
}

impl From<Vec<u8>> for OwnedIoSlice {
    fn from(v: Vec<u8>) -> Self {
        Self::from(v.into_boxed_slice())
    }
}

impl AsRef<[u8]> for OwnedIoSlice {
    fn as_ref(&self) -> &[u8] {
        cfg_if! {
            if #[cfg(windows)] {
                unsafe {
                    slice::from_raw_parts(
                        self.0.buf.buf as *const u8,
                        self.0.buf.len as usize,
                    )
                }
            } else if #[cfg(unix)] {
                unsafe {
                    slice::from_raw_parts(
                        self.0.buf.iov_base as *const u8,
                        self.0.buf.iov_len as usize,
                    )
                }
            } else {
                &self.0.buf
            }
        }
    }
}

impl AsMut<[u8]> for OwnedIoSlice {
    fn as_mut(&mut self) -> &mut [u8] {
        cfg_if! {
            if #[cfg(windows)] {
                unsafe {
                    slice::from_raw_parts_mut(
                        self.0.buf.buf as *mut u8,
                        self.0.buf.len as usize,
                    )
                }
            } else if #[cfg(unix)] {
                unsafe {
                    slice::from_raw_parts_mut(
                        self.0.buf.iov_base as *mut u8,
                        self.0.buf.iov_len as usize,
                    )
                }
            } else {
                &mut self.0.buf
            }
        }
    }
}

impl Borrow<[u8]> for OwnedIoSlice {
    fn borrow(&self) -> &[u8] {
        self.as_ref()
    }
}

impl BorrowMut<[u8]> for OwnedIoSlice {
    fn borrow_mut(&mut self) -> &mut [u8] {
        self.as_mut()
    }
}

impl Deref for OwnedIoSlice {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_ref()
    }
}

impl DerefMut for OwnedIoSlice {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.as_mut()
    }
}

impl Drop for OwnedIoSlice {
    fn drop(&mut self) {
        cfg_if! {
            if #[cfg(windows)] {
                let ptr = self.0.buf.buf as *mut u8;
                let len = self.0.buf.buf as usize;
                mem::drop(
                    unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(ptr, len)) }
                )
            } else if #[cfg(unix)] {
                let ptr = self.0.buf.iov_base as *mut u8;
                let len: usize = self.0.buf.iov_len;
                mem::drop(
                    unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(ptr, len)) }
                )
            }
        }
    }
}
