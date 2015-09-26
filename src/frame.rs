use std::mem::transmute;
use std::io::{Cursor, Read, Write};
use std::default::Default;
use std::iter::FromIterator;

use rand;
use mio::TryRead;

use result::{Result, Error, Kind};
use protocol::{OpCode, CloseCode};

fn apply_mask(buf: &mut [u8], mask: &[u8; 4]) {
    let iter = buf.iter_mut().zip(mask.iter().cycle());
    for (byte, &key) in iter {
        *byte ^= key
    }
}

#[inline]
fn generate_mask() -> [u8; 4] {
    unsafe { transmute(rand::random::<u32>()) }
}

// TODO: implement display for Frame
#[derive(Debug)]
pub struct Frame {
    finished: bool,
    rsv1: bool,
    rsv2: bool,
    rsv3: bool,
    opcode: OpCode,

    length: u64,
    mask: Option<[u8; 4]>,

    payload: Vec<u8>,

    header_length: u8,
}

impl Frame {

    #[inline]
    pub fn is_final(&self) -> bool {
        self.finished
    }

    #[inline]
    pub fn has_rsv1(&self) -> bool {
        self.rsv1
    }

    #[inline]
    pub fn has_rsv2(&self) -> bool {
        self.rsv2
    }

    #[inline]
    pub fn has_rsv3(&self) -> bool {
        self.rsv3
    }

    #[inline]
    pub fn opcode(&self) -> OpCode {
        self.opcode
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.length as usize + self.header_length as usize
    }

    #[allow(dead_code)]
    #[inline]
    pub fn payload_len(&self) -> u64 {
        self.length
    }

    #[allow(dead_code)]
    #[inline]
    pub fn header_len(&self) -> u8 {
        self.header_length
    }

    #[inline]
    pub fn is_masked(&self) -> bool {
        self.mask.is_some()
    }

    #[inline]
    pub fn mask(&mut self) -> &mut Frame {
        self.mask = Some(generate_mask());
        self
    }

    pub fn into_data(mut self) -> Vec<u8> {
        self.mask.and_then(|mask| {
            Some(apply_mask(&mut self.payload, &mask))
        });
        self.payload
    }

    #[inline]
    pub fn message(data: Vec<u8>, code: OpCode, finished: bool) -> Frame {
        debug_assert!(match code {
            OpCode::Text | OpCode::Binary | OpCode::Continue => true,
            _ => false,
        }, "Invalid opcode for data frame.");

        Frame {
            finished: finished,
            opcode: code,
            length: data.len() as u64,
            payload: data,
            .. Frame::default()
        }
    }

    #[inline]
    pub fn pong(data: Vec<u8>) -> Frame {
        Frame {
            opcode: OpCode::Pong,
            length: data.len() as u64,
            payload: data,
            .. Frame::default()
        }
    }

    #[inline]
    pub fn ping(data: Vec<u8>) -> Frame {
        Frame {
            opcode: OpCode::Ping,
            length: data.len() as u64,
            payload: data,
            .. Frame::default()
        }
    }

    #[inline]
    pub fn close(code: CloseCode, reason: &str) -> Frame {
        let raw: [u8; 2] = unsafe {
            let u: u16 = code.into();
            transmute(u.to_be())
        };

        let payload = if let CloseCode::Empty = code {
            Vec::new()
        } else {
            Vec::from_iter(
                raw[..].iter()
                       .chain(reason.as_bytes().iter())
                       .map(|&b| b))
        };

        Frame {
            length: payload.len() as u64,
            payload: payload,
            .. Frame::default()
        }
    }

    /// Parse the input stream into a frame.
    pub fn parse(cursor: &mut Cursor<Vec<u8>>, size: &mut usize) -> Result<Option<Frame>> {
        let initial = cursor.position();
        trace!("Position in buffer {}", initial);

        let mut head = [0u8; 2];
        if try!(cursor.read(&mut head)) != 2 {
            cursor.set_position(initial);
            return Ok(None)
        }

        trace!("Headers {:?}", head);

        let first = head[0];
        let second = head[1];
        trace!("{:b}", first);
        trace!("{:b}", second);

        let finished = first & 0x80 != 0;

        let rsv1 = first & 0x40 != 0;
        let rsv2 = first & 0x20 != 0;
        let rsv3 = first & 0x10 != 0;

        let opcode = OpCode::from(first & 0x0F);
        trace!("Opcode {:?}", opcode);
        if let OpCode::Bad = opcode {
            return Err(Error::new(Kind::Protocol, format!("Encountered invalid opcode: {}", first & 0x0F)))
        }
        let masked = second & 0x80 != 0;
        trace!("Masked {:?}", masked);

        let mut header_length = 2;

        let mut length = (second & 0x7F) as u64;

        if length == 126 {
            let mut length_bytes = [0u8; 2];
            if try!(cursor.read(&mut length_bytes)) != 2 {
                cursor.set_position(initial);
                return Ok(None)
            }

            length = unsafe {
                let mut wide: u16 = transmute(length_bytes);
                wide = u16::from_be(wide);
                wide
            } as u64;
            header_length += 2;
        } else if length == 127 {
            let mut length_bytes = [0u8; 8];
            if try!(cursor.read(&mut length_bytes)) != 8 {
                cursor.set_position(initial);
                return Ok(None)
            }

            unsafe { length = transmute(length_bytes); }
            length = u64::from_be(length);
            header_length += 8;
        }
        trace!("Payload length {}", length);

        // control frames must have length <= 125
        match opcode {
            OpCode::Close | OpCode::Ping | OpCode::Pong if length > 125 => {
                return Err(Error::new(Kind::Protocol, format!("Rejected WebSocket handshake.Received control frame with length: {}.", length)))
            }
            _ => ()
        }

        let mask = if masked {
            let mut mask_bytes = [0u8; 4];
            if try!(cursor.read(&mut mask_bytes)) != 4 {
                cursor.set_position(initial);
                return Ok(None)
            } else {
                header_length += 4;
                Some(mask_bytes)
            }
        } else {
            None
        };

        if *size < length as usize + header_length {
            cursor.set_position(initial);
            return Ok(None)
        }

        let mut data = Vec::with_capacity(length as usize);
        if length > 0 {
            // It is ok to unwrap here because this cursor won't block
            let read = try!(cursor.try_read_buf(&mut data)).unwrap();
            debug_assert!(read == length as usize, "Read incorrect payload length!");
        }

        let frame = Frame {
            finished: finished,
            rsv1: rsv1,
            rsv2: rsv2,
            rsv3: rsv3,
            opcode: opcode,
            length: length,
            mask: mask,
            payload: data,
            header_length: header_length as u8,
            .. Frame::default()
        };
        *size -= frame.len();
        Ok(Some(frame))
    }

    pub fn format<'w, W>(&mut self, w: &'w mut W) -> Result<()>
        where W: Write
    {
        let mut one = 0u8;
        let code: u8 = self.opcode.into();
        if self.is_final() {
            one |= 0x80;
        }
        if self.has_rsv1() {
            one |= 0x40;
        }
        if self.has_rsv2() {
            one |= 0x20;
        }
        if self.has_rsv3() {
            one |= 0x10;
        }
        one |= code;

        let mut two = 0u8;

        if self.is_masked() {
            two |= 0x80;
        }

        if self.length < 126 {
            two |= self.length as u8;
            let headers = [one, two];
            trace!("Headers {:?}", headers);
            try!(w.write(&headers));
        } else if self.length <= 65535 {
            two |= 126;
            let length_bytes: [u8; 2] = unsafe {
                let short = self.length as u16;
                transmute(short.to_be())
            };
            let headers = [one, two, length_bytes[0], length_bytes[1]];
            trace!("Headers {:?}", headers);
            try!(w.write(&headers));
        } else {
            two |= 127;
            let length_bytes: [u8; 8] = unsafe {
                transmute(self.length.to_be())
            };
            let headers = [
                one,
                two,
                length_bytes[0],
                length_bytes[1],
                length_bytes[2],
                length_bytes[3],
                length_bytes[4],
                length_bytes[5],
                length_bytes[6],
                length_bytes[7],
            ];
            trace!("Headers {:?}", headers);
            try!(w.write(&headers));
        }

        if self.is_masked() {
            let mask = self.mask.take().unwrap();
            apply_mask(&mut self.payload, &mask);
            try!(w.write(&mask));
        }

        try!(w.write(&self.payload));
        Ok(())
    }
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            finished: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: OpCode::Close,
            length: 0,
            mask: None,
            payload: Vec::new(),
            header_length: 0,
        }
    }
}

