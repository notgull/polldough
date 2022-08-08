// GNU GPL v3 License

use crate::{OpData, Raw};

/// The hidden underlying trait for `Op` that is used to not expose OS-specific
/// details.
#[doc(hidden)]
pub trait OpBase {
    /// Enqueue this function into the completion queue, given an OS-specific
    /// "OpData" object.
    fn run(
        &self,
        op_data: &mut OpData,
    );
}

/// An operation that can be enqueued into the completion queue.
pub trait Op : OpBase {
    /// The raw file descriptor that this operation is assocaited with.
    fn source(&self) -> Raw;
}