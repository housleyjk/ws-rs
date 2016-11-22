use std::net::{SocketAddr, ToSocketAddrs};
use std::borrow::Borrow;
use std::time::Duration;

use mio;
use mio::{
    Token,
    EventLoop,
    EventSet,
    PollOpt,
};
use mio::tcp::{TcpListener, TcpStream};

use url::Url;

#[cfg(feature="ssl")]
use openssl::ssl::error::SslError;

use communication::{Sender, Signal, Command};
use result::{Result, Error, Kind};
use connection::Connection;
use factory::Factory;
use util::Slab;
use super::Settings;

pub const ALL: Token = Token(0);
pub const SYSTEM: Token = Token(1);
const CONN_START: Token = Token(2);

pub type Loop<F> = EventLoop<Handler<F>>;
type Conn<F> = Connection<<F as Factory>::Handler>;
type Chan = mio::Sender<Command>;

fn url_to_addrs(url: &Url) -> Result<Vec<SocketAddr>> {

    let host = url.host_str();
    if host.is_none() || ( url.scheme() != "ws" && url.scheme() != "wss" ) {
        return Err(Error::new(Kind::Internal, format!("Not a valid websocket url: {}", url)))
    }
    let host = host.unwrap();

    let port = url.port_or_known_default().unwrap_or(80);
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

#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    connection: Token,
    event: Token,
}

pub struct Handler<F>
    where F: Factory
{
    listener: Option<TcpListener>,
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
        try!(eloop.register(&tcp, ALL, EventSet::readable(), PollOpt::level()));
        self.listener = Some(tcp);
        Ok(self)
    }

    #[cfg(feature="ssl")]
    pub fn connect(&mut self, eloop: &mut Loop<F>, url: &Url) -> Result<()> {
        let settings = self.settings;
        let mut addresses = try!(url_to_addrs(url));
        let tok = {
            // note popping from the vector will most likely give us a tcpip v4 address
            let addr = try!(addresses.pop().ok_or(
                Error::new(
                    Kind::Internal,
                    format!("Unable to obtain any socket address for {}", url))));
            addresses.push(addr); // Replace the first addr in case ssl fails and we fallback

            let sock = try!(TcpStream::connect(&addr));
            let factory = &mut self.factory;

            try!(self.connections.insert_with(|tok| {
                let handler = factory.client_connected(Sender::new(tok, eloop.channel()));
                Connection::new(tok, sock, handler, settings)
            }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")))
        };

        if let Err(error) = self.connections[tok].as_client(url, addresses) {
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            return Err(error)
        }

        if url.scheme() == "wss" {
            while let Err(ssl_error) = self.connections[tok].encrypt() {
                match ssl_error.kind {
                    Kind::Ssl(SslError::StreamError(ref io_error)) => {
                        if let Some(errno) = io_error.raw_os_error() {
                            if errno == 111 {
                                if let Err(reset_error) = self.connections[tok].reset() {
                                    trace!("Encountered error while trying to reset connection: {:?}", reset_error);
                                } else {
                                    continue
                                }
                            }
                        }
                    }
                    _ => (),
                }
                self.connections[tok].error(ssl_error);
                // Allow socket to be registered anyway to await hangup
                break
            }
        }

        eloop.register(
            self.connections[tok].socket(),
            self.connections[tok].token(),
            self.connections[tok].events(),
            PollOpt::edge() | PollOpt::oneshot(),
        ).map_err(Error::from).or_else(|err| {
            error!("Encountered error while trying to build WebSocket connection: {}", err);
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            Err(err)
        })
    }

    #[cfg(not(feature="ssl"))]
    pub fn connect(&mut self, eloop: &mut Loop<F>, url: &Url) -> Result<()> {
        let settings = self.settings;
        let mut addresses = try!(url_to_addrs(url));
        let tok = {
            // note popping from the vector will most likely give us a tcpip v4 address
            let addr = try!(addresses.pop().ok_or(
                Error::new(
                    Kind::Internal,
                    format!("Unable to obtain any socket address for {}", url))));

            let sock = try!(TcpStream::connect(&addr));
            let factory = &mut self.factory;

            try!(self.connections.insert_with(|tok| {
                let handler = factory.client_connected(Sender::new(tok, eloop.channel()));
                Connection::new(tok, sock, handler, settings)
            }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")))
        };

        if let Err(error) = self.connections[tok].as_client(url, addresses) {
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            return Err(error)
        }

        if url.scheme() == "wss" {
            let error = Error::new(Kind::Protocol, "The ssl feature is not enabled. Please enable it to use wss urls.");
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            return Err(error)
        }

        eloop.register(
            self.connections[tok].socket(),
            self.connections[tok].token(),
            self.connections[tok].events(),
            PollOpt::edge() | PollOpt::oneshot(),
        ).map_err(Error::from).or_else(|err| {
            error!("Encountered error while trying to build WebSocket connection: {}", err);
            let handler = self.connections.remove(tok).unwrap().consume();
            self.factory.connection_lost(handler);
            Err(err)
        })
    }

    #[cfg(feature="ssl")]
    pub fn accept(&mut self, eloop: &mut Loop<F>, sock: TcpStream, addr: SocketAddr) -> Result<()> {
        let factory = &mut self.factory;
        let settings = self.settings;

        let tok = try!(self.connections.insert_with(|tok| {
            let mut sender = Sender::new(tok, eloop.channel());
            sender.set_addr(addr);
            let handler = factory.server_connected(sender);
            Connection::new(tok, sock, handler, settings)
        }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")));

        let conn = &mut self.connections[tok];

        try!(conn.as_server());
        if settings.encrypt_server {
            try!(conn.encrypt())
        }

        eloop.register(
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

    #[cfg(not(feature="ssl"))]
    pub fn accept(&mut self, eloop: &mut Loop<F>, sock: TcpStream, addr: SocketAddr) -> Result<()> {
        let factory = &mut self.factory;
        let settings = self.settings;

        let tok = try!(self.connections.insert_with(|tok| {
            let mut sender = Sender::new(tok, eloop.channel());
            sender.set_addr(addr);
            let handler = factory.server_connected(sender);
            Connection::new(tok, sock, handler, settings)
        }).ok_or(Error::new(Kind::Capacity, "Unable to add another connection to the event loop.")));

        let conn = &mut self.connections[tok];

        try!(conn.as_server());
        if settings.encrypt_server {
            return Err(Error::new(Kind::Protocol, "The ssl feature is not enabled. Please enable it to use wss urls."))
        }

        eloop.register(
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
        trace!("Scheduling connection to {} as {:?}", conn.socket().peer_addr().map(|addr| addr.to_string()).unwrap_or("UNKNOWN".into()), conn.events());
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
    type Timeout = Timeout;
    type Message = Command;

    fn ready(&mut self, eloop: &mut Loop<F>, token: Token, events: EventSet) {

        match token {
            SYSTEM => {
                debug_assert!(false, "System token used for io event. This is a bug!");
                error!("System token used for io event. This is a bug!")
            }
            ALL => {
                if events.is_readable() {
                    if let Some((sock, addr)) = {
                            match self.listener.as_ref().expect("No listener provided for server websocket connections").accept() {
                                Ok(inner) => inner,
                                Err(err) => {
                                    error!("Encountered an error {:?} while accepting tcp connection.", err);
                                    None
                                }
                            }
                        }
                    {
                        info!("Accepted a new tcp connection from {}.", addr);
                        if let Err(err) = self.accept(eloop, sock, addr) {
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
                    trace!("Encountered error on tcp stream.");
                    if let Err(err) = self.connections[token].socket().take_socket_error() {
                        trace!("Error was {}", err);
                        if let Some(errno) = err.raw_os_error() {
                            if errno == 111 {
                                match self.connections[token].reset() {
                                    Ok(_) => {
                                        eloop.register(
                                            self.connections[token].socket(),
                                            self.connections[token].token(),
                                            self.connections[token].events(),
                                            PollOpt::edge() | PollOpt::oneshot(),
                                        ).or_else(|err| {
                                            self.connections[token].error(Error::from(err));
                                            let handler = self.connections.remove(token).unwrap().consume();
                                            self.factory.connection_lost(handler);
                                            Ok::<(), Error>(())
                                        }).unwrap();
                                        return
                                    },
                                    Err(err) => {
                                        trace!("Encountered error while trying to reset connection: {:?}", err);
                                    }
                                }
                            }
                        }
                        // This will trigger disconnect if the connection is open
                        self.connections[token].error(Error::from(err));
                    } else {
                        self.connections[token].disconnect();
                    }
                    trace!("Dropping connection token={:?}.", token);
                    let handler = self.connections.remove(token).unwrap().consume();
                    self.factory.connection_lost(handler);
                } else if events.is_hup() {
                    trace!("Connection token={:?} hung up.", token);
                    self.connections[token].disconnect();
                    let handler = self.connections.remove(token).unwrap().consume();
                    self.factory.connection_lost(handler);
                } else {

                    let active = {
                        let conn = &mut self.connections[token];
                        let conn_events = conn.events();

                        if (events & conn_events).is_readable() {
                            if let Err(err) = conn.read() {
                                conn.error(err)
                            }
                        }

                        if (events & conn_events).is_writable() {
                            if let Err(err) = conn.write() {
                                conn.error(err)
                            }
                        }

                        // connection events may have changed
                        conn.events().is_readable() || conn.events().is_writable()
                    };

                    // NOTE: Closing state only applies after a ws connection was successfully
                    // established. It's possible that we may go inactive while in a connecting
                    // state if the handshake fails.
                    if !active {
                        if let Ok(addr) = self.connections[token].socket().peer_addr() {
                            debug!("WebSocket connection to {} disconnected.", addr);
                        } else {
                            trace!("WebSocket connection to token={:?} disconnected.", token);
                        }
                        let handler = self.connections.remove(token).unwrap().consume();
                        self.factory.connection_lost(handler);
                    } else {
                        self.schedule(eloop, &self.connections[token]).or_else(|err| {
                            // This will be an io error, so disconnect will already be called
                            self.connections[token].error(Error::from(err));
                            let handler = self.connections.remove(token).unwrap().consume();
                            self.factory.connection_lost(handler);
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
            SYSTEM => {
                // Scaffolding for system events such as internal timeouts
            }
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
                    Signal::Timeout { delay, token: event } => {
                        match eloop.timeout(Timeout {
                                connection: ALL,
                                event: event,
                            }, Duration::from_millis(delay)).map_err(Error::from)
                        {
                            Ok(timeout) => {
                                for conn in self.connections.iter_mut() {
                                    if let Err(err) = conn.new_timeout(event, timeout.clone()) {
                                        conn.error(err)
                                    }
                                }
                            }
                            Err(err) => {
                                if self.settings.panic_on_timeout {
                                    panic!("Unable to schedule timeout: {:?}", err);
                                }
                                error!("Unable to schedule timeout: {:?}", err);
                            }
                        }
                        return
                    }
                    Signal::Cancel(timeout) => {
                        eloop.clear_timeout(&timeout);
                        return
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
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to establish connection to {}: {:?}", url, err);
                                }
                                error!("Unable to establish connection to {}: {:?}", url, err);
                            }
                        }
                        return
                    }
                    Signal::Shutdown => self.shutdown(eloop),
                    Signal::Timeout { delay, token: event } => {
                        match eloop.timeout(Timeout {
                                connection: token,
                                event: event,
                            }, Duration::from_millis(delay)).map_err(Error::from)
                        {
                            Ok(timeout) => {
                                if let Some(conn) = self.connections.get_mut(token) {
                                    if let Err(err) = conn.new_timeout(event, timeout) {
                                        conn.error(err)
                                    }
                                } else {
                                    trace!("Connection disconnected while pong signal was waiting in the queue.")
                                }
                            }
                            Err(err) => {
                                if let Some(conn) = self.connections.get_mut(token) {
                                    conn.error(err)
                                } else {
                                    trace!("Connection disconnected while pong signal was waiting in the queue.")
                                }
                            }
                        }
                        return
                    }
                    Signal::Cancel(timeout) => {
                        eloop.clear_timeout(&timeout);
                        return
                    }
                }

                if let Some(_) = self.connections.get(token) {
                    if let Err(err) = self.schedule(eloop, &self.connections[token]) {
                        self.connections[token].error(err)
                    }
                }
            }
        }
    }

    fn timeout(&mut self, _: &mut Loop<F>, Timeout { connection, event }: Timeout) {
        if let Some(conn) = self.connections.get_mut(connection) {
            if let Err(err) = conn.timeout_triggered(event) {
                conn.error(err)
            }
        } else {
            trace!("Connection disconnected while timeout was waiting.")
        }
    }

    fn interrupted(&mut self, eloop: &mut Loop<F>) {
        if self.settings.shutdown_on_interrupt {
            error!("Websocket shutting down for interrupt.");
            eloop.shutdown()
        } else {
            error!("Websocket received interupt.");
        }
    }
}

mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use std::str::FromStr;

    use url::Url;

    use result::{Error, Kind};
    use super::*;
    use super::url_to_addrs;

    #[test]
    fn test_url_to_addrs() {
        let ws_url = Url::from_str("ws://example.com?query=me").unwrap();
        let wss_url = Url::from_str("wss://example.com/suburl#fragment").unwrap();
        let bad_url = Url::from_str("http://howdy.bad.com").unwrap();
        let no_resolve = Url::from_str("ws://bad.elucitrans.com").unwrap();

        assert!(url_to_addrs(&ws_url).is_ok());
        assert!(url_to_addrs(&ws_url).unwrap().len() > 0);
        assert!(url_to_addrs(&wss_url).is_ok());
        assert!(url_to_addrs(&wss_url).unwrap().len() > 0);

        match url_to_addrs(&bad_url) {
            Ok(_) => panic!("url_to_addrs accepts http urls."),
            Err(Error { kind: Kind::Internal, details: _}) => (),  // pass
            err => panic!("{:?}", err),
        }

        match url_to_addrs(&no_resolve) {
            Ok(_) => panic!("url_to_addrs creates addresses for non-existent domains."),
            Err(Error { kind: Kind::Io(_), details: _}) => (),  // pass
            err => panic!("{:?}", err),
        }

    }

}
