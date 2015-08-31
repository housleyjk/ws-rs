use std::net::{SocketAddr, ToSocketAddrs};

use mio;
use mio::{
    Token,
    EventLoop,
    EventSet,
    PollOpt,
};
use mio::tcp::{TcpListener, TcpStream, TcpSocket};
use mio::util::Slab;
use url::Url;

use communication::{Sender, Signal, Command};
use result::{Result, Error, Kind};
use connection::Connection;
use connection::factory::Factory;
use connection::state::Endpoint;
use handshake::{Handshake, Request};
use protocol::CloseCode;

pub const ALL: Token = Token(0);
const CONN_START: Token = Token(1);

pub type Loop<F> = EventLoop<Handler<F>>;
type Conn<F> = Connection<TcpStream, <F as Factory>::Handler>;
type Chan = mio::Sender<Command>;

fn connect_to_url(url: &Url) -> Result<TcpStream> {
    let mut result: Result<TcpStream> = Err(Error::new(Kind::Internal, "Can't connect to url"));
    let host = try!(url.serialize_host().ok_or(Error::new(Kind::Internal, "No host passed for WebSocket connection.")));
    let port = url.port_or_default().unwrap_or(80);
    for addr in try!((&host[..], port).to_socket_addrs()) {
        result = TcpStream::connect(&addr).map_err(Error::from);
        if result.is_ok() {
            break
        }
    }
    result
}

enum State {
    Active,
    Inactive,
}

impl State {

    fn is_active(&self) -> bool {
        match *self {
            State::Active => true,
            State::Inactive => false,
        }
    }
}


pub struct Handler<F>
    where F: Factory
{
    listener: Option<TcpListener>,
    connections: Slab<Conn<F>>,
    factory: F,
    state: State,
}

impl<F> Handler<F>
    where F: Factory
{
    pub fn new(mut factory: F) -> Handler<F> {
        Handler {
            listener: None,
            connections: Slab::new_starting_at(CONN_START, factory.settings().max_connections),
            factory: factory,
            state: State::Active,
        }
    }

    pub fn listen(&mut self, eloop: &mut Loop<F>, addr: &SocketAddr) -> Result<&mut Handler<F>> {

        debug_assert!(
            self.listener.is_none(),
            "Attempted to listen for connections from two addresses on the same websocket.");

        // let tcp = try!(TcpListener::bind(addr));
        let socket = try!(TcpSocket::v4());
        try!(socket.set_reuseaddr(true));
        try!(socket.bind(addr));
        let tcp = try!(socket.listen(1024));
        try!(eloop.register(&tcp, ALL));
        self.listener = Some(tcp);
        Ok(self)
    }

    pub fn connect(&mut self, eloop: &mut Loop<F>, url: &Url) -> Result<&mut Handler<F>> {
        {
            if url.serialize_host().is_none() || url.scheme != "ws" { // only support non-tls for now
                return Err(Error::new(Kind::Internal, format!("Not a valid websocket url: {:?}", url)))
            }

            let sock = try!(connect_to_url(url));
            let factory = &mut self.factory;
            // TODO: use factory settings for things like extensions
            let req = try!(Request::from_url(url));

            let tok = try!(self.connections.insert_with(|tok| {
                let handler = factory.connection_made(Sender::new(tok, eloop.channel()));
                Connection::builder(tok, sock, handler).client().request(req).build()
            }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")));

            let conn = &mut self.connections[tok];
            try!(eloop.register_opt(
                conn.socket(),
                conn.token(),
                conn.events(),
                PollOpt::edge() | PollOpt::oneshot(),
            ));
        }
        Ok(self)
    }

    pub fn accept(&mut self, eloop: &mut Loop<F>, sock: TcpStream) -> Result<()> {
        let factory = &mut self.factory;
        let settings = factory.settings();
        let tok = try!(self.connections.insert_with(|tok| {
            let handler = factory.connection_made(Sender::new(tok, eloop.channel()));
            Connection::builder(tok, sock, handler).build()
        }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")));

        let conn = &mut self.connections[tok];

        eloop.register_opt(
            conn.socket(),
            conn.token(),
            conn.events(),
            PollOpt::edge() | PollOpt::oneshot(),
        ).map_err(Error::from).or_else(|err| {
            error!("Encountered error while trying to build WebSocket connection: {}", err);
            conn.error(err);
            if settings.panic_on_new_connection {
                panic!("Encountered error while trying to build WebSocket connection.");
            }
            Ok(())
        })
    }

    #[inline]
    fn schedule(&self, eloop: &mut Loop<F>, conn: &Conn<F>) -> Result<()> {
        trace!("Scheduling connection {:?} as {:?}", conn.token(), conn.events());
        Ok(try!(eloop.reregister(
            conn.socket(),
            conn.token(),
            conn.events(),
            PollOpt::edge() | PollOpt::oneshot()
        )))
    }

}


impl<F> mio::Handler for Handler <F>
    where F: Factory
{
    type Timeout = ();
    type Message = Command;

    fn ready(&mut self, eloop: &mut Loop<F>, token: Token, events: EventSet) {
        let settings = self.factory.settings();

        match token {
            ALL => {
                if events.is_readable() {
                    if let Some(sock) = {
                            match self.listener.as_ref().expect("No listener provided for server websocket connections").accept() {
                                Ok(inner) => inner,
                                Err(err) => {
                                    error!("Encountered an error {:?} while accepting tcp connection.", err);
                                    None
                                }
                            }
                        }
                    {
                        info!("Accepted a new tcp connection {:?}.", sock);
                        if let Err(err) = self.accept(eloop, sock) {
                            error!("Unable to build WebSocket connection {:?}", err);
                            if settings.panic_on_new_connection {
                                panic!("Unable to build WebSocket connection {:?}", err);
                            }
                        }

                    } else {
                        trace!("Blocked while accepting new tcp connection.")
                    }
                }
            }
            _ => {
                if events.is_error() {
                    trace!("Encountered error on tcp stream.");
                    if let Err(err) = self.connections[token].socket().take_socket_error() {
                        trace!("Error was {}", err);
                        self.connections[token].error(Error::from(err));
                    }
                    trace!("Dropping connection {:?}", token);
                    self.connections.remove(token);
                } else if events.is_hup() {
                    debug!("Tcp connection hung up on {:?}.", token);
                    self.connections.remove(token);
                } else {

                    let active = {
                        let conn = &mut self.connections[token];
                        let conn_events = conn.events();
                        if conn.state().is_connecting() {
                            if (events & conn_events).is_readable() {
                                trace!("Ready to read handshake on {:?}.", token);
                                if let Err(err) = conn.read_handshake() {
                                    trace!(
                                        "Encountered error while trying to read handshake on {:?}: {}",
                                        conn.token(),
                                        err);
                                    conn.error(err)
                                }
                            }

                            if (events & conn_events).is_writable() {
                                trace!("Ready to write handshake on {:?}.", token);
                                if let Err(err) = conn.write_handshake() {
                                    trace!(
                                        "Encountered error while trying to write handshake on {:?}: {}",
                                        conn.token(),
                                        err);
                                    conn.error(err)
                                }
                            }
                        } else { // even if we are closing, the connection might need to finish up
                            if (events & conn_events).is_readable() {
                                trace!("Ready to read messages on {:?}.", token);
                                if let Err(err) = conn.read() {
                                    trace!(
                                        "Encountered error while trying to read frames on {:?}: {}",
                                        conn.token(),
                                        err);
                                    conn.error(err)
                                }
                            }

                            if (events & conn_events).is_writable() {
                                trace!("Ready to write messages on {:?}.", token);
                                if let Err(err) = conn.write() {
                                    trace!(
                                        "Encountered error while trying to write frames on {:?}: {}",
                                        conn.token(),
                                        err);
                                    conn.error(err)
                                }
                            }
                        }

                        // connection events may have changed
                        conn.events().is_readable() || conn.events().is_writable()
                    };

                    if !active {
                        // normal closure
                        debug_assert!(
                            self.connections[token].state().is_closing(),
                            "Connection neither readable nor writable in active state!"
                        );
                        self.connections.remove(token);
                        debug!("WebSocket connection {:?} disconnected.", token);
                    } else {
                        self.schedule(eloop, &self.connections[token]).or_else(|err| {
                            self.connections[token].error(Error::from(err));
                            self.connections.remove(token);
                            Ok::<(), Error>(())
                        }).unwrap()
                    }

                }

                trace!("Active connections {:?}", self.connections.count());
                if self.connections.count() == 0 && (!self.state.is_active() || self.listener.is_none()) {
                    debug!("Shutting websocket event loop.");
                    self.factory.on_shutdown();
                    eloop.shutdown();
                }
            }
        }
    }

    fn notify(&mut self, eloop: &mut Loop<F>, cmd: Command) {
        let settings = self.factory.settings();
        let token = cmd.token();
        match cmd.into_signal() {
            Signal::Message(msg) => {
                match token {
                    ALL => {
                        trace!("Broadcasting message: {:?}", msg);
                        let mut dead = Vec::with_capacity(self.connections.count());
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_message(msg.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }

                        for conn in self.connections.iter() {
                            if let Err(err) = self.schedule(eloop, conn) {
                                dead.push((conn.token(), err))
                            }
                        }
                        for (token, err) in dead {
                            // note the same connection may be called twice
                            self.connections[token].error(err)
                        }
                        return
                    }
                    _ => {
                        if let Some(conn) = self.connections.get_mut(token) {
                            if let Err(err) = conn.send_message(msg) {
                                conn.error(err)
                            }
                        } else {
                            trace!("Connection disconnected while a message was waiting in the queue.")
                        }
                    }
                }
            }
            Signal::Close(code) => {
                if let Some(conn) = self.connections.get_mut(token) {
                    if let Err(err) = conn.send_close(code) {
                        conn.error(err)
                    }
                } else {
                    trace!("Connection disconnected while close signal was waiting in the queue.")
                }
            }
            Signal::Ping(data) => {
                    if let Some(conn) = self.connections.get_mut(token) {
                        if let Err(err) = conn.send_ping(data) {
                            conn.error(err)
                        }
                    } else {
                        trace!("Connection disconnected while ping signal was waiting in the queue.")
                    }
            }
            Signal::Pong(data) => {
                if let Some(conn) = self.connections.get_mut(token) {
                    if let Err(err) = conn.send_pong(data) {
                        conn.error(err)
                    }
                } else {
                    trace!("Connection disconnected while pong signal was waiting in the queue.")
                }
            }
            Signal::Connect(ref url) => {
                match token {
                    ALL =>  {
                        if let Err(err) = self.connect(eloop, url) {
                            if settings.panic_on_new_connection {
                                panic!("Unable to establish connection to {}: {:?}", url, err);
                            }
                        }
                    }
                    _ => {
                        if let Err(err) = self.connect(eloop, url) {
                            if let Some(conn) = self.connections.get_mut(token) {
                                conn.error(err)
                            }
                        }
                    }
                }
                return;
            }
            Signal::Shutdown => {
                debug!("Received shutdown signal. WebSocket is attempting to shut down.");
                for conn in self.connections.iter_mut() {
                    conn.shutdown();
                }
                self.factory.on_shutdown();
                self.state = State::Inactive;
                if settings.panic_on_shutdown {
                    panic!("Panicking on shutdown as per setting.")
                }
            }
        }

        if let Some(_) = self.connections.get(token) {
            if let Err(err) = self.schedule(eloop, &self.connections[token]) {
                self.connections[token].error(err)
            }
        }
    }

    fn timeout(&mut self, _: &mut Loop<F>, tout: ()) {
        debug!("Got timeout {:?}", tout);
    }

    fn interrupted(&mut self, _: &mut Loop<F>) {
        error!("Websocket shutting down for interrupt.");
    }
}
