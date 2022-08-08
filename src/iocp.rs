// GNU GPL v3 License

#![cfg(windows)]

use std::marker::PhantomData;
use windows_sys::Win32::System::IO::OVERLAPPED;

/// This `PollData` exposes an `OVERLAPPED` structure, which is used to
/// coordinate I/O operations with the OS.
#[doc(hidden)]
pub struct OpData<'a> {
    pub(crate) overlapped: *mut OVERLAPPED,
    _marker: PhantomData<'a>,
}
