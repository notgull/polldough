// GNU GPL v3 License

use std::{
    io::{IoSlice, IoSliceMut},
    ops::{Bound, RangeBounds},
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

unsafe impl Buf for &'static [u8] {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(*self)
    }
}
unsafe impl Buf for &'static mut [u8] {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(&**self)
    }
}
unsafe impl Buf for IoSlice<'static> {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(self.as_ref())
    }
}
unsafe impl Buf for IoSliceMut<'static> {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(self.as_ref())
    }
}
unsafe impl Buf for Vec<u8> {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(self.as_slice())
    }
}
unsafe impl Buf for Box<[u8]> {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(&**self)
    }
}
unsafe impl Buf for OwnedIoSlice {
    fn pointer(&self) -> NonNull<[u8]> {
        NonNull::from(self.as_ref())
    }
}

/// A buffer type that can be used to read data from a source.
///
/// # Safety
///
/// The `pointer` returned by this buffer may be used mutably.
pub unsafe trait BufMut: Buf {}

unsafe impl BufMut for &'static mut [u8] {}
unsafe impl BufMut for IoSliceMut<'static> {}
unsafe impl BufMut for Vec<u8> {}
unsafe impl BufMut for OwnedIoSlice {}

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
}

unsafe impl<T: IoBuf> VectoredBuf for &'static [T] {
    type InnerBuf = T;
    fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
        NonNull::from(*self)
    }
}
unsafe impl<T: IoBuf> VectoredBuf for &'static mut [T] {
    type InnerBuf = T;
    fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
        NonNull::from(&**self)
    }
}
unsafe impl<T: IoBuf> VectoredBuf for Vec<T> {
    type InnerBuf = T;
    fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
        NonNull::from(self.as_slice())
    }
}
unsafe impl<T: IoBuf> VectoredBuf for Box<[T]> {
    type InnerBuf = T;
    fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
        NonNull::from(&**self)
    }
}

/// Same as `VectoredBuf`, but mutable.
///
/// # Safety
///
/// The pointer must be able to be used mutably.
pub unsafe trait VectoredBufMut: VectoredBuf {}

unsafe impl<T: IoBufMut> VectoredBufMut for &'static mut [T] {}
unsafe impl<T: IoBufMut> VectoredBufMut for Vec<T> {}
unsafe impl<T: IoBufMut> VectoredBufMut for Box<[T]> {}

macro_rules! impl_array {
    ($($N:expr)+) => {
        $(
            unsafe impl Buf for [u8; $N] {
                fn pointer(&self) -> NonNull<[u8]> {
                    NonNull::from(self)
                }
            }
            unsafe impl BufMut for [u8; $N] {}
            unsafe impl<T: IoBuf> VectoredBuf for [T; $N] {
                type InnerBuf = T;
                fn pointer(&self) -> NonNull<[Self::InnerBuf]> {
                    NonNull::from(self)
                }
            }
            unsafe impl<T: IoBufMut> VectoredBufMut for [T; $N] {}
        )+
    };
}

impl_array! {
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
    64 128 256 512 1024 2048 4096 8192 16384 32768 65536 131072 262144 524288 1048576
}

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
