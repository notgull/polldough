// GNU GPL v3 License

use crate::{ops::Op, Event, Raw, Source};
use io_uring::{
    cqueue::Entry as CEvent,
    squeue::Entry as SEvent,
    types::{Fd, SubmitArgs, Timespec},
    IoUring,
};
use std::{
    cell::UnsafeCell,
    fmt,
    io::{self, Result},
    mem::MaybeUninit,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, SystemTime},
};

const ENTRY_KEY: u64 = u64::MAX;

/// A completion-oriented I/O interface based on io_uring.
pub(crate) struct Completion {
    /// The underlying interface to `io_uring`.
    uring: IoUring,
    /// The mutex guarding the submission queue.
    ///
    /// Holding this mutex grants exclusive access to the
    /// submission queue.
    submit_lock: Mutex<()>,
    /// A buffer containing a buffer used for completion
    /// events.
    ///
    /// Holding this mutex grants exclusive access to the
    /// completion queue.
    complete_buffer: Mutex<Box<[MaybeUninit<CEvent>]>>,
    /// A file descriptor for the event FD, used to wake up the
    /// `uring` waiting.
    wakeup_fd: Raw,
    /// A buffer used by the wakeup FD for reading the notification.
    ///
    /// It is guaranteed that only one reference at a time to this
    /// buffer exists.
    wakeup_buffer: UnsafeCell<[u8; 8]>,
    /// A flag indicating whether this system has already been notified.
    notified: AtomicBool,
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Completion")
            .field("notified", &self.notified.load(Ordering::Relaxed))
            .field("uring_params", &self.uring.params())
            .finish_non_exhaustive()
    }
}

impl Completion {
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        Ok(Self {
            uring: IoUring::new(capacity as _)?,
            submit_lock: Mutex::new(()),
            complete_buffer: Mutex::new({
                let mut v = Vec::with_capacity(capacity);
                unsafe {
                    v.set_len(capacity);
                }
                v.into_boxed_slice()
            }),
            wakeup_fd: syscall!(eventfd(0, libc::EFD_CLOEXEC))?,
            wakeup_buffer: [0u8; 8].into(),
            notified: AtomicBool::new(false),
        })
    }

    pub(crate) fn register(&self, _source: &impl Source) -> Result<()> {
        // no op
        Ok(())
    }

    pub(crate) fn deregister(&self, _source: &impl Source) -> Result<()> {
        // no op
        Ok(())
    }

    pub(crate) fn submit(&self, op: &mut impl Op, key: u64) -> Result<()> {
        // feed it an OpData and see if it produces an SEvent
        let mut opdata = super::OpData::Entry(None);
        op.run(&mut opdata)?;

        let entry = match opdata {
            super::OpData::Entry(Some(entry)) => entry.user_data(key),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "op did not produce an entry",
                ))
            }
        };

        // lock the submission queue and write to it
        let _guard = lock!(self.submit_lock);
        // SAFETY: with the guard held, we can write to the submission queue
        let mut queue = unsafe { self.uring.submission_shared() };

        // SAFETY: contract of Op guarantees "entry" is a valid entry
        unsafe {
            queue
                .push(&entry)
                .map_err(|err| io::Error::new(io::ErrorKind::OutOfMemory, err))?;
        }

        Ok(())
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>, out: &mut Vec<Event>) -> Result<usize> {
        // determine the timeout args
        let mut sargs = SubmitArgs::new();
        let timespec = timeout.map(|timeout| {
            let target_time = SystemTime::now() + timeout;
            let duration_since_epoch = target_time.duration_since(SystemTime::UNIX_EPOCH).unwrap();

            // create a linux timespec
            Timespec::new()
                .sec(duration_since_epoch.as_secs())
                .nsec(duration_since_epoch.subsec_nanos())
        });

        if let Some(ref timespec) = timespec {
            sargs = sargs.timespec(timespec);
        }

        // use the submitter to wait for completion events
        let submitter = self.uring.submitter();
        submitter.submit_with_args(1, &sargs)?;

        // we now have at least one event, try reading all of them
        let mut complete_buffer = lock!(self.complete_buffer);
        // SAFETY: we own the mutex, we can access the buffer
        let mut queue = unsafe { self.uring.completion_shared() };

        // read to the buffer
        let completed_events = queue.fill(&mut complete_buffer).len();

        // process the events
        out.extend(
            complete_buffer
                .iter()
                .take(completed_events)
                .map(|event| {
                    // SAFETY: we know the event is initialized
                    unsafe { ptr::read(event.as_ptr()) }
                })
                .filter(|event| {
                    // if the event is our filtered-out key,
                    // unset the notified switch and discard it
                    if event.user_data() == ENTRY_KEY {
                        self.notified.store(false, Ordering::SeqCst);
                        false
                    } else {
                        true
                    }
                })
                .map(|event| Event {
                    key: event.user_data(),
                    result: match event.result() {
                        -1 => Err(io::Error::last_os_error()),
                        n => Ok(n as _),
                    },
                }),
        );

        Ok(completed_events)
    }

    pub(crate) fn notify(&self) -> Result<()> {
        // send an event over our event FD if we aren't already notified
        if !self.notified.swap(true, Ordering::SeqCst) {
            let notification = 1u64.to_ne_bytes();
            syscall!(write(self.wakeup_fd, notification.as_ptr().cast(), 8))?;

            // wait for an event to be read
            let entry = io_uring::opcode::Read::new(
                Fd(self.wakeup_fd),
                self.wakeup_buffer.get() as *mut _,
                8,
            )
            .build();

            // submit the entry
            let _guard = lock!(self.submit_lock);
            let mut queue = unsafe { self.uring.submission_shared() };

            unsafe {
                queue
                    .push(&entry)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            }
        }

        Ok(())
    }
}
