// GNU GPL v3 License

use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Bound, Deref, DerefMut, RangeBounds},
    ptr::NonNull,
};

mod iovec;
pub use iovec::OwnedIoSlice;

/// A buffer type that can be used to write data of some kind
/// to a source.
///
/// # Safety
///
/// Buffer must be consistent and valid.
pub unsafe trait Buf: 'static {
    /// Get the pointer and valid length of this buffer.
    fn pointer(&self) -> NonNull<[u8]>;

    /// Get a sliced version of this buffer.
    fn slice(self, bounds: impl RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        let len = unsafe { &*self.pointer().as_ptr() }.len();

        let start = match bounds.start_bound() {
            Bound::Unbounded => 0,
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
        };

        let end = match bounds.end_bound() {
            Bound::Unbounded => len,
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
        };

        assert!(
            start <= end,
            "start ({}) must be less than or equal to end ({})",
            start,
            end,
        );

        assert!(
            end <= len,
            "end ({}) must be less than or equal to length ({})",
            end,
            len,
        );

        Slice {
            buf: self,
            start,
            end,
        }
    }
}

unsafe impl<T: Deref<Target = [u8]> + 'static> Buf for T {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(&**self)
    }
}

/// A buffer type that can be used to read data from a source.
///
/// # Safety
///
/// The `pointer` returned by this buffer may be used mutably.
pub unsafe trait BufMut: Buf {}

unsafe impl<T: DerefMut<Target = [u8]> + 'static> BufMut for T {}

/// A buffer type that is ABI compatible with `std::io::IoSlice`.
///
/// # Safety
///
/// In addition to fulfilling the same contracts as `Buf`, `IoBuf`
/// must also be able to be safely transmuted to `IoSlice`.
pub unsafe trait IoBuf: Buf {}

unsafe impl IoBuf for IoSlice<'static> {}
unsafe impl IoBuf for IoSliceMut<'static> {}
unsafe impl IoBuf for OwnedIoSlice {}

/// A mutable buffer type that is ABI compatible with `std::io::IoSliceMut`.
///
/// # Safety
///
/// In addition to fulfilling the same contracts as `BufMut`, `IoBufMut`
/// must also be able to be safely transmuted to `IoSliceMut`.
pub unsafe trait IoBufMut: BufMut + IoBuf {}

unsafe impl IoBufMut for IoSliceMut<'static> {}
unsafe impl IoBufMut for OwnedIoSlice {}

// Internal use trait for a slice of I/O bufs.
#[doc(hidden)]
pub unsafe trait IoBufSlice {
    type Item: IoBuf;
    fn slice_ptr(&self) -> &[Self::Item];
}
#[doc(hidden)]
pub unsafe trait IoBufSliceMut: IoBufSlice {
    fn slice_ptr_mut(&mut self) -> &mut [Self::Item];
}
unsafe impl<Item: IoBuf> IoBufSlice for [Item] {
    type Item = Item;
    fn slice_ptr(&self) -> &[Self::Item] {
        self
    }
}
unsafe impl<Item: IoBufMut> IoBufSliceMut for [Item] {
    fn slice_ptr_mut(&mut self) -> &mut [Self::Item] {
        self
    }
}

/// A buffer made up of I/O slices, for vectored I/O.
///
/// # Safety
///
/// The pointer must be consistent.
pub unsafe trait VectoredBuf {
    /// The type of I/O buffer this buffer is made up of.
    type InnerBuf: IoBuf;

    /// Get the pointer and valid length of this buffer.
    fn pointer(&self) -> NonNull<[Self::InnerBuf]>;

    /// Get a sliced version of this buffer.
    ///
    /// Note that the slice indices operate on bytes instead
    /// of the inner buffers.
    fn slice(self, bounds: impl RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        let bufs = unsafe { &*self.pointer().as_ptr() };
        let len = bufs
            .iter()
            .map(|b| unsafe { &*b.pointer().as_ref() }.len())
            .fold(0usize, |acc, l| acc.saturating_add(l));

        let start = match bounds.start_bound() {
            Bound::Unbounded => 0,
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
        };

        let end = match bounds.end_bound() {
            Bound::Unbounded => len,
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
        };

        assert!(
            start <= end,
            "start ({}) must be less than or equal to end ({})",
            start,
            end,
        );

        assert!(
            end <= len,
            "end ({}) must be less than or equal to length ({})",
            end,
            len,
        );

        Slice {
            buf: self,
            start,
            end,
        }
    }
}

unsafe impl<T: Deref + 'static> VectoredBuf for T
where
    T::Target: IoBufSlice,
{
    type InnerBuf = <T::Target as IoBufSlice>::Item;

    fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
        NonNull::from((&**self).slice_ptr())
    }
}

/// Same as `VectoredBuf`, but mutable.
///
/// # Safety
///
/// The pointer must be able to be used mutably.
pub unsafe trait VectoredBufMut: VectoredBuf {}

unsafe impl<T: DerefMut + 'static> VectoredBufMut for T where T::Target: IoBufSliceMut {}

/// A wrapper around a `Buf` that only returns a specific slice of
/// the data.
pub struct Slice<T> {
    buf: T,
    start: usize,
    end: usize,
}

unsafe impl<T: Buf> Buf for Slice<T> {
    fn pointer(&self) -> NonNull<[u8]> {
        let ptr = self.buf.pointer().as_ptr();
        let mut ptr = ptr as *mut u8;

        // get offsets
        ptr = unsafe { ptr.add(self.start) };
        let len = self.end - self.start;
        unsafe { NonNull::new_unchecked(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
    }
}

unsafe impl<T: BufMut> BufMut for Slice<T> {}
