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

use std::io::Result;

#[doc(hidden)]
pub use platform::OpData;

type PollingFn = Box<dyn FnMut() -> Result<usize> + Send + Sync + 'static>;

/// The events output from waiting.
#[derive(Debug)]
pub struct Event {
    key: u64,
    result: Result<usize>,
}
