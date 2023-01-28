use std::borrow::Borrow;
use std::fmt::{Display, Debug};
use std::io::{Error as IoError, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;
use std::usize;

use mio;
use mio::deprecated::UnixListener;
use mio::tcp::{TcpListener, TcpStream};
use mio::{Poll, PollOpt, Ready, Token};
use mio_extras;

use url::Url;

#[cfg(feature = "native_tls")]
use native_tls::Error as SslError;

use super::Settings;
use communication::{Command, Sender, Signal};
use connection::Connection;
use factory::Factory;
use slab::Slab;
use result::{Error, Kind, Result};
use stream::Stream;


const QUEUE: Token = Token(usize::MAX - 3);
const TIMER: Token = Token(usize::MAX - 4);
pub const ALL: Token = Token(usize::MAX - 5);
const SYSTEM: Token = Token(usize::MAX - 6);

type Conn<F> = Connection<<F as Factory>::Handler>;

const MAX_EVENTS: usize = 1024;
const MESSAGES_PER_TICK: usize = 256;
const TIMER_TICK_MILLIS: u64 = 100;
const TIMER_WHEEL_SIZE: usize = 1024;
const TIMER_CAPACITY: usize = 65_536;

#[cfg(not(windows))]
const CONNECTION_REFUSED: i32 = 111;
#[cfg(windows)]
const CONNECTION_REFUSED: i32 = 61;

fn url_to_addrs(url: &Url) -> Result<Vec<SocketAddr>> {
    let host = url.host_str();
    if host.is_none() || (url.scheme() != "ws" && url.scheme() != "wss") {
        return Err(Error::new(
            Kind::Internal,
            format!("Not a valid websocket url: {}", url),
        ));
    }
    let host = host.unwrap();

    let port = url.port_or_known_default().unwrap_or(80);
    let mut addrs = (&host[..], port)
        .to_socket_addrs()?
        .collect::<Vec<SocketAddr>>();
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

enum Listener {
    TcpListenner(TcpListener),
    UnixListener(mio::deprecated::UnixListener),
}

impl Listener {

    fn accept(&self) -> Result<(MyStream, Address)> {
        match self {
            Listener::TcpListenner(tcp) => {
                let native = tcp.accept()?;
                Ok((MyStream::TcpStream(native.0), Address::TcpAddress(native.1)))
            },
            Listener::UnixListener(unix) => {
                let native = unix.accept()?;
                Ok((MyStream::UnixStream(native), Address::UnixAddress(None)))
            }
        }
    }

    fn local_addr(&self) -> Result<Address> {
        match self {
            Listener::TcpListenner(tcp) => {
                let native = tcp.local_addr()?;
                Ok(Address::TcpAddress(native))
            },
            Listener::UnixListener(unix) => {
                Ok(Address::UnixAddress(None))
            }
        }
    }

}

enum MyStream {
    TcpStream(mio::net::TcpStream),
    UnixStream(mio::deprecated::unix::UnixStream),
}

pub enum Address {
    TcpAddress(std::net::SocketAddr),
    UnixAddress(Option<std::os::unix::net::SocketAddr>),
}

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::TcpAddress(tcp) => {
                std::fmt::Display::fmt(&tcp, f)
            }
            Address::UnixAddress(_) => {
                info!("Get unix domian socket address no implemented yet.");
                Ok(())
            },
        }
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::TcpAddress(tcp) => {
                std::fmt::Display::fmt(tcp, f)
            },
            Address::UnixAddress(_) => {
                info!("Get unix domian socket address no implemented yet.");
                Ok(())
            },
        }
    }
}
pub struct Handler<F>
where
    F: Factory,
{
    listener: Option<Listener>,
    connections: Slab<Conn<F>>,
    factory: F,
    settings: Settings,
    state: State,
    queue_tx: mio::channel::SyncSender<Command>,
    queue_rx: mio::channel::Receiver<Command>,
    timer: mio_extras::timer::Timer<Timeout>,
    next_connection_id: u32,
}

impl<F> Handler<F>
where
    F: Factory,
{
    pub fn new(factory: F, settings: Settings) -> Handler<F> {
        let (tx, rx) = mio::channel::sync_channel(settings.max_connections * settings.queue_size);
        let timer = mio_extras::timer::Builder::default()
            .tick_duration(Duration::from_millis(TIMER_TICK_MILLIS))
            .num_slots(TIMER_WHEEL_SIZE)
            .capacity(TIMER_CAPACITY)
            .build();
        Handler {
            listener: None,
            connections: Slab::with_capacity(settings.max_connections),
            factory,
            settings,
            state: State::Inactive,
            queue_tx: tx,
            queue_rx: rx,
            timer,
            next_connection_id: 0,
        }
    }

    pub fn sender(&self) -> Sender {
        Sender::new(ALL, self.queue_tx.clone(), 0)
    }

    pub fn listen(&mut self, poll: &mut Poll, addr: &SocketAddr) -> Result<&mut Handler<F>> {
        debug_assert!(
            self.listener.is_none(),
            "Attempted to listen for connections from two addresses on the same websocket."
        );

        let tcp = Listener::TcpListenner(TcpListener::bind(addr)?);
        let Listener::TcpListenner(tcp) = tcp else {panic!()};
        // TODO: consider net2 in order to set reuse_addr
        poll.register(&tcp, ALL, Ready::readable(), PollOpt::level())?;
        self.listener = Some(Listener::TcpListenner(tcp));
        Ok(self)
    }

    pub fn listen_unix<P: AsRef<Path> + ?Sized>(&mut self, poll: &mut Poll, addr: &P) -> Result<&mut Handler<F>> {
        debug_assert!(
            self.listener.is_none(),
            "Attempted to listen for connections from two addresses on the same websocket."
        );

        let unix = Listener::UnixListener(UnixListener::bind(addr)?);
        if let Listener::UnixListener(unix) = unix {
            // TODO: consider net2 in order to set reuse_addr
            poll.register(&unix, ALL, Ready::readable(), PollOpt::level())?;
            self.listener = Some(Listener::UnixListener(unix));
        }
        Ok(self)
    }

    pub fn local_addr(&self) -> ::std::io::Result<SocketAddr> {
        if let Some(ref listener) = self.listener {
            let addr = listener.local_addr().unwrap();
            let Address::TcpAddress(addr) = addr else {panic!()};
            Ok(addr)
        }
        else {
            Err(IoError::new(ErrorKind::NotFound, "Not a listening socket"))
        }
    }

    #[cfg(any(feature = "ssl", feature = "nativetls"))]
    pub fn connect(&mut self, poll: &mut Poll, url: Url) -> Result<()> {
        let settings = self.settings;

        let (tok, addresses) = {
            let (tok, entry, connection_id, handler) =
                if self.connections.len() < settings.max_connections {
                    let entry = self.connections.vacant_entry();
                    let tok = Token(entry.key());
                    let connection_id = self.next_connection_id;
                    self.next_connection_id = self.next_connection_id.wrapping_add(1);
                    (
                        tok,
                        entry,
                        connection_id,
                        self.factory.client_connected(Sender::new(
                            tok,
                            self.queue_tx.clone(),
                            connection_id,
                        )),
                    )
                } else {
                    return Err(Error::new(
                        Kind::Capacity,
                        "Unable to add another connection to the event loop.",
                    ));
                };

            let mut addresses = match url_to_addrs(&url) {
                Ok(addresses) => addresses,
                Err(err) => {
                    self.factory.connection_lost(handler);
                    return Err(err);
                }
            };

            loop {
                if let Some(addr) = addresses.pop() {
                    if let Ok(sock) = TcpStream::connect(&addr) {
                        if settings.tcp_nodelay {
                            sock.set_nodelay(true)?
                        }
                        addresses.push(addr); // Replace the first addr in case ssl fails and we fallback
                        entry.insert(Connection::new(tok, Stream::Tcp(sock), handler, settings, connection_id));
                        break;
                    }
                } else {
                    self.factory.connection_lost(handler);
                    return Err(Error::new(
                        Kind::Internal,
                        format!("Unable to obtain any socket address for {}", url),
                    ));
                }
            }

            (tok, addresses)
        };

        let will_encrypt = url.scheme() == "wss";

        if let Err(error) = self.connections[tok.into()].as_client(url, addresses) {
            let handler = self.connections.remove(tok.into()).consume();
            self.factory.connection_lost(handler);
            return Err(error);
        }

        if will_encrypt {
            while let Err(ssl_error) = self.connections[tok.into()].encrypt() {
                match ssl_error.kind {
                    #[cfg(feature = "ssl")]
                    Kind::Ssl(ref inner_ssl_error) => {
                        if let Some(io_error) = inner_ssl_error.io_error() {
                            if let Some(errno) = io_error.raw_os_error() {
                                if errno == CONNECTION_REFUSED {
                                    if let Err(reset_error) = self.connections[tok.into()].reset() {
                                        trace!(
                                            "Encountered error while trying to reset connection: {:?}",
                                            reset_error
                                        );
                                    } else {
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    #[cfg(feature = "nativetls")]
                    Kind::Ssl(_) => {
                        if let Err(reset_error) = self.connections[tok.into()].reset() {
                            trace!(
                                "Encountered error while trying to reset connection: {:?}",
                                reset_error
                            );
                        } else {
                            continue;
                        }
                    }
                    _ => (),
                }
                self.connections[tok.into()].error(ssl_error);
                // Allow socket to be registered anyway to await hangup
                break;
            }
        }

        let result = self.dispatch_by_type(self.connections[tok.into()].socket(), tok, poll);
        result.map_err(Error::from)
            .or_else(|err| {
                error!(
                    "Encountered error while trying to build WebSocket connection: {}",
                    err
                );
                let handler = self.connections.remove(tok.into()).consume();
                self.factory.connection_lost(handler);
                Err(err)
            })
    }

    #[cfg(not(any(feature = "ssl", feature = "nativetls")))]
    pub fn connect(&mut self, poll: &mut Poll, url: Url) -> Result<()> {
        let settings = self.settings;

        let (tok, addresses) = {
            let (tok, entry, connection_id, handler) =
                if self.connections.len() < settings.max_connections {
                    let entry = self.connections.vacant_entry();
                    let tok = Token(entry.key());
                    let connection_id = self.next_connection_id;
                    self.next_connection_id = self.next_connection_id.wrapping_add(1);
                    (
                        tok,
                        entry,
                        connection_id,
                        self.factory.client_connected(Sender::new(
                            tok,
                            self.queue_tx.clone(),
                            connection_id,
                        )),
                    )
                } else {
                    return Err(Error::new(
                        Kind::Capacity,
                        "Unable to add another connection to the event loop.",
                    ));
                };

            let mut addresses = match url_to_addrs(&url) {
                Ok(addresses) => addresses,
                Err(err) => {
                    self.factory.connection_lost(handler);
                    return Err(err);
                }
            };

            loop {
                if let Some(addr) = addresses.pop() {
                    if let Ok(sock) = TcpStream::connect(&addr) {
                        if settings.tcp_nodelay {
                            sock.set_nodelay(true)?
                        }
                        entry.insert(Connection::new(tok, Stream::Tcp(sock), handler, settings, connection_id));
                        break;
                    }
                } else {
                    self.factory.connection_lost(handler);
                    return Err(Error::new(
                        Kind::Internal,
                        format!("Unable to obtain any socket address for {}", url),
                    ));
                }
            }

            (tok, addresses)
        };

        if url.scheme() == "wss" {
            let error = Error::new(
                Kind::Protocol,
                "The ssl feature is not enabled. Please enable it to use wss urls.",
            );
            let handler = self.connections.remove(tok.into()).consume();
            self.factory.connection_lost(handler);
            return Err(error);
        }

        if let Err(error) = self.connections[tok.into()].as_client(url, addresses) {
            let handler = self.connections.remove(tok.into()).consume();
            self.factory.connection_lost(handler);
            return Err(error);
        }

        let result = self.dispatch_by_type(&self.connections[tok.into()].socket(), tok, poll);
        result.map_err(Error::from)
            .or_else(|err| {
                error!(
                    "Encountered error while trying to build WebSocket connection: {}",
                    err
                );
                let handler = self.connections.remove(tok.into()).consume();
                self.factory.connection_lost(handler);
                Err(err)
            })
    }

    #[cfg(any(feature = "ssl", feature = "nativetls"))]
    pub fn accept(&mut self, poll: &mut Poll, sock: Stream) -> Result<()> {
        let factory = &mut self.factory;
        let settings = self.settings;

        if settings.tcp_nodelay {
            if let Stream::Tcp(ref sock) = sock {
                sock.set_nodelay(true)?
            }
        }

        let tok = {
            if self.connections.len() < settings.max_connections {
                let entry = self.connections.vacant_entry();
                let tok = Token(entry.key());
                let connection_id = self.next_connection_id;
                self.next_connection_id = self.next_connection_id.wrapping_add(1);
                let handler = factory.server_connected(Sender::new(
                    tok,
                    self.queue_tx.clone(),
                    connection_id,
                ));
                entry.insert(Connection::new(tok, sock, handler, settings, connection_id));
                tok
            } else {
                return Err(Error::new(
                    Kind::Capacity,
                    "Unable to add another connection to the event loop.",
                ));
            }
        };

        let conn = &mut self.connections[tok.into()];

        conn.as_server()?;
        if settings.encrypt_server {
            conn.encrypt()?
        }

        let conn = &self.connections[tok.into()];
        let result = self.dispatch_by_type(conn.socket(), tok, poll);
        let conn = &mut self.connections[tok.into()];
        result.map_err(Error::from)
            .or_else(|err| {
                error!(
                    "Encountered error while trying to build WebSocket connection: {}",
                    err
                );
                conn.error(err);
                if settings.panic_on_new_connection {
                    panic!("Encountered error while trying to build WebSocket connection.");
                }
                Ok(())
            })
    }

    #[cfg(not(any(feature = "ssl", feature = "nativetls")))]
    pub fn accept(&mut self, poll: &mut Poll, sock: Stream) -> Result<()> {
        let factory = &mut self.factory;
        let settings = self.settings;

        if settings.tcp_nodelay {
            if let Stream::Tcp(ref sock) = sock {
                sock.set_nodelay(true)?
            }
        }

        let tok = {
            if self.connections.len() < settings.max_connections {
                let entry = self.connections.vacant_entry();
                let tok = Token(entry.key());
                let connection_id = self.next_connection_id;
                self.next_connection_id = self.next_connection_id.wrapping_add(1);
                let handler = factory.server_connected(Sender::new(
                    tok,
                    self.queue_tx.clone(),
                    connection_id,
                ));
                entry.insert(Connection::new(tok, sock, handler, settings, connection_id));
                tok
            } else {
                return Err(Error::new(
                    Kind::Capacity,
                    "Unable to add another connection to the event loop.",
                ));
            }
        };

        let conn = &mut self.connections[tok.into()];

        conn.as_server()?;
        if settings.encrypt_server {
            return Err(Error::new(
                Kind::Protocol,
                "The ssl feature is not enabled. Please enable it to use wss urls.",
            ));
        }
        
        let conn = &self.connections[tok.into()];
        let result = self.dispatch_by_type(conn.socket(), tok, poll);
        let conn = &mut self.connections[tok.into()];
        result.map_err(Error::from).or_else(|err| {
            error!(
                "Encountered error while trying to build WebSocket connection: {}",
                err
            );
            conn.error(err);
            if settings.panic_on_new_connection {
                panic!("Encountered error while trying to build WebSocket connection.");
            }
            Ok(())
        })
    }

    pub fn run(&mut self, poll: &mut Poll) -> Result<()> {
        trace!("Running event loop");
        poll.register(
            &self.queue_rx,
            QUEUE,
            Ready::readable(),
            PollOpt::edge() | PollOpt::oneshot(),
        )?;
        poll.register(&self.timer, TIMER, Ready::readable(), PollOpt::edge())?;

        self.state = State::Active;
        let result = self.event_loop(poll);
        self.state = State::Inactive;

        result
            .and(poll.deregister(&self.timer).map_err(Error::from))
            .and(poll.deregister(&self.queue_rx).map_err(Error::from))
    }

    #[inline]
    fn event_loop(&mut self, poll: &mut Poll) -> Result<()> {
        let mut events = mio::Events::with_capacity(MAX_EVENTS);
        while self.state.is_active() {
            trace!("Waiting for event");
            let nevents = match poll.poll(&mut events, None) {
                Ok(nevents) => nevents,
                Err(err) => {
                    if err.kind() == ErrorKind::Interrupted {
                        if self.settings.shutdown_on_interrupt {
                            error!("Websocket shutting down for interrupt.");
                            self.state = State::Inactive;
                        } else {
                            error!("Websocket received interrupt.");
                        }
                        0
                    } else {
                        return Err(Error::from(err));
                    }
                }
            };
            trace!("Processing {} events", nevents);

            for i in 0..nevents {
                let evt = events.get(i).unwrap();
                self.handle_event(poll, evt.token(), evt.kind());
            }

            self.check_count();
        }
        Ok(())
    }

    #[inline]
    fn schedule(&self, poll: &mut Poll, conn: &Conn<F>) -> Result<()> {
        trace!(
            "Scheduling connection to {} as {:?}",
            conn.socket()
                .peer_addr()
                .map(|addr| addr.to_string())
                .unwrap_or_else(|_| "UNKNOWN".into()),
            conn.events()
        );
        match conn.socket() {
            Stream::Tcp(tcp) => {
                poll.reregister(
                    tcp,
                    conn.token(),
                    conn.events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )?;
            },
            Stream::Unix(unix) => {
                poll.reregister(
                    unix,
                    conn.token(),
                    conn.events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )?;
            },
            #[cfg(any(feature = "ssl", feature = "nativetls"))]
            Stream::Tls(tls) => {
                poll.reregister(
                    tls.evented(),
                    conn.token(),
                    conn.events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )?;
            },
        };
        
        Ok(())
    }

    fn shutdown(&mut self) {
        debug!("Received shutdown signal. WebSocket is attempting to shut down.");
        for (_, conn) in self.connections.iter_mut() {
            conn.shutdown();
        }
        self.factory.on_shutdown();
        self.state = State::Inactive;
        if self.settings.panic_on_shutdown {
            panic!("Panicking on shutdown as per setting.")
        }
    }

    #[inline]
    fn check_active(&mut self, poll: &mut Poll, active: bool, token: Token) {
        // NOTE: Closing state only applies after a ws connection was successfully
        // established. It's possible that we may go inactive while in a connecting
        // state if the handshake fails.
        if !active {
            if let Ok(addr) = self.connections[token.into()].socket().peer_addr() {
                debug!("WebSocket connection to {} disconnected.", addr);
            } else {
                trace!("WebSocket connection to token={:?} disconnected.", token);
            }
            let handler = self.connections.remove(token.into()).consume();
            self.factory.connection_lost(handler);
        } else {
            self.schedule(poll, &self.connections[token.into()])
                .or_else(|err| {
                    // This will be an io error, so disconnect will already be called
                    self.connections[token.into()].error(err);
                    let handler = self.connections.remove(token.into()).consume();
                    self.factory.connection_lost(handler);
                    Ok::<(), Error>(())
                })
                .unwrap()
        }
    }

    #[inline]
    fn is_client(&self) -> bool {
        self.listener.is_none()
    }

    #[inline]
    fn check_count(&mut self) {
        trace!("Active connections {:?}", self.connections.len());
        if self.connections.is_empty() {
            if !self.state.is_active() {
                debug!("Shutting down websocket server.");
            } else if self.is_client() {
                debug!("Shutting down websocket client.");
                self.factory.on_shutdown();
                self.state = State::Inactive;
            }
        }
    }

    fn handle_event(&mut self, poll: &mut Poll, token: Token, events: Ready) {
        match token {
            SYSTEM => {
                debug_assert!(false, "System token used for io event. This is a bug!");
                error!("System token used for io event. This is a bug!");
            }
            ALL => {
                if events.is_readable() {
                    let accept_result = self.listener
                    .as_ref()
                    .expect("No listener provided for server websocket connections")
                    .accept();
                    match accept_result
                    {
                        Ok((MyStream::TcpStream(sock), Address::TcpAddress(addr))) => {
                            info!("Accepted a new tcp connection from {}.", addr);
                            if let Err(err) = self.accept(poll, Stream::Tcp(sock)) {
                                error!("Unable to build WebSocket connection {:?}", err);
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to build WebSocket connection {:?}", err);
                                }
                            }
                        },
                        Ok((MyStream::UnixStream(sock), Address::UnixAddress(_))) => {
                            info!("Accepted a new connection from unix domain socket.");
                            if let Err(err) = self.accept(poll, Stream::Unix(sock)) {
                                error!("Unable to build WebSocket connection {:?}", err);
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to build WebSocket connection {:?}", err);
                                }
                            }
                        },
                        Err(err) => error!(
                            "Encountered an error {:?} while accepting tcp connection.",
                            err
                        ),
                        _ => {
                            unreachable!();
                        }
                    }
                }
            }
            TIMER => while let Some(t) = self.timer.poll() {
                self.handle_timeout(poll, t);
            },
            QUEUE => {
                for _ in 0..MESSAGES_PER_TICK {
                    match self.queue_rx.try_recv() {
                        Ok(cmd) => self.handle_queue(poll, cmd),
                        _ => break,
                    }
                }
                let _ = poll.reregister(
                    &self.queue_rx,
                    QUEUE,
                    Ready::readable(),
                    PollOpt::edge() | PollOpt::oneshot(),
                );
            }
            _ => {
                let active = {
                    let conn_events = self.connections[token.into()].events();

                    if (events & conn_events).is_readable() {
                        if let Err(err) = self.connections[token.into()].read() {
                            trace!("Encountered error while reading: {}", err);
                            if let Kind::Io(ref err) = err.kind {
                                if let Some(errno) = err.raw_os_error() {
                                    if errno == CONNECTION_REFUSED {
                                        match self.connections[token.into()].reset() {
                                            Ok(_) => {
                                                self.dispatch_by_type(&self.connections[token.into()].socket(), token, poll)
                                                    .or_else(|err| {
                                                        self.connections[token.into()]
                                                            .error(Error::from(err));
                                                        let handler = self.connections
                                                            .remove(token.into())
                                                            .consume();
                                                        self.factory.connection_lost(handler);
                                                        Ok::<(), Error>(())
                                                    })
                                                    .unwrap();
                                                return;
                                            }
                                            Err(err) => {
                                                trace!("Encountered error while trying to reset connection: {:?}", err);
                                            }
                                        }
                                    }
                                }
                            }
                            // This will trigger disconnect if the connection is open
                            self.connections[token.into()].error(err)
                        }
                    }

                    let conn_events = self.connections[token.into()].events();

                    if (events & conn_events).is_writable() {
                        if let Err(err) = self.connections[token.into()].write() {
                            trace!("Encountered error while writing: {}", err);
                            if let Kind::Io(ref err) = err.kind {
                                if let Some(errno) = err.raw_os_error() {
                                    if errno == CONNECTION_REFUSED {
                                        match self.connections[token.into()].reset() {
                                            Ok(_) => {
                                                self.dispatch_by_type(&self.connections[token.into()].socket(), token, poll)
                                                    .or_else(|err| {
                                                        self.connections[token.into()]
                                                            .error(Error::from(err));
                                                        let handler = self.connections
                                                            .remove(token.into())
                                                            .consume();
                                                        self.factory.connection_lost(handler);
                                                        Ok::<(), Error>(())
                                                    })
                                                    .unwrap();
                                                return;
                                            }
                                            Err(err) => {
                                                trace!("Encountered error while trying to reset connection: {:?}", err);
                                            }
                                        }
                                    }
                                }
                            }
                            // This will trigger disconnect if the connection is open
                            self.connections[token.into()].error(err)
                        }
                    }

                    // connection events may have changed
                    self.connections[token.into()].events().is_readable()
                        || self.connections[token.into()].events().is_writable()
                };

                self.check_active(poll, active, token)
            }
        }
    }

    fn handle_queue(&mut self, poll: &mut Poll, cmd: Command) {
        match cmd.token() {
            SYSTEM => {
                // Scaffolding for system events such as internal timeouts
            }
            ALL => {
                let mut dead = Vec::with_capacity(self.connections.len());

                match cmd.into_signal() {
                    Signal::Message(msg) => {
                        trace!("Broadcasting message: {:?}", msg);
                        for (_, conn) in self.connections.iter_mut() {
                            if let Err(err) = conn.send_message(msg.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Close(code, reason) => {
                        trace!("Broadcasting close: {:?} - {}", code, reason);
                        for (_, conn) in self.connections.iter_mut() {
                            if let Err(err) = conn.send_close(code, reason.borrow()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Ping(data) => {
                        trace!("Broadcasting ping");
                        for (_, conn) in self.connections.iter_mut() {
                            if let Err(err) = conn.send_ping(data.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Pong(data) => {
                        trace!("Broadcasting pong");
                        for (_, conn) in self.connections.iter_mut() {
                            if let Err(err) = conn.send_pong(data.clone()) {
                                dead.push((conn.token(), err))
                            }
                        }
                    }
                    Signal::Connect(url) => {
                        if let Err(err) = self.connect(poll, url.clone()) {
                            if self.settings.panic_on_new_connection {
                                panic!("Unable to establish connection to {}: {:?}", url, err);
                            }
                            error!("Unable to establish connection to {}: {:?}", url, err);
                        }
                        return;
                    }
                    Signal::Shutdown => self.shutdown(),
                    Signal::Timeout {
                        delay,
                        token: event,
                    } => {
                        let timeout = self.timer.set_timeout(
                            Duration::from_millis(delay),
                            Timeout {
                                connection: ALL,
                                event,
                            },
                        );
                        for (_, conn) in self.connections.iter_mut() {
                            if let Err(err) = conn.new_timeout(event, timeout.clone()) {
                                conn.error(err);
                            }
                        }
                        return;
                    }
                    Signal::Cancel(timeout) => {
                        self.timer.cancel_timeout(&timeout);
                        return;
                    }
                }

                for (_, conn) in self.connections.iter() {
                    if let Err(err) = self.schedule(poll, conn) {
                        dead.push((conn.token(), err))
                    }
                }
                for (token, err) in dead {
                    // note the same connection may be called twice
                    self.connections[token.into()].error(err)
                }
            }
            token => {
                let connection_id = cmd.connection_id();
                match cmd.into_signal() {
                    Signal::Message(msg) => {
                        if let Some(conn) = self.connections.get_mut(token.into()) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_message(msg) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while a message was waiting in the queue.")
                            }
                        } else {
                            trace!(
                                "Connection disconnected while a message was waiting in the queue."
                            )
                        }
                    }
                    Signal::Close(code, reason) => {
                        if let Some(conn) = self.connections.get_mut(token.into()) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_close(code, reason) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while close signal was waiting in the queue.")
                            }
                        } else {
                            trace!("Connection disconnected while close signal was waiting in the queue.")
                        }
                    }
                    Signal::Ping(data) => {
                        if let Some(conn) = self.connections.get_mut(token.into()) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_ping(data) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while ping signal was waiting in the queue.")
                            }
                        } else {
                            trace!("Connection disconnected while ping signal was waiting in the queue.")
                        }
                    }
                    Signal::Pong(data) => {
                        if let Some(conn) = self.connections.get_mut(token.into()) {
                            if conn.connection_id() == connection_id {
                                if let Err(err) = conn.send_pong(data) {
                                    conn.error(err)
                                }
                            } else {
                                trace!("Connection disconnected while pong signal was waiting in the queue.")
                            }
                        } else {
                            trace!("Connection disconnected while pong signal was waiting in the queue.")
                        }
                    }
                    Signal::Connect(url) => {
                        if let Err(err) = self.connect(poll, url.clone()) {
                            if let Some(conn) = self.connections.get_mut(token.into()) {
                                conn.error(err)
                            } else {
                                if self.settings.panic_on_new_connection {
                                    panic!("Unable to establish connection to {}: {:?}", url, err);
                                }
                                error!("Unable to establish connection to {}: {:?}", url, err);
                            }
                        }
                        return;
                    }
                    Signal::Shutdown => self.shutdown(),
                    Signal::Timeout {
                        delay,
                        token: event,
                    } => {
                        let timeout = self.timer.set_timeout(
                            Duration::from_millis(delay),
                            Timeout {
                                connection: token,
                                event,
                            },
                        );
                        if let Some(conn) = self.connections.get_mut(token.into()) {
                            if let Err(err) = conn.new_timeout(event, timeout) {
                                conn.error(err)
                            }
                        } else {
                            trace!("Connection disconnected while pong signal was waiting in the queue.")
                        }
                        return;
                    }
                    Signal::Cancel(timeout) => {
                        self.timer.cancel_timeout(&timeout);
                        return;
                    }
                }

                if self.connections.get(token.into()).is_some() {
                    if let Err(err) = self.schedule(poll, &self.connections[token.into()]) {
                        self.connections[token.into()].error(err)
                    }
                }
            }
        }
    }

    fn handle_timeout(&mut self, poll: &mut Poll, Timeout { connection, event }: Timeout) {
        let active = {
            if let Some(conn) = self.connections.get_mut(connection.into()) {
                if let Err(err) = conn.timeout_triggered(event) {
                    conn.error(err)
                }

                conn.events().is_readable() || conn.events().is_writable()
            } else {
                trace!("Connection disconnected while timeout was waiting.");
                return;
            }
        };
        self.check_active(poll, active, connection);
    }

    fn dispatch_by_type(&self, stream: &Stream, tok: Token, poll: &Poll) -> std::io::Result<()> {
        let result;
        match stream {
            Stream::Tcp(ref tcp) => {
                result = poll.register(
                    tcp,
                    self.connections[tok.into()].token(),
                    self.connections[tok.into()].events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )
            },
            Stream::Unix(ref unix) => {
                result = poll.register(
                    unix,
                    self.connections[tok.into()].token(),
                    self.connections[tok.into()].events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )
            },
            #[cfg(any(feature = "ssl", feature = "nativetls"))]
            Stream::Tls(ref tls) => {
                result = poll.register(
                    tls.evented(),
                    self.connections[tok.into()].token(),
                    self.connections[tok.into()].events(),
                    PollOpt::edge() | PollOpt::oneshot(),
                )
            },
        }
        result
    }

}

mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use std::str::FromStr;

    use url::Url;

    use super::url_to_addrs;
    use super::*;
    use result::{Error, Kind};

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
            Err(Error {
                kind: Kind::Internal,
                details: _,
            }) => (), // pass
            err => panic!("{:?}", err),
        }

        match url_to_addrs(&no_resolve) {
            Ok(_) => panic!("url_to_addrs creates addresses for non-existent domains."),
            Err(Error {
                kind: Kind::Io(_),
                details: _,
            }) => (), // pass
            err => panic!("{:?}", err),
        }
    }

}
