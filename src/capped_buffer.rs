use bytes::BufMut;
use std::ops::Deref;
use std::io;

/// Safe wrapper around Vec<u8> with custom `bytes::BufMut` and `std::io::Write`
/// implementations that ensure the buffer never exceeds maximum capacity.
pub struct CappedBuffer {
    buf: Vec<u8>,
    max: usize,
}

impl CappedBuffer {
    /// Create a new `CappedBuffer` with initial `capacity`, and a limit
    /// capacity set to `max`.
    pub fn new(mut capacity: usize, max: usize) -> Self {
        if capacity > max {
            capacity = max;
        }

        Self {
            buf: Vec::with_capacity(capacity),
            max,
        }
    }

    /// Remaining amount of bytes that can be written to the buffer
    /// before reaching max capacity
    #[inline]
    pub fn remaining(&self) -> usize {
        self.max - self.buf.len()
    }

    /// Shift the content of the buffer to the left by `shift`,
    /// effectively forgetting the shifted out bytes.
    /// New length of the buffer will be adjusted accordingly.
    pub fn shift(&mut self, shift: usize) {
        if shift >= self.buf.len() {
            self.buf.clear();
            return;
        }

        let src = self.buf[shift..].as_ptr();
        let dst = self.buf.as_mut_ptr();
        let new_len = self.buf.len() - shift;

        // This is a simple, potentially overlapping memcpy within
        // the buffer, shifting `new_len` bytes at offset `shift` (`src`)
        // to the beginning of the buffer (`dst`)
        unsafe {
            std::ptr::copy(src, dst, new_len);
            self.buf.set_len(new_len);
        }
    }
}

impl AsRef<[u8]> for CappedBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.buf
    }
}

impl AsMut<[u8]> for CappedBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Deref for CappedBuffer {
    type Target = Vec<u8>;

    fn deref(&self) -> &Vec<u8> {
        &self.buf
    }
}

impl io::Write for CappedBuffer {
    fn write(&mut self, mut buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.remaining() {
            buf = &buf[..self.remaining()];
        }
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if buf.len() <= self.remaining() {
            self.buf.extend_from_slice(buf);
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Exceeded maximum buffer capacity"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buf.flush()
    }
}

impl BufMut for CappedBuffer {
    fn remaining_mut(&self) -> usize {
        self.remaining()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        assert!(cnt <= self.remaining(), "Exceeded buffer capacity");

        self.buf.advance_mut(cnt);
    }

    unsafe fn bytes_mut(&mut self) -> &mut [u8] {
        let remaining = self.remaining();

        // `self.buf.bytes_mut` does an implicit allocation
        if remaining == 0 {
            return &mut [];
        }

        let mut bytes = self.buf.bytes_mut();

        if bytes.len() > remaining {
            bytes = &mut bytes[..remaining];
        }

        bytes
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;
    use super::*;

    #[test]
    fn shift() {
        let mut buffer = CappedBuffer::new(10, 20);

        buffer.write_all(b"Hello World").unwrap();
        buffer.shift(6);

        assert_eq!(&*buffer, b"World");
        assert_eq!(buffer.remaining(), 15);
    }

    #[test]
    fn shift_zero() {
        let mut buffer = CappedBuffer::new(10, 20);

        buffer.write_all(b"Hello World").unwrap();
        buffer.shift(0);

        assert_eq!(&*buffer, b"Hello World");
        assert_eq!(buffer.remaining(), 9);
    }

    #[test]
    fn shift_all() {
        let mut buffer = CappedBuffer::new(10, 20);

        buffer.write_all(b"Hello World").unwrap();
        buffer.shift(11);

        assert_eq!(&*buffer, b"");
        assert_eq!(buffer.remaining(), 20);
    }

    #[test]
    fn shift_capacity() {
        let mut buffer = CappedBuffer::new(10, 20);

        buffer.write_all(b"Hello World").unwrap();
        buffer.shift(20);

        assert_eq!(&*buffer, b"");
        assert_eq!(buffer.remaining(), 20);
    }

    #[test]
    fn shift_over_capacity() {
        let mut buffer = CappedBuffer::new(10, 20);

        buffer.write_all(b"Hello World").unwrap();
        buffer.shift(50);

        assert_eq!(&*buffer, b"");
        assert_eq!(buffer.remaining(), 20);
    }
}
