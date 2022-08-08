// GNU GPL v3 License

#![cfg(target_os = "linux")]

mod uring;

use std::{io::Result, time::Duration};

use crate::{ops::Op, polling, Event, Source};
use io_uring::squeue::Entry as SEntry;

/// This `OpData` is either a wrapper around the `polling`
/// `OpData`, or, if applicable, a wrapper around a submission
/// queue entry for the `io_uring` library.
#[doc(hidden)]
pub enum OpData<'a> {
    Polling(polling::OpData<'a>),
    Entry(Option<SEntry>),
}

#[derive(Debug)]
pub(crate) enum Completion {
    Polling(polling::Completion),
    Uring(uring::Completion),
}

macro_rules! defer {
    ($self: ident . $fnname: ident $($arg: tt)*) => {{
        match $self {
            Self::Polling(po) => po.$fnname $($arg)*,
            Self::Uring(uo) => uo.$fnname $($arg)*,
        }
    }}
}

impl Completion {
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        match uring::Completion::new(capacity) {
            Ok(ur) => Ok(Completion::Uring(ur)),
            Err(e) => {
                tracing::error!("Failed to create uring completion: {:?}", e);
                polling::Completion::new(capacity).map(Completion::Polling)
            }
        }
    }

    pub(crate) fn register(&self, source: &impl Source) -> Result<()> {
        defer!(self.register(source))
    }

    pub(crate) fn deregister(&self, source: &impl Source) -> Result<()> {
        defer!(self.deregister(source))
    }

    pub(crate) fn submit(&self, op: &mut impl Op, key: u64) -> Result<()> {
        defer!(self.submit(op, key))
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>, out: &mut Vec<Event>) -> Result<usize> {
        defer!(self.wait(timeout, out))
    }

    pub(crate) fn notify(&self) -> Result<()> {
        defer!(self.notify())
    }
}
