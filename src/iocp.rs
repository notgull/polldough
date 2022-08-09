// GNU GPL v3 License

#![cfg(windows)]

use slab::Slab;
use std::{
    collections::HashMap,
    cell::UnsafeCell,
    fmt,
    io::{self, Result},
    marker::PhantomData,
    mem::{zeroed, MaybeUninit},
    ptr::{self, null_mut},
    sync::{atomic::AtomicBool, Arc, Mutex, MutexGuard, Weak},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::IO::{
        CreateIoCompletionPort, PostQueuedCompletionStatus, OVERLAPPED, OVERLAPPED_ENTRY,
    },
};

use crate::{ops::Op, Event, Source};

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
    /// A buffer to store the results of the I/O operations.
    ///
    /// Holding this mutex implies the exclusive right to poll for
    /// completion events.
    result_buffer: Mutex<Box<[MaybeUninit<OVERLAPPED_ENTRY>]>>,
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
    ///
    /// This doesn't protect `active_ops` directly because calling
    /// `GetQueuedCompletionStatus` will produce several references to
    /// entries within `active_ops`, essentially bypassing the mutex.
    mutation_lock: Mutex<()>,
    /// Notification OVERLAPPED instance.
    notification: UnsafeCell<OpEntry>,
    /// Is the completion object notified?
    notified: AtomicBool,
}

unsafe impl Send for Completion {}
unsafe impl Sync for Completion {}

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
                    }
                    None => f.write_str("<mutation lock held>"),
                }
            }
        }

        f.debug_struct("Completion")
            .field(
                "active_ops",
                &ActiveOpsLen {
                    not_mutating: self.mutation_lock.try_lock().ok(),
                    ops: self.active_ops.get(),
                },
            )
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
    /// The index of the operation in the `active_ops` slab.
    index: usize,
    /// The type of the source.
    ///
    /// This determines what we determine is the error code.
    source_type: SourceType,
}

impl Completion {
    /// Create a new completion object.
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        let iocp_port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 1) };

        if iocp_port == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        Ok(Completion {
            iocp_port,
            source_type: HashMap::new(),
            result_buffer: Mutex::new({
                let mut buffer = Vec::with_capacity(capacity);
                buffer.resize(capacity, MaybeUninit::zeroed());
                buffer
            }),
            active_ops: UnsafeCell::new(Slab::with_capacity(capacity)),
            mutation_lock: Mutex::new(()),
            notification: UnsafeCell::new(OpEntry {
                overlapped: unsafe { zeroed() },
                key: NOTIFY_KEY,
                index: usize::MAX,
                source_type: SourceType::File,
            }),
            notified: AtomicBool::new(false),
        })
    }

    pub(crate) fn register(&self, source: &impl Source) -> Result<()> {
        // register using the CreateIoCompletionPort function
        let result = unsafe { CreateIoCompletionPort(source.as_raw() as _, self.iocp_port, 0, 0) };

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

        // see if we are able to add a new entry
        if active_ops.len() == active_ops.capacity() {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "too many active operations",
            ));
        }

        // add a new entry to the slab
        let entry = OpEntry {
            overlapped: unsafe { zeroed() },
            key,
            index: active_ops.vacant_entry(),
            source_type: op.variant(),
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
        let mut buffer = lock!(self.result_buffer);
        let mut entries_removed = 0;

        // preform the IOCP wait
        unsafe {
            GetQueuedCompletionStatusEx(
                self.iocp_port as _,
                buffer.as_mut_ptr().cast(),
                buffer.len() as _,
                &mut entries_removed,
                timeout_to_ms(timeout),
                FALSE,
            );
        }

        let entries_removed = entries_removed as usize;

        // process the results in the buffer
        // since every entry in the buffer basically contains
        // a reference to the active_ops slab, we have to lock it
        // every entry we grab is owned by us now
        let _guard = lock!(self.mutation_lock);
        let mut ops = unsafe { &mut *self.active_ops.get() };
        let mut process_notify = false;

        out.extend(
            buffer
                .iter()
                .take(entries_removed)
                .map(|entry| {
                    // SAFETY: entry is initialized
                    unsafe { ptr::read(entry.as_ptr()) }
                })
                .filter_map(|entry| {
                    // cast back to an OpEntry and remove it from the slab
                    let op_entry: *mut OpEntry = entry.lpOverlapped.cast();
                    let op_entry = unsafe { &*op_entry };

                    // if this is a notification, flip the switch back
                    // and return
                    if op_entry.key == NOTIFY_KEY {
                        self.notified.store(false, Ordering::SeqCst);
                        process_notify = true;
                        return None;
                    }

                    let index = op_entry.index;
                    ops.remove(index)
                })
                .map(|op| {
                    // convert to an event
                    Event {
                        key: op.key,
                        result: match (op.source_type, op.overlapped.Internal as isize) {
                            (SourceType::File, 0) | (SourceType::Socket, -1) => {
                                Err(io::Error::last_os_error())
                            }
                            (_, code) => Ok(code as _),
                        },
                    }
                }),
        );

        Ok(entries_removed - (process_notify as usize))
    }

    pub(crate) fn notify(&self) -> Result<()> {
        if !self.notified.swap(true, Ordering::SeqCst) {
            // wake up the completion port by posting a message to it
            let res = unsafe {
                PostQueuedCompletionStatus(
                    self.iocp_port as _,
                    0,
                    NOTIFY_KEY as _,
                    &mut *self.notification.get().overlapped,
                )
            };

            if res == 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }
}

fn timeout_to_ms(timeout: Option<Duration>) -> u32 {
    match timeout {
        Some(timeout) => {
            let ms: Option<u32> = timeout.as_millis().try_into().ok();
            ms.and_then(|ms| ms.checked_add(if timeout.subsec_micros > 0 { 1 } else { 0 }))
                .unwrap_or(INFINITE)
        }
        None => INFINITE,
    }
}
