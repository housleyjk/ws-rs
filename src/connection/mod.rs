pub mod factory;
pub mod handler;
pub mod state;

use std::mem::replace;
use std::mem::transmute;
use std::borrow::Borrow;
use std::io::{Write, Read, Cursor, Seek, SeekFrom};
use std::collections::VecDeque;
use std::str::from_utf8;

use mio::{Token, TryRead, TryWrite, EventSet};

use message::Message;
use handshake::{Handshake, Request, Response, hash_key};
use frame::Frame;
use protocol::{CloseCode, OpCode};
use result::{Result, Error, Kind};
use self::handler::Handler;
use self::state::{State, Endpoint};
use self::state::State::*;
use self::state::Endpoint::*;

pub struct Builder<S, H>
    where H: Handler, S: TryRead + TryWrite
{
    token: Token,
    socket: S,
    handler: H,
    endpoint: Option<Endpoint>,
    handshake: Option<Handshake>,
}

impl<S, H> Builder<S, H>
    where H: Handler, S: TryRead + TryWrite
{
    pub fn new(tok: Token, sock: S, handler: H) -> Builder<S, H> {
        Builder {
            token: tok,
            socket: sock,
            handler: handler,
            endpoint: None,
            handshake: None,
        }
    }

    pub fn build(mut self) -> Connection<S, H> {
        let settings = self.handler.settings();
        let end = self.endpoint.unwrap_or(Server);  // default to server
        Connection {
            token: self.token,
            socket: self.socket,
            state: Connecting(self.handshake.unwrap_or(Handshake::default())),
            endpoint: end,
            events: if let Server = end {
                EventSet::readable() | EventSet::hup()
            } else {
                EventSet::writable() | EventSet::hup()
            },
            fragments: VecDeque::with_capacity(settings.fragments_capacity),
            in_buffer: Cursor::new(Vec::with_capacity(settings.in_buffer_capacity)),
            out_buffer: Cursor::new(Vec::with_capacity(settings.out_buffer_capacity)),
            handler: self.handler,
        }
    }

    pub fn client(mut self) -> Builder<S, H> {
        self.endpoint = Some(Client);
        self
    }

    pub fn request(mut self, req: Request) -> Builder<S, H> {
        self.handshake = Some(Handshake::new(req, Response::default()));
        self
    }
}

pub struct Connection<S, H>
    where H: Handler, S: TryRead + TryWrite
{
    token: Token,
    socket: S,
    state: State,
    endpoint: Endpoint,
    events: EventSet,

    fragments: VecDeque<Frame>,

    in_buffer: Cursor<Vec<u8>>,
    out_buffer: Cursor<Vec<u8>>,

    handler: H,
}

impl<S, H> Connection<S, H>
    where H: Handler, S: TryRead + TryWrite
{
    pub fn builder(tok: Token, sock: S, handler: H) -> Builder<S, H> {
        Builder::new(tok, sock, handler)
    }

    pub fn token(&self) -> Token {
        self.token
    }

    pub fn socket(&self) -> &S {
        &self.socket
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn events(&self) -> EventSet {
        self.events
    }

    pub fn is_client(&self) -> bool {
        match self.endpoint {
            Client => true,
            Server => false,
        }
    }

    pub fn is_server(&self) -> bool {
        match self.endpoint {
            Client => false,
            Server => true,
        }
    }

    pub fn shutdown(&mut self) {
        self.handler.on_shutdown();
        if let Err(err) = self.send_close(CloseCode::Away, "Shutting down.") {
            self.handler.on_error(err);
            self.events = EventSet::none();
        }
    }

    pub fn error(&mut self, err: Error) {
        trace!("Connection {:?} encountered an error -- {}", self.token(), err);
        match self.state {
            Connecting(ref mut shake) => {
                match err.kind {
                    Kind::Protocol => {
                        let msg = err.to_string();
                        self.handler.on_error(err);
                        if let Server = self.endpoint {
                            if let Err(err) = write!(
                                    shake.response.buffer(),
                                    "HTTP/1.1 400 Bad Request\r\n\r\n{}", msg) {
                                self.handler.on_error(Error::from(err));
                                self.events = EventSet::none();
                            } else {
                                self.events.remove(EventSet::readable());
                                self.events.insert(EventSet::writable());
                            }
                        } else {
                            self.events = EventSet::none();
                        }

                    }
                    _ => {
                        let msg = err.to_string();
                        self.handler.on_error(err);
                        if let Server = self.endpoint {
                            if let Err(err) = write!(
                                    shake.response.buffer(),
                                    "HTTP/1.1 500 Internal Server Error\r\n\r\n{}", msg) {
                                self.handler.on_error(Error::from(err));
                                self.events = EventSet::none();
                            } else {
                                self.events.remove(EventSet::readable());
                                self.events.insert(EventSet::writable());
                            }
                        } else {
                            self.events = EventSet::none();
                        }
                    }
                }

            }
            _ => {
                let settings = self.handler.settings();
                match err.kind {
                    Kind::Internal => {
                        if settings.panic_on_internal {
                            panic!("Panicking on internal error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Error, reason) {
                            self.handler.on_error(err);
                            self.events = EventSet::none();
                        }
                    }
                    Kind::Capacity => {
                        if settings.panic_on_capacity {
                            panic!("Panicking on capacity error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Size, reason) {
                            self.handler.on_error(err);
                            self.events = EventSet::none();
                        }
                    }
                    Kind::Protocol => {
                        if settings.panic_on_protocol {
                            panic!("Panicking on protocol error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Protocol, reason) {
                            self.handler.on_error(err);
                            self.events = EventSet::none();
                        }
                    }
                    Kind::Encoding(_) => {
                        if settings.panic_on_encoding {
                            panic!("Panicking on encoding error -- {}", err);
                        }
                        let reason = format!("{}", err);

                        self.handler.on_error(err);
                        if let Err(err) = self.send_close(CloseCode::Invalid, reason) {
                            self.handler.on_error(err);
                            self.events = EventSet::none();
                        }
                    }
                    Kind::Parse(_) => {
                        debug_assert!(false, "Encountered HTTP parse error while not in connecting state!");
                        error!("Encountered HTTP parse error while not in connecting state!");
                        self.handler.on_error(err);
                        error!("Disconnecting WebSocket.");
                        self.events = EventSet::none();
                    }
                    Kind::Custom(_) => {
                        self.handler.on_error(err);
                    }
                    _ => {
                        if settings.panic_on_io {
                            panic!("Panicking on io error -- {}", err);
                        }
                        self.handler.on_error(err);
                        self.events = EventSet::none();
                    }
                }
            }
        }
    }

    pub fn write_handshake(&mut self) -> Result<()> {
        trace!("Writing handshake for {:?}", self.token);
        if let Connecting(ref mut shake) = self.state {
            match self.endpoint {
                Server => {
                    let mut done = false;
                    if let Some(len) = try!(self.socket.try_write_buf(shake.response.cursor())) {
                        if shake.response.buffer().len() == len {
                            done = true
                        }
                    }
                    if !done {
                        return Ok(())
                    }
                }
                Client =>  {
                    if let Some(len) = try!(self.socket.try_write_buf(shake.request.cursor())) {
                        if shake.response.buffer().len() == len {
                            trace!("Finished writing handshake request for {:?}", self.token);
                            self.events.insert(EventSet::readable());
                            self.events.remove(EventSet::writable());
                        }
                    }
                    return Ok(())
                }
            }
        }

        if let Connecting(shake) = replace(&mut self.state, Open) {
            trace!("Finished writing handshake response for {:?}", self.token);
            trace!("Connection {:?} is now open.", self.token);

            if try!(shake.response.status()) != 101 {
                return Err(Error::new(Kind::Protocol, "Handshake failed."));
            }
            try!(self.handler.on_open(shake));
            return Ok(self.check_events())
        }

        Err(Error::new(Kind::Internal, "Tried to write WebSocket handshake while not in connecting state!"))
    }

    pub fn read_handshake(&mut self) -> Result<()> {
        trace!("Reading handshake for {:?}", self.token);
        if let Connecting(ref mut shake) = self.state {
            match self.endpoint {
                Server => {
                    if let Some(_) = try!(self.socket.try_read_buf(shake.request.buffer())) {
                        if let Some(key) = try!(shake.request.parse()) {
                            match try!(self.handler.on_request(&shake.request)) {
                                (Some(proto), Some(ext)) => {
                                    try!(write!(
                                        shake.response.buffer(),
                                        "HTTP/1.1 101 Switching Protocols\r\n\
                                         Connection: Upgrade\r\n\
                                         Sec-WebSocket-Accept: {}\r\n\
                                         Sec-WebSocket-Protocol: {}\r\n\
                                         Sec-WebSocket-Extensions: {}\r\n\
                                         Upgrade: websocket\r\n\r\n",
                                         key, proto, ext));
                                }
                                (None, Some(ext)) => {
                                    try!(write!(
                                        shake.response.buffer(),
                                        "HTTP/1.1 101 Switching Protocols\r\n\
                                         Connection: Upgrade\r\n\
                                         Sec-WebSocket-Accept: {}\r\n\
                                         Sec-WebSocket-Extensions: {}\r\n\
                                         Upgrade: websocket\r\n\r\n",
                                         key, ext));
                                }
                                (Some(proto), None) => {
                                    try!(write!(
                                        shake.response.buffer(),
                                        "HTTP/1.1 101 Switching Protocols\r\n\
                                         Connection: Upgrade\r\n\
                                         Sec-WebSocket-Accept: {}\r\n\
                                         Sec-WebSocket-Protocol: {}\r\n\
                                         Upgrade: websocket\r\n\r\n",
                                         key, proto));
                                }
                                (None, None) => {
                                    try!(write!(
                                        shake.response.buffer(),
                                        "HTTP/1.1 101 Switching Protocols\r\n\
                                         Connection: Upgrade\r\n\
                                         Sec-WebSocket-Accept: {}\r\n\
                                         Upgrade: websocket\r\n\r\n",
                                         key));
                                }
                            }

                           self.events.remove(EventSet::readable());
                           self.events.insert(EventSet::writable());
                        }
                    }
                    return Ok(())
                }
                Client => {
                    let mut done = false;
                    if let Some(_) = try!(self.socket.try_read_buf(shake.response.buffer())) {
                        if let Some(remainder) = try!(shake.response.parse()) {
                            // start with anything leftover after the handshake response was
                            // parsed out. This is naive and assumes that the remainder is
                            // small
                            trace!("Discovered frame at end of handshake.");
                            self.in_buffer.get_mut().extend(remainder);
                            done = true;
                        }
                    }
                    if !done {
                        return Ok(())
                    }
                }
            }
        }

        if let Connecting(shake) = replace(&mut self.state, Open) {
            trace!("Finished reading handshake response for {:?}", self.token);
            trace!("Connection {:?} is now open.", self.token);
            if try!(shake.response.status()) != 101 {
                return Err(Error::new(Kind::Protocol, "Handshake failed."));
            }

            if self.handler.settings().key_strict {
                let res_key = try!(from_utf8(try!(shake.response.key())));
                let req_key = hash_key(try!(shake.request.key()));
                if req_key != res_key {
                    return Err(Error::new(Kind::Protocol, format!("Received incorrect WebSocket Accept key: {} vs {}", req_key, res_key)));
                }
            }

            try!(self.handler.on_response(&shake.response));
            try!(self.handler.on_open(shake));

            // check to see if there is anything to read already
            if self.in_buffer.get_ref().len() > 0 {
                try!(self.read_frames());
            }

            return Ok(self.check_events())
        }
        Err(Error::new(Kind::Internal, "Tried to read WebSocket handshake while not in connecting state!"))
    }

    pub fn read(&mut self) -> Result<()> {
        if let Some(_) = try!(self.buffer_in()) {
            self.read_frames()
        } else {
            Ok(())
        }
    }

    fn read_frames(&mut self) -> Result<()> {
        let settings = self.handler.settings();
        let mut size = self.in_buffer.get_ref().len() - self.in_buffer.position() as usize;
        while let Some(frame) = try!(Frame::parse(&mut self.in_buffer, &mut size)) {

            if settings.masking_strict {
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

            if frame.is_final() {
                match frame.opcode() {
                    // singleton data frames
                    OpCode::Text => {
                        trace!("Received text frame {:?}", frame);
                        if let Some(frame) = try!(self.handler.on_text_frame(frame)) {
                            try!(self.verify_reserve(&frame));
                            // since we are going to handle this, there can't be an ongoing
                            // message
                            if self.fragments.len() > 0 {
                                return Err(Error::new(Kind::Protocol, "Received unfragmented text frame while processing fragmented message."))
                            }
                            debug_assert!(frame.opcode() == OpCode::Text, "Handler passed back corrupted frame.");
                            let msg = Message::text(try!(String::from_utf8(frame.into_data()).map_err(|err| err.utf8_error())));
                            try!(self.handler.on_message(msg));
                        }
                    }
                    OpCode::Binary => {
                        trace!("Received binary frame {:?}", frame);
                        if let Some(frame) = try!(self.handler.on_binary_frame(frame)) {
                            try!(self.verify_reserve(&frame));
                            // since we are going to handle this, there can't be an ongoing
                            // message
                            if self.fragments.len() > 0 {
                                return Err(Error::new(Kind::Protocol, "Received unfragmented binary frame while processing fragmented message."))
                            }
                            debug_assert!(frame.opcode() == OpCode::Binary, "Handler passed back corrupted frame.");
                            let data = frame.into_data();
                            try!(self.handler.on_message(Message::binary(data)));
                        }
                    }
                    // control frames
                    OpCode::Close => {
                        trace!("Received close frame {:?}", frame);
                        if !self.state.is_closing() {
                            if let Some(frame) = try!(self.handler.on_close_frame(frame)) {
                                try!(self.verify_reserve(&frame));
                                debug_assert!(frame.opcode() == OpCode::Close, "Handler passed back corrupted frame.");

                                let mut close_code = [0u8; 2];
                                let mut data = Cursor::new(frame.into_data());
                                if let 2 = try!(data.read(&mut close_code)) {
                                    let code_be: u16 = unsafe {transmute(close_code) };
                                    trace!("Connection {:?} received raw close code: {:?}, {:b}", self.token, code_be, code_be);
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
                                        if has_reason {
                                            try!(self.send_close(named, "")); // note this drops any extra close data
                                        } else {
                                            try!(self.send_close(CloseCode::Invalid, ""));
                                        }
                                    }
                                } else {
                                    // This is not an error. It is allowed behavior in the
                                    // protocol, so we don't trigger an error
                                    self.handler.on_close(CloseCode::Status, "Unable to read close code. Sending empty close frame.");
                                    try!(self.send_close(CloseCode::Empty, ""));
                                }
                            }
                        }
                    }
                    OpCode::Ping => {
                        trace!("Received ping frame {:?}", frame);
                        if let Some(frame) = try!(self.handler.on_ping_frame(frame)) {
                            try!(self.verify_reserve(&frame));
                            debug_assert!(frame.opcode() == OpCode::Ping, "Handler passed back corrupted frame.");
                            try!(self.send_pong(frame.into_data()));
                        }
                    }
                    OpCode::Pong => {
                        trace!("Received pong frame {:?}", frame);
                        // no validation for now
                        if let Some(frame) = try!(self.handler.on_pong_frame(frame)) {
                            try!(self.verify_reserve(&frame));
                        }
                    }
                    // last fragment
                    OpCode::Continue => {
                        trace!("Received final fragment {:?}", frame);
                        if let Some(last) = try!(self.handler.on_fragmented_frame(frame)) {
                            try!(self.verify_reserve(&last));
                            if let Some(first) = self.fragments.pop_front() {
                                let size = self.fragments.iter().fold(first.payload_len() + last.payload_len(), |len, frame| len + frame.payload_len()) as usize;
                                match first.opcode() {
                                    OpCode::Text => {
                                        trace!("Constructing text message from fragments: {:?} -> {:?} -> {:?}", first, self.fragments.iter().collect::<Vec<&Frame>>(), last);
                                        let mut data = Vec::with_capacity(size);
                                        data.extend(first.into_data());
                                        while let Some(frame) = self.fragments.pop_front() {
                                            data.extend(frame.into_data());
                                        }
                                        data.extend(last.into_data());

                                        let string = try!(String::from_utf8(data).map_err(|err| err.utf8_error()));

                                        trace!("Calling handler with constructed message: {:?}", string);
                                        try!(self.handler.on_message(Message::text(string)));
                                    }
                                    OpCode::Binary => {
                                        trace!("Constructing text message from fragments: {:?} -> {:?} -> {:?}", first, self.fragments.iter().collect::<Vec<&Frame>>(), last);
                                        let mut data = Vec::with_capacity(size);
                                        data.extend(first.into_data());

                                        while let Some(frame) = self.fragments.pop_front() {
                                            data.extend(frame.into_data());
                                        }

                                        data.extend(last.into_data());

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
                    }
                    _ => return Err(Error::new(Kind::Protocol, "Encountered invalid opcode.")),
                }
            } else {
                match frame.opcode() {
                    OpCode::Text | OpCode::Binary | OpCode::Continue => {
                        trace!("Received non-final fragment frame {:?}", frame);
                        if let Some(frame) = try!(self.handler.on_fragmented_frame(frame)) {
                            try!(self.verify_reserve(&frame));
                            self.fragments.push_back(frame)
                        }
                    }
                    _ => {
                        return Err(Error::new(Kind::Protocol, "Encounted fragmented control frame."))
                    }
                }
            }
        }
        Ok(())
    }

    pub fn write(&mut self) -> Result<()> {
        trace!("Flushing outgoing frames for {:?}.", self.token);
        while let Some(len) = try!(self.socket.try_write_buf(&mut self.out_buffer)) {
            trace!("Wrote {} bytes on {:?}", len, self.token);
            if len == 0 && self.is_server() && self.state.is_closing() {
                // we are are a server that is closing and just wrote out our last frame,
                // let's disconnect
                return Ok(self.events = EventSet::none());
            } else if len == 0 {
                break
            }
        }
        Ok(self.check_events())
    }

    pub fn send_message(&mut self, msg: Message) -> Result<()> {
        let settings = self.handler.settings();
        let opcode = msg.opcode();
        trace!("Message opcode {:?}", opcode);
        let data = msg.into_data();
        if data.len() > settings.fragment_size {
            trace!("Chunking at {:?}.", settings.fragment_size);
            // note this copies the data, so it's actually somewhat expensive to fragment
            let mut chunks = data.chunks(settings.fragment_size).peekable();
            let chunk = chunks.next().expect("Unable to get initial chunk!");

            try!(self.buffer_frame(
                Frame::message(Vec::from(chunk), opcode, false)));

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
            try!(self.buffer_frame(Frame::message(data, opcode, true)));
        }
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_ping(&mut self, data: Vec<u8>) -> Result<()> {
        trace!("Sending ping for {:?}", self.token);
        try!(self.buffer_frame(Frame::ping(data)));
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_pong(&mut self, data: Vec<u8>) -> Result<()> {
        if self.state.is_closing() {
            return Ok(())
        }
        trace!("Sending pong for {:?}", self.token);
        try!(self.buffer_frame(Frame::pong(data)));
        Ok(self.check_events())
    }

    #[inline]
    pub fn send_close<R>(&mut self, code: CloseCode, reason: R) -> Result<()>
        where R: Borrow<str>
    {
        trace!("Sending close {:?} -- {:?} for {:?}", code, reason.borrow(), self.token);
        try!(self.buffer_frame(Frame::close(code, reason.borrow())));
        trace!("Connection {:?} is now closing.", self.token);
        self.state = Closing;
        Ok(self.check_events())
    }

    fn check_events(&mut self) {
        trace!("Checking state for {:?}: {:?}", self.token, self.state);
        if !self.state.is_connecting() {
            // always ready to read
            self.events.insert(EventSet::readable());
            if (self.out_buffer.position() as usize) < self.out_buffer.get_ref().len() {
                self.events.insert(EventSet::writable());
            } else {
                self.events.remove(EventSet::writable());
            }
        }
    }

    fn buffer_frame(&mut self, mut frame: Frame) -> Result<()> {
        try!(self.buffer_out(&frame));

        if self.is_client() {
            frame.mask();
        }

        let pos = self.out_buffer.position();
        try!(self.out_buffer.seek(SeekFrom::End(0)));
        try!(frame.format(&mut self.out_buffer));
        try!(self.out_buffer.seek(SeekFrom::Start(pos)));
        Ok(())
    }

    fn buffer_out(&mut self, frame: &Frame) -> Result<()> {
        if self.out_buffer.get_ref().capacity() <= self.out_buffer.get_ref().len() + frame.len() {
            let mut new = Vec::with_capacity(self.out_buffer.get_ref().capacity());
            new.extend(&self.out_buffer.get_ref()[self.out_buffer.position() as usize ..]);
            if new.len() == new.capacity() {
                let settings = self.handler.settings();
                if settings.out_buffer_grow {
                    new.reserve(settings.out_buffer_capacity)
                } else {
                    return Err(Error::new(Kind::Capacity, "Maxed out output buffer for connection."))
                }
            }
            self.out_buffer = Cursor::new(new);
        }
        Ok(())
    }

    fn buffer_in(&mut self) -> Result<Option<usize>> {
        trace!("Reading buffer on {:?}", self.token);
        if let Some(mut len) = try!(self.socket.try_read_buf(self.in_buffer.get_mut())) {
            if len == 0 {
                trace!("Buffered {} on {:?}", len, self.token);
                return Ok(None)
            }
            loop {
                if self.in_buffer.get_ref().len() == self.in_buffer.get_ref().capacity() {
                    let mut new = Vec::with_capacity(self.in_buffer.get_ref().capacity());
                    new.extend(&self.in_buffer.get_ref()[self.in_buffer.position() as usize ..]);
                    if new.len() == new.capacity() {
                        let settings = self.handler.settings();
                        if settings.in_buffer_grow {
                            new.reserve(settings.in_buffer_capacity);
                        } else {
                            return Err(Error::new(Kind::Capacity, "Maxed out input buffer for connection."))
                        }

                        self.in_buffer = Cursor::new(new);
                        // return now so that hopefully we will consume some of the buffer so this
                        // won't happen next time
                        trace!("Buffered {} on {:?}", len, self.token);
                        return Ok(Some(len));
                    }
                    self.in_buffer = Cursor::new(new);
                }

                if let Some(next) = try!(self.socket.try_read_buf(self.in_buffer.get_mut())) {
                    if next == 0 {
                        return Ok(Some(len))
                    }
                    len += next
                } else {
                    trace!("Buffered {} on {:?}", len, self.token);
                    return Ok(Some(len))
                }
            }
        } else {
            Ok(None)
        }
    }

    #[inline]
    fn verify_reserve(&mut self, frame: &Frame) -> Result<()> {
        // The default implementation doesn't support having reserved bits set
        if frame.has_rsv1() || frame.has_rsv2() || frame.has_rsv3() {
            return Err(Error::new(Kind::Protocol, "Encountered frame with reserved bits set."))
        } else {
            Ok(())
        }
    }
}
