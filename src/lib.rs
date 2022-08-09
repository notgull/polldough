// GNU GPL v3 License

// utility macros

macro_rules! syscall {
    ($name: ident $($args: tt)*) => {{
        match unsafe { libc::$name $($args)* } {
            -1 => Err(std::io::Error::last_os_error()),
            n => Ok(n),
        }
    }}
}

macro_rules! lock {
    ($mtx: expr) => {{
        match ($mtx).lock() {
            Ok(lk) => lk,
            Err(e) => {
                tracing::error!("Mutex was poisoned: {:?}", &e);
                e.into_inner()
            }
        }
    }};
}

// modules

mod buf;
pub use buf::{Buf, BufMut, OwnedIoSlice};

mod ops;

#[cfg(unix)]
mod polling;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod iocp;

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        use linux as platform;
    } else if #[cfg(unix)] {
        use polling as platform;
    } else if #[cfg(windows)] {
        use iocp as platform;
    } else {
        compile_error! { "Unsupported platform" }
    }
}

mod source;
pub use source::{Raw, Source, SourceType};

use ops::Op;
use std::{fmt, io::Result, time::Duration};

#[doc(hidden)]
pub use platform::OpData;

type PollingFn = Box<dyn FnMut() -> Result<usize> + Send + Sync + 'static>;

/// The events output from waiting.
#[derive(Debug)]
pub struct Event {
    key: u64,
    result: Result<usize>,
}

/// The interface to system faculties for polling for completion on
/// certain events.
pub struct Completion {
    inner: platform::Completion,
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl Completion {
    /// Create a new `Completion` instance with the specified capacity.
    pub fn new(capacity: usize) -> Result<Self> {
        platform::Completion::new(capacity).map(Into::into)
    }

    /// Register a source with the completion.
    pub fn register(&self, source: &impl Source) -> Result<()> {
        self.inner.register(source)
    }

    /// Deregister a source from the completion.
    pub fn deregister(&self, source: &impl Source) -> Result<()> {
        self.inner.deregister(source)
    }

    /// Submit an operation to the completion queue.
    ///
    /// # Safety
    ///
    /// Cannot submit the same `op` more than once.
    pub unsafe fn submit(&self, op: &mut impl Op, key: u64) -> Result<()> {
        self.inner.submit(op, key)
    }

    /// Wait for events to be available.
    pub fn wait(&self, timeout: Option<Duration>, out: &mut Vec<Event>) -> Result<usize> {
        self.inner.wait(timeout, out)
    }

    /// Notify the completion, either interrupting a wait cycle or
    /// pre-empting the next wait cycle.
    pub fn notify(&self) -> Result<()> {
        self.inner.notify()
    }
}

impl From<platform::Completion> for Completion {
    fn from(inner: platform::Completion) -> Self {
        Completion { inner }
    }
}
