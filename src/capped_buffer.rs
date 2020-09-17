use bytes::BufMut;
use std::ops::Deref;
use std::io;

pub struct CappedBuffer {
    buf: Vec<u8>,
    max: usize,
}

impl CappedBuffer {
    pub fn new(mut capacity: usize, max: usize) -> Self {
        if capacity > max {
            capacity = max;
        }

        Self {
            buf: Vec::with_capacity(capacity),
            max,
        }
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.max - self.buf.len()
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
        let mut bytes = self.buf.bytes_mut();

        if bytes.len() > remaining {
            bytes = &mut bytes[..remaining];
        }

        bytes
    }
}
