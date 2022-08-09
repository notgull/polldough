// GNU GPL v3 License

#![cfg(windows)]

use slab::Slab;
use std::{cell::UnsafeCell, marker::PhantomData, fmt, sync::{Arc, Weak, Mutex}};
use windows_sys::Win32::{
    System::IO::OVERLAPPED,
    Foundation::HANDLE,
};

/// This `PollData` exposes an `OVERLAPPED` structure, which is used to
/// coordinate I/O operations with the OS.
#[doc(hidden)]
pub struct OpData<'a> {
    pub(crate) overlapped: *mut OVERLAPPED,
    _marker: PhantomData<'a>,
}

pub(crate) struct Completion {
    /// The handle to the IOCP port.
    iocp_port: HANDLE,
    /// A buffer for holding active operations.
    /// 
    /// This is implied to be stable. Empty slots are owned by the
    /// completion object, while each `OpEntry` full slot is owned
    /// by the in-progress operation. Changing a slot from empty to
    /// full or vice versa requires locking the mutex.
    /// 
    /// The capacity should never change, since `OpEntry` does not
    /// have a stable deref.
    active_ops: UnsafeCell<Slab<OpEntry>>,
    /// An exclusion lock for changes to the `active_ops` slab.
    mutation_lock: Mutex<()>,
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Completion")
            .field("active_ops", &self.active_ops.len())
            .finish_non_exhaustive()
    }
}

/// An entry in the active ops list used to keep track of the
/// state of an operation.
#[repr(C)]
struct OpEntry {
    /// The OVERLAPPED entry used in this operation.
    /// 
    /// Thanks to `repr(C)`, this always comes first, meaning a pointer
    /// to our `OpEntry` is the same as a pointer to the `OVERLAPPED`
    /// and vice versa.
    overlapped: OVERLAPPED,
    /// The event ID for this operation.
    key: u64,
}

impl Completion {
    /// Create a new completion object.
    pub fn new() -> Result<Self> {
        let iocp_port = unsafe {
            CreateIoCompletionPort(
                INVALID_HANDLE_VALUE,
                null_mut(),
                0,
                1,
            )
        };

        if iocp_port == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error()); 
        }
        
        Completion {
            iocp_port,
            active_ops: UnsafeCell::new(Slab::with_capacity(capacity)),
            mutation_lock: Mutex::new(()),
        }
    }
}