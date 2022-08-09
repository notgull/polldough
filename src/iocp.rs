// GNU GPL v3 License

#![cfg(windows)]

use slab::Slab;
use std::{cell::UnsafeCell, marker::PhantomData, fmt, io::{self, Result}, sync::{Arc, Weak, Mutex, MutexGuard}, ptr::null_mut, mem::zeroed, time::Duration};
use windows_sys::Win32::{
    System::IO::{OVERLAPPED, CreateIoCompletionPort},
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
};

use crate::{Source, ops::Op, Event};

const NOTIFY_KEY: u64 = u64::MAX;

/// This `PollData` exposes an `OVERLAPPED` structure, which is used to
/// coordinate I/O operations with the OS.
#[doc(hidden)]
pub struct OpData<'a> {
    pub(crate) overlapped: *mut OVERLAPPED,
    _marker: PhantomData<&'a ()>,
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
        struct ActiveOpsLen<'a> {
            not_mutating: Option<MutexGuard<'a, ()>>,
            ops: *mut Slab<OpEntry>,
        }

        impl fmt::Debug for ActiveOpsLen<'_> {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                match self.not_mutating {
                    Some(_) => {
                        let ops = unsafe { &*self.ops };
                        fmt::Debug::fmt(&ops.len(), f)
                    },
                    None => f.write_str("<mutation lock held>"),
                }
            }
        }

        f.debug_struct("Completion")
            .field("active_ops", &ActiveOpsLen {
                not_mutating: self.mutation_lock.try_lock().ok(),
                ops: self.active_ops.get(),
            })
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
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        let iocp_port = unsafe {
            CreateIoCompletionPort(
                INVALID_HANDLE_VALUE,
                0,
                0,
                1,
            )
        };

        if iocp_port == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error()); 
        }
        
        Ok(Completion {
            iocp_port,
            active_ops: UnsafeCell::new(Slab::with_capacity(capacity)),
            mutation_lock: Mutex::new(()),
        })
    }

    pub(crate) fn register(&self, source: &impl Source) -> Result<()> {
        // register using the CreateIoCompletionPort function
        let result = unsafe {
            CreateIoCompletionPort(source.as_raw() as _, self.iocp_port, 0, 0)
        };

        if result == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub(crate) fn deregister(&self, _source: &impl Source) -> Result<()> {
        // TODO: is there a way of doing this? 

        Ok(())
    }

    pub(crate) fn submit(&self, op: &mut impl Op, key: u64) -> Result<()> {
        // acquire the lock to add a new entry
        let mut _guard = lock!(self.mutation_lock);
        let mut active_ops = unsafe { &mut *self.active_ops.get() };

        // add a new entry to the slab
        let entry = OpEntry {
            overlapped: unsafe { zeroed() },
            key,
        };
        let index = active_ops.insert(entry);
        let mut entry = active_ops.get_mut(index).unwrap();

        // submit the operation
        // from this point on, the operation owns the entry
        let mut op_data = OpData {
            overlapped: &mut entry.overlapped,
            _marker: PhantomData,
        };
        op.run(&mut op_data)?;

        Ok(())
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>, out: &mut Vec<Event>) -> Result<usize> {
        // wait for an event
        Ok(0)
    }

    pub(crate) fn notify(&self) -> Result<()> {
        Ok(())
    }
}