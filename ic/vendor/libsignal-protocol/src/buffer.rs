use std::{
    cmp::{Ord, Ordering},
    fmt::{self, Debug, Formatter},
    io::{self, Write},
    mem,
    ops::{Index, IndexMut},
};

/// A byte buffer (e.g. `Vec<u8>`).
pub struct Buffer {
    raw: *mut sys::signal_buffer,
}

impl Buffer {
    /// Create a new empty buffer.
    pub fn new() -> Buffer {
        Buffer::with_capacity(0)
    }

    pub(crate) unsafe fn from_raw(raw: *mut sys::signal_buffer) -> Buffer {
        assert!(!raw.is_null());
        Buffer { raw }
    }

    /// Create a new buffer with the provided size.
    pub fn with_capacity(capacity: usize) -> Buffer {
        unsafe { Buffer::from_raw(sys::signal_buffer_alloc(capacity)) }
    }

    /// How many bytes are in this buffer?
    pub fn len(&self) -> usize {
        unsafe { sys::signal_buffer_len(self.raw) }
    }

    /// Is the buffer empty?
    pub fn is_empty(&self) -> bool {
        self.len() > 0
    }

    /// Extract the underlying raw pointer.
    ///
    /// # Note
    ///
    /// It is the user's responsibility to ensure the buffer is later free'd
    /// (e.g. with [`Buffer::from_raw`] or [`sys::signal_buffer_free`]).
    pub fn into_raw(self) -> *mut sys::signal_buffer {
        let raw = self.raw;
        mem::forget(self);
        raw
    }

    /// Get an immutable reference to the underlying data.
    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            let ptr = sys::signal_buffer_data(self.raw);
            assert!(!ptr.is_null());
            std::slice::from_raw_parts(ptr, self.len())
        }
    }

    /// Get a mutable reference to the underlying data.
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe {
            let len = self.len();
            let ptr = sys::signal_buffer_data(self.raw);
            assert!(!ptr.is_null());
            std::slice::from_raw_parts_mut(ptr, len)
        }
    }

    /// Append some data to this buffer.
    ///
    /// # Note
    ///
    /// Every append results in a re-allocation of the underlying buffer.
    pub fn append(&mut self, data: &[u8]) {
        unsafe {
            self.raw = sys::signal_buffer_append(self.raw, data.as_ptr(), data.len());
        }
    }
}

impl Ord for Buffer {
    fn cmp(&self, other: &Buffer) -> Ordering {
        let ret = unsafe { sys::signal_buffer_compare(self.raw, other.raw) };

        if ret < 0 {
            Ordering::Less
        } else if ret > 0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl PartialOrd for Buffer {
    fn partial_cmp(&self, other: &Buffer) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Buffer) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Buffer {}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for Buffer {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(other: Vec<u8>) -> Buffer {
        Buffer::from(other.as_slice())
    }
}

impl<'a> From<&'a [u8]> for Buffer {
    fn from(other: &'a [u8]) -> Buffer {
        unsafe { Buffer::from_raw(sys::signal_buffer_create(other.as_ptr(), other.len())) }
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_slice_mut()
    }
}

impl<T> Index<T> for Buffer
where
    [u8]: Index<T>,
{
    type Output = <[u8] as Index<T>>::Output;

    fn index(&self, ix: T) -> &Self::Output {
        self.as_slice().index(ix)
    }
}

impl<T> IndexMut<T> for Buffer
where
    [u8]: IndexMut<T>,
{
    fn index_mut(&mut self, ix: T) -> &mut Self::Output {
        self.as_slice_mut().index_mut(ix)
    }
}

impl Write for Buffer {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.append(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Buffer {
        unsafe {
            let raw = sys::signal_buffer_copy(self.raw);
            assert!(!raw.is_null());
            Buffer { raw }
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            sys::signal_buffer_free(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_delete() {
        let buffer = Buffer::new();
        drop(buffer);
    }

    #[test]
    fn create_with_capacity() {
        let cap = 12345;
        let buffer = Buffer::with_capacity(cap);
        assert_eq!(buffer.len(), cap);
    }

    #[test]
    fn get_an_item() {
        let mut buffer = Buffer::with_capacity(128);
        buffer[10] = 0xde;
        buffer[11] = 0xad;
        buffer[12] = 0xbe;
        buffer[13] = 0xef;

        let dead_beef = &buffer[10..14];
        assert_eq!(dead_beef, &[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn write_to_a_buffer() {
        let mut buffer = Buffer::new();

        write!(buffer, "Hello").unwrap();
        write!(buffer, ",").unwrap();
        writeln!(buffer, " World!").unwrap();

        let got = std::str::from_utf8(buffer.as_slice()).unwrap();
        assert_eq!("Hello, World!\n", got);
    }
}
