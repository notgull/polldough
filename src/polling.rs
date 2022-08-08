// GNU GPL v3 License

#![cfg(unix)]

use crate::{ops::Op, Event, PollingFn, Raw, Source, SourceType};
use polling::{Event as PollEvent, Poller};
use slab::Slab;
use std::{
    collections::HashMap,
    fmt,
    io::{self, Result},
    marker::PhantomData,
    os::unix::prelude::RawFd,
    sync::Mutex,
    task::Poll,
    time::Duration,
};

/// This `OpData` is a carrier for a function that polls for
/// readiness on a source.
#[doc(hidden)]
pub struct OpData<'a> {
    pub(crate) slot: Option<PollingFn>,
    pub(crate) read: bool,
    pub(crate) write: bool,
    _marker: PhantomData<&'a ()>,
}

#[derive(Debug)]
pub(crate) struct Completion {
    /// The inner interface to the polling runtime.
    poller: Poller,
    /// A buffer for holding events.
    event_buffer: Mutex<Vec<PollEvent>>,
    /// The list of sources we have to mind.
    sources: Mutex<Sources>,
    /// Deferred events that succeeded on the first try.
    deferred: Mutex<Vec<Event>>,
}

#[derive(Debug)]
struct Sources {
    /// List of sources and their data, indexed by the key.
    sources: Slab<SourceEntry>,
    /// Reverses the `sources` slab, mapping raw FDs to their keys.
    fd_to_key: HashMap<Raw, usize>,
}

#[derive(Debug)]
struct SourceEntry {
    /// The ongoing list of operations.
    operations: Vec<OpEntry>,
    /// Is this source currently registered as readable?
    readable: bool,
    /// Is this source currently registered as writable?
    writable: bool,
    /// The raw source for this entry.
    source: Raw,
}

struct OpEntry {
    /// The function used to poll for readiness.
    poll: PollingFn,
    /// The key for the source.
    key: u64,
    /// Do we poll for read readiness?
    read: bool,
    /// Do we poll for write readiness?
    write: bool,
}

impl fmt::Debug for OpEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpEntry")
            .field("key", &self.key)
            .field("read", &self.read)
            .field("write", &self.write)
            .finish_non_exhaustive()
    }
}

impl Completion {
    pub(crate) fn new(capacity: usize) -> Result<Self> {
        Ok(Self {
            poller: Poller::new()?,
            event_buffer: Mutex::new(Vec::with_capacity(capacity)),
            sources: Mutex::new(Sources {
                sources: Slab::new(),
                fd_to_key: HashMap::new(),
            }),
            deferred: Mutex::new(vec![]),
        })
    }

    pub(crate) fn register<S: Source>(&self, source: &S) -> Result<()> {
        assert!(
            S::SOURCE_TYPE != SourceType::File,
            "File sources are not supported on this platform"
        );
        let raw = source.as_raw();

        // get the key for the source as we create an entry
        let mut sources = lock!(self.sources);
        let key = sources.sources.insert(SourceEntry {
            operations: Vec::new(),
            readable: false,
            writable: false,
            source: raw,
        });

        // also allow reversing the source
        if sources.fd_to_key.insert(raw, key).is_some() {
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        } else {
            Ok(())
        }
    }

    pub(crate) fn deregister(&self, source: &impl Source) -> Result<()> {
        let mut sources = lock!(self.sources);
        let key = match sources.fd_to_key.remove(&source.as_raw()) {
            Some(key) => key,
            None => return Ok(()),
        };

        sources.sources.remove(key);
        Ok(())
    }

    pub(crate) fn submit(&self, op: &mut impl Op, key: u64) -> Result<()> {
        let mut sources = lock!(self.sources);

        // get the source entry for the raw FD
        let raw = op.source();
        let poll_key = *sources
            .fd_to_key
            .get(&raw)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let entry = sources.sources.get_mut(poll_key).unwrap();

        // populate an OpData structure
        let mut op_data = OpData {
            slot: None,
            read: false,
            write: false,
            _marker: PhantomData,
        };

        #[cfg(target_os = "linux")]
        let mut op_data = crate::OpData::Polling(op_data);

        op.run(&mut op_data)?;

        let mut new_op = match op_data {
            #[cfg(target_os = "linux")]
            crate::OpData::Polling(OpData {
                slot: Some(poll),
                read,
                write,
                ..
            }) => OpEntry {
                poll,
                key,
                read,
                write,
            },
            #[cfg(not(target_os = "linux"))]
            OpData {
                slot: Some(poll),
                read,
                write,
                ..
            } => OpEntry {
                poll,
                key,
                read,
                write,
            },
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "No polling function provided",
                ))
            }
        };

        // poll the operation once to see if we even need to register
        // the source for polling
        match (new_op.poll)() {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            result => {
                // it successfully resolved on the first try
                // so we don't need to register the source for polling
                let mut deferred = lock!(self.deferred);
                deferred.push(Event { key, result });

                // notify the poller that we may already have new events
                self.poller.notify()?;

                return Ok(());
            }
        }

        // add the operation to the source entry
        let mut register = false;
        if !entry.readable && new_op.read {
            register = true;
            entry.readable = true;
        }
        if !entry.writable && new_op.write {
            register = true;
            entry.writable = true;
        }

        if register {
            // we need to re-register this source into the poller
            self.poller.add(
                raw,
                PollEvent {
                    key: poll_key,
                    readable: entry.readable,
                    writable: entry.writable,
                },
            )?;
        }

        entry.operations.push(new_op);

        Ok(())
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>, out: &mut Vec<Event>) -> Result<usize> {
        // begin waiting for events
        let mut poll_events = lock!(self.event_buffer);
        self.poller.wait(&mut poll_events, timeout)?;

        // process the events
        let mut sources = lock!(self.sources);
        let mut num_events = 0;
        for event in poll_events.drain(..) {
            // match the event to a source entry
            let poll_key = event.key;
            let entry = sources.sources.get_mut(poll_key).unwrap();

            // clear the flags
            entry.readable = false;
            entry.writable = false;
            let mut register = false;

            // poll the operations to see which ones are ready
            for i in (0..entry.operations.len()).rev() {
                let op = &mut entry.operations[i];
                match (op.poll)() {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // blocked, re-run the operation
                        entry.readable |= op.read;
                        entry.writable |= op.write;
                        register = true;
                    }
                    result => {
                        // resolved to a final result, return it
                        let op = entry.operations.swap_remove(i);
                        out.push(Event {
                            key: op.key,
                            result,
                        });
                        num_events += 1;
                    }
                }
            }

            // register again if we need to
            if register {
                self.poller.add(
                    entry.source,
                    PollEvent {
                        key: poll_key,
                        readable: entry.readable,
                        writable: entry.writable,
                    },
                )?;
            }
        }

        // see if we had any deferred events while waiting
        num_events += self.try_deferred_events(out);

        Ok(num_events)
    }

    pub(crate) fn notify(&self) -> Result<()> {
        self.poller.notify()
    }

    fn try_deferred_events(&self, out: &mut Vec<Event>) -> usize {
        let mut deferred = lock!(self.deferred);
        if deferred.is_empty() {
            return 0;
        }

        let num_events = deferred.len();
        out.append(&mut deferred);
        num_events
    }
}
