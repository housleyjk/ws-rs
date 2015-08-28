use std::default::Default;
use std::io::{Cursor, Write};
use std::mem::transmute;

use sha1;
use rand;
use url;
use httparse;

use result::{Result, Error, Kind};

static WS_GUID: &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
static BASE64: &'static [u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn generate_key() -> String {
    let key: [u8; 16] = unsafe {
        transmute(rand::random::<(u64, u64)>())
    };
    encode_base64(&key)
}

fn hash_key(key: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    let mut buf = [0u8; 20];

    hasher.update(key);
    hasher.update(WS_GUID.as_bytes());
    hasher.output(&mut buf);

    return encode_base64(&buf)
}

/// This code is based on rustc_serialize base64 STANDARD
fn encode_base64(data: &[u8]) -> String {
    let len = data.len();
    let mod_len = len % 3;

    let mut encoded = vec![b'='; (len + 2) / 3 * 4];
    {
        let mut in_iter = data[..len - mod_len].iter().map(|&c| c as u32);
        let mut out_iter = encoded.iter_mut();

        let enc = |val| BASE64[val as usize];
        let mut write = |val| *out_iter.next().unwrap() = val;

        while let (Some(one), Some(two), Some(three)) = (in_iter.next(), in_iter.next(), in_iter.next()) {
            let g24 = one << 16 | two << 8 | three;
            write(enc((g24 >> 18) & 63));
            write(enc((g24 >> 12) & 63));
            write(enc((g24 >> 6 ) & 63));
            write(enc((g24 >> 0 ) & 63));
        }

        match mod_len {
            1 => {
                let pad = (data[len-1] as u32) << 16;
                write(enc((pad >> 18) & 63));
                write(enc((pad >> 12) & 63));
            }
            2 => {
                let pad = (data[len-2] as u32) << 16 | (data[len-1] as u32) << 8;
                write(enc((pad >> 18) & 63));
                write(enc((pad >> 12) & 63));
                write(enc((pad >> 6) & 63));
            }
            _ => (),
        }
    }

    String::from_utf8(encoded).unwrap()
}

#[derive(Debug)]
pub struct Handshake {
    pub request: Request,
    pub response: Response,
}

impl Handshake {

    pub fn new(req: Request, res: Response) -> Handshake {
        Handshake {
            request: req,
            response: res,
        }
    }

    pub fn from_url(url: &url::Url) -> Result<Handshake> {
        debug!("Building handshake request from url {:?}", url.serialize());
        let mut shake = Handshake::default();
        try!(write!(
            shake.request.buffer(),
            "GET {url} HTTP/1.1\r\n\
             Connection: Upgrade\r\n\
             Host: {host}:{port}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Upgrade: websocket\r\n\r\n",
            url=url.serialize(),
            host=try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection."))),
            port=url.port_or_default().unwrap_or(80),
            key=generate_key()
        ));
        Ok(shake)
    }

}

impl Default for Handshake {

    fn default() -> Handshake {
        Handshake::new(
            Request::default(),
            Response::default(),
        )
    }
}


#[derive(Debug)]
pub struct Request {
    raw: Cursor<Vec<u8>>,
}

impl Request {

    #[inline]
    pub fn new(data: Vec<u8>) -> Request {
        Request {
            raw: Cursor::new(data),
        }
    }

    #[inline]
    pub fn buffer(&mut self) -> &mut Vec<u8> {
        self.raw.get_mut()
    }

    #[inline]
    pub fn cursor(&mut self) -> &mut Cursor<Vec<u8>> {
        &mut self.raw
    }

    pub fn parse(&self) -> Result<Option<String>> {
        let mut headers = [httparse::EMPTY_HEADER; 124];
        let mut req = httparse::Request::new(&mut headers);
        let parsed = try!(req.parse(self.raw.get_ref()));
        if !parsed.is_partial() {
            // TODO: parse all data out
            debug!(
                "Handshake request received: \n{}",
                String::from_utf8_lossy(self.raw.get_ref()));

            // for now just get key
            let header = try!(req.headers.iter()
                                         .find(|h| h.name == "Sec-WebSocket-Key")
                                         .ok_or(Error::new(Kind::Protocol, "Unable to parse WebSocket key.")));
            let key = header.value;
            Ok(Some(hash_key(key)))
        } else {
            Ok(None)
        }
    }

}

impl Default for Request {

    fn default() -> Request {
        Request::new(Vec::with_capacity(2048))
    }
}

#[derive(Debug)]
pub struct Response {
    raw: Cursor<Vec<u8>>,
}

impl Response {

    pub fn new(data: Vec<u8>) -> Response {
        Response {
            raw: Cursor::new(data),
        }
    }

    pub fn buffer(&mut self) -> &mut Vec<u8> {
        self.raw.get_mut()
    }

    pub fn cursor(&mut self) -> &mut Cursor<Vec<u8>> {
        &mut self.raw
    }

    pub fn parse(&self) -> Result<Option<&[u8]>> {
        let data = self.raw.get_ref();
        let end = data.iter()
                      .enumerate()
                      .take_while(|&(ind, _)| !data[..ind].ends_with(b"\r\n\r\n"))
                      .count();
        if !data[..end].ends_with(b"\r\n\r\n") {
            return Ok(None)
        }

        let res_data = &data[..end];

        let mut headers = [httparse::EMPTY_HEADER; 124];
        let mut res = httparse::Response::new(&mut headers);

        let parsed = try!(res.parse(res_data));
        if !parsed.is_partial() {
            // TODO: parse all data out
            debug!(
                "Handshake response received: \n{}",
                String::from_utf8_lossy(res_data));

            // TODO: verify key
            let header = try!(res.headers.iter()
                                         .find(|h| h.name == "Sec-WebSocket-Accept")
                                         .ok_or(Error::new(Kind::Protocol, "Unable to parse WebSocket accept.")));
            let key = header.value;

            // parse was successful, the rest of the data is incoming frames
            Ok(Some(&data[end..]))
        } else {
            Ok(None)
        }

    }
}

impl Default for Response {

    fn default() -> Response {
        Response::new(Vec::with_capacity(1024))
    }
}

