use std::default::Default;
use std::io::{Cursor, Write};
use std::mem::transmute;
use std::str::from_utf8;
use std::net::SocketAddr;

use log::LogLevel::Trace as TraceLevel;

use sha1;
use rand;
use url;
use httparse;

use result::{Result, Error, Kind};

static WS_GUID: &'static str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
static BASE64: &'static [u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const MAX_HEADERS: usize = 124;

fn generate_key() -> String {
    let key: [u8; 16] = unsafe {
        transmute(rand::random::<(u64, u64)>())
    };
    encode_base64(&key)
}

pub fn hash_key(key: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    let mut buf = [0u8; 20];

    hasher.update(key);
    hasher.update(WS_GUID.as_bytes());
    hasher.output(&mut buf);

    encode_base64(&buf)
}

// This code is based on rustc_serialize base64 STANDARD
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
            write(enc(g24 & 63));
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

/// A struct representing the two halves of the WebSocket handshake.
#[derive(Debug)]
pub struct Handshake {
    /// The HTTP request sent to begin the handshake.
    pub request: Request,
    /// The HTTP response from the server confirming the handshake.
    pub response: Response,
    /// The socket address of the other endpoint. This address may
    /// be an intermediary such as a proxy server.
    pub peer_addr: Option<SocketAddr>,
    /// The socket address of this enpoint.
    pub local_addr: Option<SocketAddr>,
}

impl Handshake {

    pub fn new(req: Request, res: Response) -> Handshake {
        Handshake {
            request: req,
            response: res,
            .. Default::default()
        }
    }

    /// Get the IP address of the remote connection.
    ///
    /// This is the preferred method of obtaining the client's IP address.
    /// It will attempt to retrieve the most likely IP address based on request
    /// headers, falling back to the address of the peer.
    ///
    /// # Note
    /// This assumes that the peer is a client. If you are implementing a
    /// WebSocket client and want to obtain the address of the server, use
    /// `Handshake::peer_addr` instead.
    ///
    /// This method does not ensure that the address is a valid IP address.
    #[allow(dead_code)]
    pub fn remote_addr(&self) -> Result<Option<String>> {
        Ok(try!(self.request.client_addr()).map(String::from).or_else(|| {
            if let Some(addr) = self.peer_addr {
                Some(addr.to_string())
            } else {
                None
            }
        }))
    }

}

impl Default for Handshake {

    fn default() -> Handshake {
        Handshake {
            request: Request::default(),
            response: Response::default(),
            peer_addr: None,
            local_addr: None,
        }
    }
}


/// The handshake request.
#[derive(Debug)]
pub struct Request {
    raw: Cursor<Vec<u8>>,
}

impl Request {

    fn get_header(&self, header: &str) -> Result<Option<&[u8]>> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        try!(req.parse(self.raw.get_ref()));

        Ok(req.headers.iter()
                      .find(|h| h.name.to_lowercase() == header)
                      .map(|h| h.value))
    }

    /// Get the origin of the request if it comes from a browser.
    #[allow(dead_code)]
    // note this assumes that the request is complete
    pub fn origin(&self) -> Result<Option<&str>> {
        if let Some(origin) = try!(self.get_header("origin")) {
            Ok(Some(try!(from_utf8(origin))))
        } else {
            Ok(None)
        }
    }

    /// Get the WebSocket key sent in the request.
    // note this assumes that the request is complete
    pub fn key(&self) -> Result<&[u8]> {
        try!(self.get_header("sec-websocket-key")).ok_or(Error::new(Kind::Protocol, "Unable to parse WebSocket key."))
    }

    /// Get the WebSocket protocol version from the request (should be 13).
    #[allow(dead_code)]
    // note this assumes that the request is complete
    pub fn version(&self) -> Result<&str> {
        if let Some(version) = try!(self.get_header("sec-websocket-version")) {
            from_utf8(version).map_err(Error::from)
        } else {
            Err(Error::new(Kind::Protocol, "The Sec-WebSocket-Version header is missing."))
        }
    }

    /// Get the path of the request.
    #[allow(dead_code)]
    // note this assumes that the request is complete
    pub fn resource(&self) -> Result<&str> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        try!(req.parse(self.raw.get_ref()));
        req.path.ok_or(Error::new(Kind::Protocol, "The resource is missing. This is likely because the request is incomplete"))
    }

    /// Get the possible protocols the other endpoint supports.
    #[allow(dead_code)]
    pub fn protocols(&self) -> Result<Vec<&str>> {
        if let Some(protos) = try!(self.get_header("sec-websocket-protocol")) {
            Ok(try!(from_utf8(protos)).split(',').map(|proto| proto.trim()).collect())
        } else {
            Ok(Vec::new())
        }
    }

    /// Get the extensions that the other endpoint is trying to negotiate.
    #[allow(dead_code)]
    pub fn extensions(&self) -> Result<Vec<&str>> {
        if let Some(exts) = try!(self.get_header("sec-websocket-extensions")) {
            Ok(try!(from_utf8(exts)).split(',').map(|ext| ext.trim()).collect())
        } else {
            Ok(Vec::new())
        }
    }

    /// Access the request headers as a vector of tuples.
    #[allow(dead_code)]
    pub fn headers(&self) -> Result<Vec<(&str, &str)>> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        try!(req.parse(self.raw.get_ref()));

        let mut headers = Vec::with_capacity(MAX_HEADERS);
        for header in req.headers.iter() {
            headers.push((header.name, try!(from_utf8(header.value))))
        }
        Ok(headers)
    }

    /// Get the IP address of the client.
    ///
    /// This method will attempt to retrieve the most likely IP address of the requester
    /// in the following manner:
    ///
    /// If the `X-Forwarded-For` header exists, this method will return the left most
    /// address in the list.
    ///
    /// If the [Forwarded HTTP Header Field](https://tools.ietf.org/html/rfc7239) exits,
    /// this method will return the left most address indicated by the `for` parameter,
    /// if it exists.
    ///
    /// # Note
    /// This method does not ensure that the address is a valid IP address.
    #[allow(dead_code)]
    pub fn client_addr(&self) -> Result<Option<&str>> {
        if let Some(x_forward) = try!(self.get_header("x-forwarded-for")) {
            return Ok(try!(from_utf8(x_forward)).split(',').next())
        }

        // We only care about the first forwarded header, so get_header is ok
        if let Some(forward) = try!(self.get_header("forwarded")) {
            if let Some(_for) = try!(from_utf8(forward))
                .split(';')
                .find(|f| f.trim().starts_with("for"))
            {
                if let Some(_for_eq) = _for.trim().split(',').next() {
                    let mut it = _for_eq.split('=');
                    it.next();
                    return Ok(it.next())
                }
            }
        }
        Ok(None)
    }

    #[doc(hidden)]
    #[inline]
    pub fn new(data: Vec<u8>) -> Request {
        Request {
            raw: Cursor::new(data),
        }
    }

    #[doc(hidden)]
    #[inline]
    pub fn buffer(&mut self) -> &mut Vec<u8> {
        self.raw.get_mut()
    }

    #[doc(hidden)]
    #[inline]
    pub fn cursor(&mut self) -> &mut Cursor<Vec<u8>> {
        &mut self.raw
    }

    #[doc(hidden)]
    pub fn parse(&self) -> Result<Option<String>> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut req = httparse::Request::new(&mut headers);
        let parsed = try!(req.parse(self.raw.get_ref()));
        if !parsed.is_partial() {
            debug!(
                "Handshake request received: \n{}",
                String::from_utf8_lossy(self.raw.get_ref()));

            let key = try!(req.headers.iter()
                                      .find(|h| h.name.to_lowercase() == "sec-websocket-key")
                                      .map(|h| h.value)
                                      .ok_or(Error::new(Kind::Protocol, "Unable to parse WebSocket key.")));
            Ok(Some(hash_key(key)))
        } else {
            Ok(None)
        }
    }

    #[doc(hidden)]
    pub fn from_url(url: &url::Url, protocols: Option<&str>, extensions: Option<&str>) -> Result<Request> {
        debug!("Building handshake request from url {:?}", url.serialize());
        let mut req = Request::default();
        if let Some(proto) = protocols {
            if let Some(ext) = extensions {
                try!(write!(
                    req.buffer(),
                    "GET {path}{query} HTTP/1.1\r\n\
                     Connection: Upgrade\r\n\
                     Host: {host}:{port}\r\n\
                     Sec-WebSocket-Version: 13\r\n\
                     Sec-WebSocket-Key: {key}\r\n\
                     Sec-WebSocket-Protocol: {proto}\r\n\
                     Sec-WebSocket-Extensions: {ext}\r\n\
                     Upgrade: websocket\r\n\r\n",
                    path=url.serialize_path().unwrap_or("/".to_owned()),
                    query=url.query.clone().and_then(|query| Some(format!("?{}", query))).unwrap_or("".to_owned()),
                    host=try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection."))),
                    port=url.port_or_default().unwrap_or(80),
                    key=generate_key(),
                    proto=proto,
                    ext=ext,
                ));
            } else {
                try!(write!(
                    req.buffer(),
                    "GET {path}{query} HTTP/1.1\r\n\
                     Connection: Upgrade\r\n\
                     Host: {host}:{port}\r\n\
                     Sec-WebSocket-Version: 13\r\n\
                     Sec-WebSocket-Key: {key}\r\n\
                     Sec-WebSocket-Protocol: {proto}\r\n\
                     Upgrade: websocket\r\n\r\n",
                    path=url.serialize_path().unwrap_or("/".to_owned()),
                    query=url.query.clone().and_then(|query| Some(format!("?{}", query))).unwrap_or("".to_owned()),
                    host=try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection."))),
                    port=url.port_or_default().unwrap_or(80),
                    key=generate_key(),
                    proto=proto,
                ));
            }

        } else {
            if let Some(ext) = extensions {
                try!(write!(
                    req.buffer(),
                    "GET {path}{query} HTTP/1.1\r\n\
                     Connection: Upgrade\r\n\
                     Host: {host}:{port}\r\n\
                     Sec-WebSocket-Version: 13\r\n\
                     Sec-WebSocket-Key: {key}\r\n\
                     Sec-WebSocket-Extensions: {ext}\r\n\
                     Upgrade: websocket\r\n\r\n",
                    path=url.serialize_path().unwrap_or("/".to_owned()),
                    query=url.query.clone().and_then(|query| Some(format!("?{}", query))).unwrap_or("".to_owned()),
                    host=try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection."))),
                    port=url.port_or_default().unwrap_or(80),
                    key=generate_key(),
                    ext=ext,
                ));
            } else {
                try!(write!(
                    req.buffer(),
                    "GET {path}{query} HTTP/1.1\r\n\
                     Connection: Upgrade\r\n\
                     Host: {host}:{port}\r\n\
                     Sec-WebSocket-Version: 13\r\n\
                     Sec-WebSocket-Key: {key}\r\n\
                     Upgrade: websocket\r\n\r\n",
                    path=url.serialize_path().unwrap_or("/".to_owned()),
                    query=url.query.clone().and_then(|query| Some(format!("?{}", query))).unwrap_or("".to_owned()),
                    host=try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection."))),
                    port=url.port_or_default().unwrap_or(80),
                    key=generate_key(),
                ));
            }
        }

        if log_enabled!(TraceLevel) {
            use std::io::Read;
            let mut req_string = String::with_capacity(req.buffer().len());
            try!(req.cursor().read_to_string(&mut req_string));
            trace!("{}", req_string);
            req.cursor().set_position(0);
        }
        Ok(req)
    }

}

impl Default for Request {

    fn default() -> Request {
        Request::new(Vec::with_capacity(2048))
    }
}

/// The handshake response.
#[derive(Debug)]
pub struct Response {
    raw: Cursor<Vec<u8>>,
}

impl Response {

    fn get_header(&self, header: &str) -> Result<Option<&[u8]>> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut res = httparse::Response::new(&mut headers);
        try!(res.parse(self.raw.get_ref()));

        Ok(res.headers.iter()
                      .find(|h| h.name.to_lowercase() == header)
                      .map(|h| h.value))
    }

    /// Get the status of the response
    #[allow(dead_code)]
    pub fn status(&self) -> Result<u16> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut res = httparse::Response::new(&mut headers);
        try!(res.parse(self.raw.get_ref()));
        res.code.ok_or(Error::new(Kind::Protocol, "Unable to parse HTTP status."))
    }

    /// Get the hashed WebSocket key
    pub fn key(&self) -> Result<&[u8]> {
        try!(self.get_header("sec-websocket-accept")).ok_or(Error::new(Kind::Protocol, "Unable to parse WebSocket key."))
    }

    /// Get the protocol that the server has decided to use
    #[allow(dead_code)]
    pub fn protocol(&self) -> Result<Option<&str>> {
        if let Some(proto) = try!(self.get_header("sec-websocket-protocol")) {
            Ok(Some(try!(from_utf8(proto))))
        } else {
            Ok(None)
        }
    }

    /// Get the extensions that the server has decided to use. If these are unacceptable, it is
    /// appropriate to send an Extension close code
    #[allow(dead_code)]
    pub fn extensions(&self) -> Result<Vec<&str>> {
        if let Some(exts) = try!(self.get_header("sec-websocket-extensions")) {
            Ok(try!(from_utf8(exts)).split(',').map(|proto| proto.trim()).collect())
        } else {
            Ok(Vec::new())
        }
    }

    #[doc(hidden)]
    #[inline]
    pub fn new(data: Vec<u8>) -> Response {
        Response {
            raw: Cursor::new(data),
        }
    }

    #[doc(hidden)]
    #[inline]
    pub fn buffer(&mut self) -> &mut Vec<u8> {
        self.raw.get_mut()
    }

    #[doc(hidden)]
    #[inline]
    pub fn cursor(&mut self) -> &mut Cursor<Vec<u8>> {
        &mut self.raw
    }

    #[doc(hidden)]
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

        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut res = httparse::Response::new(&mut headers);

        let parsed = try!(res.parse(res_data));
        if !parsed.is_partial() {
            debug!(
                "Handshake response received: \n{}",
                String::from_utf8_lossy(res_data));

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

mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use std::io::Write;
    use super::*;

    #[test]
    fn test_remote_addr_x_forwarded_for() {
        let mut shake = Handshake::default();
        shake.request = Request::default();
        write!(
            shake.request.buffer(),
            "GET / HTTP/1.1\r\n\
            Connection: Upgrade\r\n\
            Upgrade: websocket\r\n\
            X-Forwarded-For: 192.168.1.1, 192.168.1.2, 192.168.1.3\r\n\
            Sec-WebSocket-Version: 13\r\n\
            Sec-WebSocket-Key: q16eN37NCfVwUChPvBdk4g==\r\n\r\n").unwrap();
        assert_eq!(shake.remote_addr().unwrap().unwrap(), "192.168.1.1");
    }

    #[test]
    fn test_remote_addr_forwarded() {
        let mut shake = Handshake::default();
        shake.request = Request::default();
        write!(
            shake.request.buffer(),
            "GET / HTTP/1.1\r\n\
            Connection: Upgrade\r\n\
            Upgrade: websocket\r\n\
            Forwarded: by=192.168.1.1; for=192.0.2.43, for=\"[2001:db8:cafe::17]\", for=unknown
            Sec-WebSocket-Version: 13\r\n\
            Sec-WebSocket-Key: q16eN37NCfVwUChPvBdk4g==\r\n\r\n").unwrap();
        assert_eq!(shake.remote_addr().unwrap().unwrap(), "192.0.2.43");
    }
}
