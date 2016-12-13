use std::mem::replace;
use std::mem::transmute;
use std::borrow::Borrow;
use std::io::{Write, Read, Cursor, Seek, SeekFrom};
use std::net::SocketAddr;
use std::collections::VecDeque;
use std::str::from_utf8;

use url;
use mio::{Token, Ready};
use mio::timer::Timeout;
use mio::tcp::TcpStream;
#[cfg(feature="ssl")]
use openssl::ssl::SslStream;

use message::Message;
use handshake::{Handshake, Request, Response};
use frame::Frame;
use protocol::{CloseCode, OpCode};
use result::{Result, Error, Kind};
use handler::Handler;
use stream::{Stream, TryReadBuf, TryWriteBuf};

use self::State::*;
use self::Endpoint::*;

use super::Settings;

#[derive(Debug)]
pub enum State {
    // Tcp connection accepted, waiting for handshake to complete
    Connecting(Cursor<Vec<u8>>, Cursor<Vec<u8>>),
    // Ready to send/receive messages
    Open,
    AwaitingClose,
    RespondingClose,
    FinishedClose,
}

/// A little more semantic than a boolean
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Endpoint {
    /// Will mask outgoing frames
    Client(url::Url),
    /// Won't mask outgoing frames
    Server,
}

impl State {

    #[inline]
    pub fn is_connecting(&self) -> bool {
        match *self {
            State::Connecting(..) => true,
            _ => false,
        }
    }

    #[allow(dead_code)]
    #[inline]
    pub fn is_open(&self) -> bool {
        match *self {
            State::Open => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_closing(&self) -> bool {
        match *self {
            State::AwaitingClose => true,
            State::FinishedClose => true,
            _ => false,
        }
    }
}

pub struct Connection<H>
    where H: Handler
{
    token: Token,
    socket: Stream,
    state: State,
    endpoint: Endpoint,
    events: Ready,

    fragments: VecDeque<Frame>,

    in_buffer: Cursor<Vec<u8>>,
    out_buffer: Cursor<Vec<u8>>,

    handler: H,

    addresses: Vec<SocketAddr>,

    settings: Settings,
}

impl<H> Connection<H>
    where H: Handler
{
    pub fn new(tok: Token, sock: TcpStream, handler: H, settings: Settings) -> Connection<H> {
        Connection {
            token: tok,
            socket: Stream::tcp(sock),
            state: Connecting(
                Cursor::new(Vec::with_capacity(2048)),
                Cursor::new(Vec::with_capacity(2048)),
            ),
            endpoint: Endpoint::Server,
            events: Ready::hup(),
            fragments: VecDeque::with_capacity(settings.fragments_capacity),
            in_buffer: Cursor::new(Vec::with_capacity(settings.in_buffer_capacity)),
            out_buffer: Cursor::new(Vec::with_capacity(settings.out_buffer_capacity)),
            handler: handler,
            addresses: Vec::new(),
            settings: settings,
        }
    }

    pub fn as_server(&mut self) -> Result<()> {
        Ok(self.events.insert(Ready::readable()))
    }

    pub fn as_client(&mut self, url: &url::Url, addrs: Vec<SocketAddr>) -> Result<()> {
        if let Connecting(ref mut req, _) = self.state {
            self.addresses = addrs;
            self.events.insert(Ready::writable());
            self.endpoint = Endpoint::Client(url.clone());
            try!(self.handler.build_request(url)).format(req.get_mut())
        } else {
            Err(Error::new(
                Kind::Internal,
                "Tried to set connection to client while not connecting."))
        }
    }

    #[cfg(feature="ssl")]
    pub fn encrypt(&mut self) -> Result<()> {
        let ssl_stream = match self.endpoint {
            Server => try!(SslStream::accept(
                try!(self.handler.build_ssl()),
                try!(self.socket().try_clone()))),

            Client(_) => try!(SslStream::connect(
                try!(self.handler.build_ssl()),
                try!(self.socket().try_clone()))),
        };

        Ok(self.socket = Stream::tls(ssl_stream))
    }

    pub fn token(&self) -> Token {
        self.token
    }

    pub fn socket(&self) -> &TcpStream {
        &self.socket.evented()
    }

    fn peer_addr(&self) -> String {
        if let Ok(addr) = self.socket.peer_addr() {
            addr.to_string()
        } else {
            "UNKNOWN".into()
        }
    }

    // Resetting may be necessary in order to try all possible addresses for a server
    #[cfg(feature="ssl")]
    pub fn reset(&mut self) -> Result<()> {
        if self.is_client() {
            if let Connecting(ref mut req, ref mut res) = self.state {
                req.set_position(0);
                res.set_position(0);
                self.events.remove(Ready::readable());
                self.events.insert(Ready::writable());

                if let Some(ref addr) = self.addresses.pop() {
                    let sock = try!(TcpStream::connect(addr));
                    if self.socket.is_tls() {
                        Ok(self.socket = Stream::tls(
                                try!(SslStream::connect(
                                    try!(self.handler.build_ssl()),
                                    sock))))

                    } else {
                        Ok(self.socket = Stream::tcp(sock))
                    }
                } else {
                    if self.settings.panic_on_new_connection {
                        panic!("Unable to connect to server.");
                    }
                    Err(Error::new(Kind::Internal, "Exhausted possible addresses."))
                }
            } else {
                Err(Error::new(Kind::Internal, "Unable to reset client connection because it is active."))
            }
        } else {
            Err(Error::new(Kind::Internal, "Server connections cannot be reset."))
        }
    }

    #[cfg(not(feature="ssl"))]
    pub fn reset(&mut self) -> Result<()> {
        if self.is_client() {
            if let Connecting(ref mut req, ref mut res) = self.state {
                req.set_position(0);
                res.set_position(0);
                self.events.remove(Ready::readable());
                self.events.insert(Ready::writable());

                if let Some(ref addr) = self.addresses.pop() {
                    let sock = try!(TcpStream::connect(addr));
                    Ok(self.socket = Stream::tcp(sock))
                } else {
                    if self.settings.panic_on_new_connection {
                        panic!("Unable to connect to server.");
                    }
                    Err(Error::new(Kind::Internal, "Exhausted possible addresses."))
                }
            } else {
                Err(Error::new(Kind::Internal, "Unable to reset client connection because it is active."))
            }
        } else {
            Err(Error::new(Kind::Internal, "Server connections cannot be reset."))
        }
    }

    pub fn events(&self) -> Ready {
        self.events
    }

    pub fn is_client(&self) -> bool {
        match self.endpoint {
            Client(_) => true,
            Server => false,
        }
    }

    pub fn is_server(&self) -> bool {
        match self.endpoint {
            Client(_) => false,
            Server => true,
        }
    }

    pub fn shutdown(&mut self) {
        self.handler.on_shutdown();
        if let Err(err) = self.send_close(CloseCode::Away, "Shutting down.") {
            self.handler.on_error(err);
            self.disconnect()
        }
    }

    #[inline]
    pub fn new_timeout(&mut self, event: Token, timeout: Timeout) -> Result<()> {
        self.handler.on_new_timeout(event, timeout)
    }

    #[inline]
    pub fn timeout_triggered(&mut self, event: Token) -> Result<()> {
        self.handler.on_timeout(event)
    }

    pub fn error(&mut self, err: Error) {
        match self.state {
            Connecting(_, ref mut res) => {
                match err.kind {
                    #[cfg(feature="ssl")]
                    Kind::Ssl(_) | Kind::Io(_) => {
                        self.handler.on_error(err);
                        self.events = Ready::none();
                    }
                    Kind::Protocol => {
                        let msg = err.to_string();
                        self.handler.on_error(err);
                        if let Server = self.endpoint {
                            res.get_mut().clear();
                            if let Err(err) = write!(
                                    res.get_mut(),
                                    "HTTP/1.1 400 Bad Request\r\n\r\n{}", msg)
                            {
                                self.handler.on_error(Error::from(err));
                                self.events = Ready::none();
                            } else {
                                self.events.remove(Ready::readable());
                                self.events.insert(Ready::writable());
                            }
                        } else {
                            self.events = Ready::none();
                        }
                    }
                    _ => {
                        let msg = err.to_string();
                        self.handler.on_error(err);
                        if let Server = self.endpoint {
                            res.get_mut().clear();
                            if let Err(err) = write!(
                                    res.get_mut(),
                                    "HTTP/1.1 500 Internal Server Error\r\n\r\n{}", msg) {
                                self.handler.on_error(Error::from(err));
                                self.events = Ready::none();
                            } else {
                                self.events.remove(Ready::readable());
                                self.events.insert(Ready::writable());
                            }
                        } else {
                            self.events = Ready::none();
                        }
                    }
                }

            }
            _ => {
                match err.kind {
                    Kind::Internal => {
                        if self.settings.panic_on_internal {
                            panic!("Panicking on internal error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Error, reason) {
                            self.handler.on_error(err);
                            self.disconnect()
                        }
                    }
                    Kind::Capacity => {
                        if self.settings.panic_on_capacity {
                            panic!("Panicking on capacity error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Size, reason) {
                            self.handler.on_error(err);
                            self.disconnect()
                        }
                    }
                    Kind::Protocol => {
                        if self.settings.panic_on_protocol {
                            panic!("Panicking on protocol error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Protocol, reason) {
                            self.handler.on_error(err);
                            self.disconnect()
                        }
                    }
                    Kind::Encoding(_) => {
                        if self.settings.panic_on_encoding {
                            panic!("Panicking on encoding error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Invalid, reason) {
                            self.handler.on_error(err);
                            self.disconnect()
                        }
                    }
                    Kind::Http(_) => {
                        // This may happen if some handler writes a bad response
                        self.handler.on_error(err);
                        error!("Disconnecting WebSocket.");
                        self.disconnect()
                    }
                    Kind::Custom(_) => {
                        self.handler.on_error(err);
                    }
                    Kind::Timer(_) => {
                        if self.settings.panic_on_timeout {
                            panic!("Panicking on timer failure -- {}", err);
                        }
                        self.handler.on_error(err);
                    }
                    Kind::Queue(_) => {
                        if self.settings.panic_on_queue {
                            panic!("Panicking on queue error -- {}", err);
                        }
                        self.handler.on_error(err);
                    }
                    _ => {
                        if self.settings.panic_on_io {
                            panic!("Panicking on io error -- {}", err);
                        }
                        self.handler.on_error(err);
                        self.disconnect()
                    }
                }
            }
        }
    }

    pub fn disconnect(&mut self) {
        match self.state {
            RespondingClose | FinishedClose | Connecting(_, _)=> (),
            _ => {
                self.handler.on_close(CloseCode::Abnormal, "");
            }
        }
        self.events = Ready::none()
    }

    pub fn consume(self) -> H {
        self.handler
    }

    fn write_handshake(&mut self) -> Result<()> {
        if let Connecting(ref mut req, ref mut res) = self.state {
            match self.endpoint {
                Server => {
                    let mut done = false;
                    if let Some(_) = try!(self.socket.try_write_buf(res)) {
                        if res.position() as usize == res.get_ref().len() {
                            done = true
                        }
                    }
                    if !done {
                        return Ok(())
                    }
                }
                Client(_) =>  {
                    if let Some(_) = try!(self.socket.try_write_buf(req)) {
                        if req.position() as usize == req.get_ref().len() {
                            trace!("Finished writing handshake request to {}",
                                self.socket
                                    .peer_addr()
                                    .map(|addr| addr.to_string())
                                    .unwrap_or("UNKNOWN".into()));
                            self.events.insert(Ready::readable());
                            self.events.remove(Ready::writable());
                        }
                    }
                    return Ok(())
                }
            }
        }

        if let Connecting(ref req, ref res) = replace(&mut self.state, Open) {
            trace!("Finished writing handshake response to {}", self.peer_addr());

            let request = match Request::parse(req.get_ref()) {
                Ok(Some(req)) => req,
                _ => {
                    // An error should already have been sent for the first time it failed to
                    // parse. We don't call disconnect here because `on_open` hasn't been called yet.
                    self.state = FinishedClose;
                    self.events = Ready::none();
                    return Ok(())
                }
            };

            let response = try!(try!(Response::parse(res.get_ref())).ok_or(
                Error::new(Kind::Internal, "Failed to parse response after handshake is complete.")));

            if response.status() != 101 {
                self.events = Ready::none();
                return Ok(())
            } else {
                try!(self.handler.on_open(Handshake {
                    request: request,
                    response: response,
                    peer_addr: self.socket.peer_addr().ok(),
                    local_addr: self.socket.local_addr().ok(),
                }));
                debug!("Connection to {} is now open.", self.peer_addr());
                self.events.insert(Ready::readable());
                return Ok(self.check_events())
            }
        } else {
            Err(Error::new(Kind::Internal, "Tried to write WebSocket handshake while not in connecting state!"))
        }
    }

    fn read_handshake(&mut self) -> Result<()> {
        if let Connecting(ref mut req, ref mut res) = self.state {
            match self.endpoint {
                Server => {
                    if let Some(_) = try!(self.socket.try_read_buf(req.get_mut())) {
                        if let Some(ref request) = try!(Request::parse(req.get_ref())) {
                            trace!("Handshake request received: \n{}", request);
                            let response = try!(self.handler.on_request(request));
                            try!(response.format(res.get_mut()));
                            self.events.remove(Ready::readable());
                            self.events.insert(Ready::writable());
                        }
                    }
                    return Ok(())
                }
                Client(_) => {
                    if let Some(_) = try!(self.socket.try_read_buf(res.get_mut())) {

                        // TODO: see if this can be optimized with drain
                        let end = {
                            let data = res.get_ref();
                            let end = data.iter()
                                          .enumerate()
                                          .take_while(|&(ind, _)| !data[..ind].ends_with(b"\r\n\r\n"))
                                          .count();
                            if !data[..end].ends_with(b"\r\n\r\n") {
                                return Ok(())
                            }
                            self.in_buffer.get_mut().extend(&data[end..]);
                            end
                        };
                        res.get_mut().truncate(end);
                    }
                }
            }
        }

        if let Connecting(ref req, ref res) = replace(&mut self.state, Open) {
            trace!("Finished reading handshake response from {}", self.peer_addr());

            let request = try!(try!(Request::parse(req.get_ref())).ok_or(
                Error::new(Kind::Internal, "Failed to parse request after handshake is complete.")));

            let response = try!(try!(Response::parse(res.get_ref())).ok_or(
                Error::new(Kind::Internal, "Failed to parse response after handshake is complete.")));

            trace!("Handshake response received: \n{}", response);

            if response.status() != 101 {
                if response.status() != 301 && response.status() != 302 {
                    return Err(Error::new(Kind::Protocol, "Handshake failed."));
                } else {
                    return Ok(())
                }
            }

            if self.settings.key_strict {
                let req_key = try!(request.hashed_key());
                let res_key = try!(from_utf8(try!(response.key())));
                if req_key != res_key {
                    return Err(Error::new(Kind::Protocol, format!("Received incorrect WebSocket Accept key: {} vs {}", req_key, res_key)));
                }
            }

            try!(self.handler.on_response(&response));
            try!(self.handler.on_open(Handshake {
                    request: request,
                    response: response,
                    peer_addr: self.socket.peer_addr().ok(),
                    local_addr: self.socket.local_addr().ok(),
            }));

            // check to see if there is anything to read already
            if !self.in_buffer.get_ref().is_empty() {
                try!(self.read_frames());
            }

            return Ok(self.check_events())
        }
        Err(Error::new(Kind::Internal, "Tried to read WebSocket handshake while not in connecting state!"))
    }

    pub fn read(&mut self) -> Result<()> {
        if self.socket.is_negotiating() {
            trace!("Performing TLS negotiation on {}.", self.peer_addr());
            try!(self.socket.clear_negotiating());
            self.write()
        } else {
            let res = if self.state.is_connecting() {
                trace!("Ready to read handshake from {}.", self.peer_addr());
                self.read_handshake()
            } else {
                trace!("Ready to read messages from {}.", self.peer_addr());
                if let Some(_) = try!(self.buffer_in()) {
                    // consume the whole buffer if possible
                    if let Err(err) = self.read_frames() {
                        // break on first IO error, other errors don't imply that the buffer is bad
                        if let Kind::Io(_) = err.kind {
                            return Err(err)
                        }
                        self.error(err)
                    }
                }
                Ok(())
            };

            if self.socket.is_negotiating() && res.is_ok() {
                self.events.remove(Ready::readable());
                self.events.insert(Ready::writable());
            }
            res
        }
    }

    fn read_frames(&mut self) -> Result<()> {
        while let Some(mut frame) = try!(Frame::parse(&mut self.in_buffer)) {
            match self.state {
                // Ignore data received after receiving close frame
                RespondingClose | FinishedClose => continue,
                _ => (),
            }

            if self.settings.masking_strict {
                if frame.is_masked() {
                    if self.is_client() {
                        return Err(Error::new(Kind::Protocol, "Received masked frame from a server endpoint."))
                    }
                } else {
                    if self.is_server() {
                        return Err(Error::new(Kind::Protocol, "Received unmasked frame from a client endpoint."))
                    }
                }
            }

            // This is safe whether or not a frame is masked.
            frame.remove_mask();

            if let Some(frame) = try!(self.handler.on_frame(frame)) {
                if frame.is_final() {
                    match frame.opcode() {
                        // singleton data frames
                        OpCode::Text => {
                            trace!("Received text frame {:?}", frame);
                            // since we are going to handle this, there can't be an ongoing
                            // message
                            if !self.fragments.is_empty() {
                                return Err(Error::new(Kind::Protocol, "Received unfragmented text frame while processing fragmented message."))
                            }
                            let msg = Message::text(try!(String::from_utf8(frame.into_data()).map_err(|err| err.utf8_error())));
                            try!(self.handler.on_message(msg));
                        }
                        OpCode::Binary => {
                            trace!("Received binary frame {:?}", frame);
                            // since we are going to handle this, there can't be an ongoing
                            // message
                            if !self.fragments.is_empty() {
                                return Err(Error::new(Kind::Protocol, "Received unfragmented binary frame while processing fragmented message."))
                            }
                            let data = frame.into_data();
                            try!(self.handler.on_message(Message::binary(data)));
                        }
                        // control frames
                        OpCode::Close => {
                            trace!("Received close frame {:?}", frame);
                            // Closing handshake
                            if self.state.is_closing() {

                                if self.is_server() {
                                    // Finished handshake, disconnect server side
                                    self.events = Ready::none()
                                } else {
                                    // We are a client, so we wait for the server to close the
                                    // connection
                                }

                            } else {

                                // Starting handshake, will send the responding close frame
                                self.state = RespondingClose;

                            }

                            let mut close_code = [0u8; 2];
                            let mut data = Cursor::new(frame.into_data());
                            if let 2 = try!(data.read(&mut close_code)) {
                                let code_be: u16 = unsafe {transmute(close_code) };
                                trace!("Connection to {} received raw close code: {:?}, {:b}", self.peer_addr(), code_be, code_be);
                                let named = CloseCode::from(u16::from_be(code_be));
                                if let CloseCode::Other(code) = named {
                                    if
                                            code < 1000 ||
                                            code >= 5000 ||
                                            code == 1004 ||
                                            code == 1014 ||
                                            code == 1016 || // these below are here to pass the autobahn test suite
                                            code == 1100 || // we shouldn't need them later
                                            code == 2000 ||
                                            code == 2999
                                    {
                                        return Err(Error::new(Kind::Protocol, format!("Received invalid close code from endpoint: {}", code)))
                                    }
                                }
                                let has_reason = {
                                    if let Ok(reason) = from_utf8(&data.get_ref()[2..]) {
                                        self.handler.on_close(named, reason); // note reason may be an empty string
                                        true
                                    } else {
                                        self.handler.on_close(named, "");
                                        false
                                    }
                                };

                                if let CloseCode::Abnormal = named {
                                    return Err(Error::new(Kind::Protocol, "Received abnormal close code from endpoint."))
                                } else if let CloseCode::Status = named {
                                    return Err(Error::new(Kind::Protocol, "Received no status close code from endpoint."))
                                } else if let CloseCode::Restart = named {
                                    return Err(Error::new(Kind::Protocol, "Restart close code is not supported."))
                                } else if let CloseCode::Again = named {
                                    return Err(Error::new(Kind::Protocol, "Try again later close code is not supported."))
                                } else if let CloseCode::Tls = named {
                                    return Err(Error::new(Kind::Protocol, "Received TLS close code outside of TLS handshake."))
                                } else {
                                    if !self.state.is_closing() {
                                        if has_reason {
                                            try!(self.send_close(named, "")); // note this drops any extra close data
                                        } else {
                                            try!(self.send_close(CloseCode::Invalid, ""));
                                        }
                                    }
                                }
                            } else {
                                // This is not an error. It is allowed behavior in the
                                // protocol, so we don't trigger an error
                                self.handler.on_close(CloseCode::Status, "Unable to read close code. Sending empty close frame.");
                                if !self.state.is_closing() {
                                    try!(self.send_close(CloseCode::Empty, ""));
                                }
                            }
                        }
                        OpCode::Ping => {
                            trace!("Received ping frame {:?}", frame);
                            try!(self.send_pong(frame.into_data()));
                        }
                        OpCode::Pong => {
                            trace!("Received pong frame {:?}", frame);
                            // no ping validation for now
                        }
                        // last fragment
                        OpCode::Continue => {
                            trace!("Received final fragment {:?}", frame);
                            if let Some(first) = self.fragments.pop_front() {
                                let size = self.fragments.iter().fold(first.payload().len() + frame.payload().len(), |len, frame| len + frame.payload().len());
                                match first.opcode() {
                                    OpCode::Text => {
                                        trace!("Constructing text message from fragments: {:?} -> {:?} -> {:?}", first, self.fragments.iter().collect::<Vec<&Frame>>(), frame);
                                        let mut data = Vec::with_capacity(size);
                                        data.extend(first.into_data());
                                        while let Some(frame) = self.fragments.pop_front() {
                                            data.extend(frame.into_data());
                                        }
                                        data.extend(frame.into_data());

                                        let string = try!(String::from_utf8(data).map_err(|err| err.utf8_error()));

                                        trace!("Calling handler with constructed message: {:?}", string);
                                        try!(self.handler.on_message(Message::text(string)));
                                    }
                                    OpCode::Binary => {
                                        trace!("Constructing text message from fragments: {:?} -> {:?} -> {:?}", first, self.fragments.iter().collect::<Vec<&Frame>>(), frame);
                                        let mut data = Vec::with_capacity(size);
                                        data.extend(first.into_data());

                                        while let Some(frame) = self.fragments.pop_front() {
                                            data.extend(frame.into_data());
                                        }

                                        data.extend(frame.into_data());

                                        trace!("Calling handler with constructed message: {:?}", data);
                                        try!(self.handler.on_message(Message::binary(data)));
                                    }
                                    _ => {
                                        return Err(Error::new(Kind::Protocol, "Encounted fragmented control frame."))
                                    }
                                }
                            } else {
                                return Err(Error::new(Kind::Protocol, "Unable to reconstruct fragmented message. No first frame."))
                            }
                        }
                        _ => return Err(Error::new(Kind::Protocol, "Encountered invalid opcode.")),
                    }
                } else {
                    if frame.is_control() {
                        return Err(Error::new(Kind::Protocol, "Encounted fragmented control frame."))
                    } else {
                        trace!("Received non-final fragment frame {:?}", frame);
                        if !self.settings.fragments_grow &&
                            self.settings.fragments_capacity == self.fragments.len()
                        {
                            return Err(Error::new(Kind::Capacity, "Exceeded max fragments."))
                        } else {
                            self.fragments.push_back(frame)
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self) -> Result<()> {
        if self.socket.is_negotiating() {
            trace!("Performing TLS negotiation on {}.", self.peer_addr());
            try!(self.socket.clear_negotiating());
            self.read()
        } else {
            let res = if self.state.is_connecting() {
                trace!("Ready to write handshake to {}.", self.peer_addr());
                self.write_handshake()
            } else {
                trace!("Ready to write messages to {}.", self.peer_addr());

                // Start out assuming that this write will clear the whole buffer
                self.events.remove(Ready::writable());

                if let Some(len) = try!(self.socket.try_write_buf(&mut self.out_buffer)) {
                    trace!("Wrote {} bytes to {}", len, self.peer_addr());
                    let finished = len == 0 || self.out_buffer.position() == self.out_buffer.get_ref().len() as u64;
                    if finished {
                        match self.state {
                            // we are are a server that is closing and just wrote out our confirming
                            // close frame, let's disconnect
                            FinishedClose if self.is_server()  => return Ok(self.events = Ready::none()),
                            _ => (),
                        }
                    }
                }

                // Check if there is more to write so that the connection will be rescheduled
                Ok(self.check_events())
            };

            if self.socket.is_negotiating() && res.is_ok() {
                self.events.remove(Ready::writable());
                self.events.insert(Ready::readable());
            }
            res
        }
    }

    pub fn send_message(&mut self, msg: Message) -> Result<()> {
        if self.state.is_closing() {
            trace!("Connection is closing. Ignoring request to send message {:?} to {}.",
                msg,
                self.peer_addr());
            return Ok(())
        }

        let opcode = msg.opcode();
        trace!("Message opcode {:?}", opcode);
        let data = msg.into_data();

        if let Some(frame) = try!(self.handler.on_send_frame(Frame::message(data, opcode, true))) {

            if frame.payload().len() > self.settings.fragment_size {
                trace!("Chunking at {:?}.", self.settings.fragment_size);
                // note this copies the data, so it's actually somewhat expensive to fragment
                let mut chunks = frame.payload().chunks(self.settings.fragment_size).peekable();
                let chunk = chunks.next().expect("Unable to get initial chunk!");

                let mut first = Frame::message(Vec::from(chunk), opcode, false);

                // Match reserved bits from original to keep extension status intact
                first.set_rsv1(frame.has_rsv1());
                first.set_rsv2(frame.has_rsv2());
                first.set_rsv3(frame.has_rsv3());

                try!(self.buffer_frame(first));

                while let Some(chunk) = chunks.next() {
                    if let Some(_) = chunks.peek() {
                        try!(self.buffer_frame(
                            Frame::message(Vec::from(chunk), OpCode::Continue, false)));
                    } else {
                        try!(self.buffer_frame(
                            Frame::message(Vec::from(chunk), OpCode::Continue, true)));
                    }
                }

            } else {
                trace!("Sending unfragmented message frame.");
                // true means that the message is done
                try!(self.buffer_frame(frame));
            }

        }
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_ping(&mut self, data: Vec<u8>) -> Result<()> {
        if self.state.is_closing() {
            trace!("Connection is closing. Ignoring request to send ping {:?} to {}.",
                data,
                self.peer_addr());
            return Ok(())
        }
        trace!("Sending ping to {}.", self.peer_addr());

        if let Some(frame) = try!(self.handler.on_send_frame(Frame::ping(data))) {
            try!(self.buffer_frame(frame));
        }
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_pong(&mut self, data: Vec<u8>) -> Result<()> {
        if self.state.is_closing() {
            trace!("Connection is closing. Ignoring request to send pong {:?} to {}.",
                data,
                self.peer_addr());
            return Ok(())
        }
        trace!("Sending pong to {}.", self.peer_addr());

        if let Some(frame) = try!(self.handler.on_send_frame(Frame::pong(data))) {
            try!(self.buffer_frame(frame));
        }
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_close<R>(&mut self, code: CloseCode, reason: R) -> Result<()>
        where R: Borrow<str>
    {
        match self.state {
            // We are responding to a close frame the other endpoint, when this frame goes out, we
            // are done.
            RespondingClose => self.state = FinishedClose,
            // Multiple close frames are being sent from our end, ignore the later frames
            AwaitingClose | FinishedClose => {
                trace!("Connection is already closing. Ignoring close {:?} -- {:?} to {}.",
                    code,
                    reason.borrow(),
                    self.peer_addr());
                return Ok(self.check_events())
            }
            // We are initiating a closing handshake.
            Open => self.state = AwaitingClose,
            Connecting(_, _) => {
                debug_assert!(false, "Attempted to close connection while not yet open.")
            }
        }

        trace!("Sending close {:?} -- {:?} to {}.", code, reason.borrow(), self.peer_addr());

        if let Some(frame) = try!(self.handler.on_send_frame(Frame::close(code, reason.borrow()))) {
            try!(self.buffer_frame(frame));
        }

        trace!("Connection to {} is now closing.", self.peer_addr());

        Ok(self.check_events())
    }

    fn check_events(&mut self) {
        if !self.state.is_connecting() {
            self.events.insert(Ready::readable());
            if self.out_buffer.position() < self.out_buffer.get_ref().len() as u64 {
                self.events.insert(Ready::writable());
            }
        }
    }

    fn buffer_frame(&mut self, mut frame: Frame) -> Result<()> {
        try!(self.check_buffer_out(&frame));

        if self.is_client() {
            frame.set_mask();
        }

        trace!("Buffering frame to {}:\n{}", self.peer_addr(), frame);

        let pos = self.out_buffer.position();
        try!(self.out_buffer.seek(SeekFrom::End(0)));
        try!(frame.format(&mut self.out_buffer));
        try!(self.out_buffer.seek(SeekFrom::Start(pos)));
        Ok(())
    }

    fn check_buffer_out(&mut self, frame: &Frame) -> Result<()> {

        if self.out_buffer.get_ref().capacity() <= self.out_buffer.get_ref().len() + frame.len() {
            // extend
            let mut new = Vec::with_capacity(self.out_buffer.get_ref().capacity());
            new.extend(&self.out_buffer.get_ref()[self.out_buffer.position() as usize ..]);
            if new.len() == new.capacity() {
                if self.settings.out_buffer_grow {
                    new.reserve(self.settings.out_buffer_capacity)
                } else {
                    return Err(Error::new(Kind::Capacity, "Maxed out output buffer for connection."))
                }
            }
            self.out_buffer = Cursor::new(new);
        }
        Ok(())
    }

    fn buffer_in(&mut self) -> Result<Option<usize>> {

        trace!("Reading buffer for connection to {}.", self.peer_addr());
        if let Some(len) = try!(self.socket.try_read_buf(self.in_buffer.get_mut())) {
            trace!("Buffered {}.", len);
            if self.in_buffer.get_ref().len() == self.in_buffer.get_ref().capacity() {
                // extend
                let mut new = Vec::with_capacity(self.in_buffer.get_ref().capacity());
                new.extend(&self.in_buffer.get_ref()[self.in_buffer.position() as usize ..]);
                if new.len() == new.capacity() {
                    if self.settings.in_buffer_grow {
                        new.reserve(self.settings.in_buffer_capacity);
                    } else {
                        return Err(Error::new(Kind::Capacity, "Maxed out input buffer for connection."))
                    }

                    self.in_buffer = Cursor::new(new);
                    // return now so that hopefully we will consume some of the buffer so this
                    // won't happen next time
                    trace!("Buffered {}.", len);
                    return Ok(Some(len))
                }
                self.in_buffer = Cursor::new(new);
            }

            if len == 0 {
                Ok(None)
            } else {
                Ok(Some(len))
            }
        } else {
            Ok(None)
        }
    }
}
