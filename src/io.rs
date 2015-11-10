use std::net::{SocketAddr, ToSocketAddrs};
use std::borrow::Borrow;

use mio;
use mio::{
    Token,
    EventLoop,
    EventSet,
    PollOpt,
};
use mio::tcp::{TcpListener, TcpStream};
use mio::util::Slab;
use url::Url;

use communication::{Sender, Signal, Command};
use result::{Result, Error, Kind};
use connection::Connection;
use connection::factory::Factory;
use handshake::{Request, Handshake};
use super::Settings;

pub const ALL: Token = Token(0);
const CONN_START: Token = Token(1);

pub type Loop<F> = EventLoop<Handler<F>>;
type Conn<F> = Connection<TcpStream, <F as Factory>::Handler>;
type Chan = mio::Sender<Command>;

fn url_to_addrs(url: &Url) -> Result<Vec<SocketAddr>> {

    let host = url.serialize_host();
    if host.is_none() || url.scheme != "ws" { // only support non-tls for now
        return Err(Error::new(Kind::Internal, format!("Not a valid websocket url: {:?}", url)))
    }
    let host = host.unwrap();

    let port = url.port_or_default().unwrap_or(80);
    let mut addrs = try!((&host[..], port).to_socket_addrs()).collect::<Vec<SocketAddr>>();
    addrs.dedup();
    Ok(addrs)
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
    addresses: Vec<SocketAddr>,
    connections: Slab<Conn<F>>,
    factory: F,
    settings: Settings,
    state: State,
}

impl<F> Handler<F>
    where F: Factory
{
    pub fn new(factory: F, settings: Settings) -> Handler<F> {
        Handler {
            listener: None,
            addresses: Vec::new(),
            connections: Slab::new_starting_at(CONN_START, settings.max_connections),
            factory: factory,
            settings: settings,
            state: State::Active,
        }
    }

    pub fn listen(&mut self, eloop: &mut Loop<F>, addr: &SocketAddr) -> Result<&mut Handler<F>> {

        debug_assert!(
            self.listener.is_none(),
            "Attempted to listen for connections from two addresses on the same websocket.");

        let tcp = try!(TcpListener::bind(addr));
        // TODO: consider net2 in order to set reuse_addr
        try!(eloop.register_opt(&tcp, ALL, EventSet::readable(), PollOpt::level()));
        self.listener = Some(tcp);
        Ok(self)
    }

    pub fn connect(&mut self, eloop: &mut Loop<F>, url: &Url) -> Result<&mut Handler<F>> {
        {
            self.addresses = try!(url_to_addrs(url));
            // note popping from the vector will most likely give us a tcpip v4 address
            let addr = try!(self.addresses.pop().ok_or(
                Error::new(
                    Kind::Internal,
                    format!("Unable to obtain any socket address for {}", url))));
            let sock = try!(TcpStream::connect(&addr).map_err(Error::from));
            let factory = &mut self.factory;
            let settings = self.settings;

            let req = try!(Request::from_url(url, settings.protocols, settings.extensions));

            let mut shake = Handshake::default();
            shake.request = req;
            shake.peer_addr = Some(try!(sock.peer_addr()));
            shake.local_addr = Some(try!(sock.local_addr()));

            let tok = try!(self.connections.insert_with(|tok| {
                let handler = factory.connection_made(Sender::new(tok, eloop.channel()));
                Connection::builder(tok, sock, handler).client().handshake(shake).build(settings)
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
        let settings = self.settings;

        let mut shake = Handshake::default();
        shake.peer_addr = Some(try!(sock.peer_addr()));
        shake.local_addr = Some(try!(sock.local_addr()));


        let tok = try!(self.connections.insert_with(|tok| {
            let handler = factory.connection_made(Sender::new(tok, eloop.channel()));
            Connection::builder(tok, sock, handler).handshake(shake).build(settings)
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

    fn shutdown(&mut self, eloop: &mut Loop<F>) {
        debug!("Received shutdown signal. WebSocket is attempting to shut down.");
        for conn in self.connections.iter_mut() {
            conn.shutdown();
        }
        self.factory.on_shutdown();
        self.state = State::Inactive;
        // If the shutdown command is received after connections have disconnected,
        // we need to shutdown now because ready only fires on io events
        if self.connections.count() == 0 {
            eloop.shutdown()
        }
        if self.settings.panic_on_shutdown {
            panic!("Panicking on shutdown as per setting.")
        }
    }

}


impl<F> mio::Handler for Handler <F>
    where F: Factory
{
    type Timeout = ();
    type Message = Command;

    fn ready(&mut self, eloop: &mut Loop<F>, token: Token, events: EventSet) {

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
                        info!("Accepted a new tcp connection.");
                        if let Err(err) = self.accept(eloop, sock) {
                            error!("Unable to build WebSocket connection {:?}", err);
                            if self.settings.panic_on_new_connection {
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
                    debug!("Encountered error on tcp stream.");
                    if let Err(err) = self.connections[token].socket().take_socket_error() {
                        debug!("Error was {}", err);
                        if let Some(errno) = err.raw_os_error() {
                            if errno == 111 {
                                if let Some(addr) = self.addresses.pop() {
                                    if let Ok(sock) = TcpStream::connect(&addr) {
                                        if let Err(err) = self.connections[token].reset(sock) {
                                            trace!("Unable to reset client connection: {:?}", err);
                                        } else {
                                            eloop.register_opt(
                                                self.connections[token].socket(),
                                                self.connections[token].token(),
                                                self.connections[token].events(),
                                                PollOpt::edge() | PollOpt::oneshot(),
                                            ).or_else(|err| {
                                                self.connections[token].error(Error::from(err));
                                                self.connections.remove(token);
                                                Ok::<(), Error>(())
                                            }).unwrap();
                                            return
                                        }
                                    }
                                }
                            }
                        }
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
                if self.connections.count() == 0 {
                    if !self.state.is_active() {
                        debug!("Shutting down websocket server.");
                        eloop.shutdown();
                    } else if self.listener.is_none() {
                        debug!("Shutting down websocket client.");
                        self.factory.on_shutdown();
                        eloop.shutdown();
                    }
                }
            }
        }
    }

    fn notify(&mut self, eloop: &mut Loop<F>, cmd: Command) {
        match cmd.token() {
            ALL => {
                let mut dead = Vec::with_capacity(self.connections.count());

                match cmd.into_signal() {
                    Signal::Message(msg) => {
                        trace!("Broadcasting message: {:?}", msg);
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_message(msg.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Close(code, reason) => {
                        trace!("Broadcasting close: {:?} - {}", code, reason);
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_close(code, reason.borrow()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Ping(data) => {
                        trace!("Broadcasting ping");
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_ping(data.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Pong(data) => {
                        trace!("Broadcasting pong");
                        for conn in self.connections.iter_mut() {
                            if let Err(err) = conn.send_pong(data.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Connect(ref url) => {
                        if let Err(err) = self.connect(eloop, url) {
                            if self.settings.panic_on_new_connection {
                                panic!("Unable to establish connection to {}: {:?}", url, err);
                            }
                            error!("Unable to establish connection to {}: {:?}", url, err);
                        }
                        return
                    }
                    Signal::Shutdown => self.shutdown(eloop),
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
            }
            token => {
                match cmd.into_signal() {
                    Signal::Message(msg) => {
                        if let Some(conn) = self.connections.get_mut(token) {
                            if let Err(err) = conn.send_message(msg) {
                                conn.error(err)
                            }
                        } else {
                            trace!("Connection disconnected while a message was waiting in the queue.")
                        }
                    }
                    Signal::Close(code, reason) => {
                        if let Some(conn) = self.connections.get_mut(token) {
                            if let Err(err) = conn.send_close(code, reason) {
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
                        if let Err(err) = self.connect(eloop, url) {
                            if let Some(conn) = self.connections.get_mut(token) {
                                conn.error(err)
                            } else {
                                error!("Unable to establish connection to {}: {:?}", url, err);
                            }
                        }
                        return
                    }
                    Signal::Shutdown => self.shutdown(eloop),
                }

                if let Some(_) = self.connections.get(token) {
                    if let Err(err) = self.schedule(eloop, &self.connections[token]) {
                        self.connections[token].error(err)
                    }
                }
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
