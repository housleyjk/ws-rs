use bytes::BufMut;
use std::ops::{Deref, DerefMut};
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

impl DerefMut for CappedBuffer {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }
}

impl io::Write for CappedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() < self.remaining_mut() {
            self.buf.write(buf)
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Input exceeds buffer capacity"))
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
        self.buf.advance_mut(cnt);
    }

    unsafe fn bytes_mut(&mut self) -> &mut [u8] {
        self.buf.bytes_mut()
    }
}
