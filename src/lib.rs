//! Usage
//! -----
//!
//! For simple applications, use one of the utility functions `listen` and `connect`:
//!
//! `listen` accepts a string that represents a socket address and a Factory, see
//! [Architecture](#architecture).
//!
//! ```no_run
//! // A WebSocket echo server
//!
//! use ws::listen;
//!
//! listen("127.0.0.1:3012", |out| {
//!     move |msg| {
//!        out.send(msg)
//!     }
//! }).unwrap()
//! ```
//!
//! `connect` accepts a string that represents a WebSocket URL (i.e. one that starts with ws://),
//! and it will attempt to connect to a WebSocket server at that location. It also accepts a
//! Factory.
//!
//! ```no_run
//! // A WebSocket client that sends one message then closes
//!
//! use ws::{connect, CloseCode};
//!
//! connect("ws://127.0.0.1:3012", |out| {
//!     out.send("Hello WebSocket").unwrap();
//!
//!     move |msg| {
//!         println!("Got message: {}", msg);
//!         out.close(CloseCode::Normal)
//!     }
//! }).unwrap()
//! ```
//!
//! Each of these functions encapsulates a mio EventLoop, creating and running a WebSocket in the
//! current thread. These are blocking functions, so they will only return after the encapsulated
//! WebSocket has been shutdown.
//!
//! Architecture
//! ------
//!
//! A WebSocket requires two basic components: a Factory and a Handler. A Factory is any struct
//! that implements the `Factory` trait. WS-RS already provides an implementation of `Factory` for
//! closures, so it is possible to pass a closure as a Factory to either of the utility functions.
//! Your Factory will be called each time the underlying TCP connection has been successfully
//! established, and it will need to return a Handler that will handle the new WebSocket connection.
//!
//! Factories can be used to manage state that applies to multiple WebSocket connections,
//! whereas Handlers manage the state of individual connections. Most of the time, a closure
//! Factory is sufficient, and you will only need to focus on writing your Handler.

//! Your Factory will be passed a Sender struct that represents the output of the WebSocket.
//! The Sender allows the Handler to send messages, initiate a WebSocket closing handshake
//! by sending a close code, and other useful actions. If you need to send messages from other parts
//! of your application it is possible to clone and send the Sender across threads allowing
//! other code to send messages on the WebSocket without blocking the event loop.
//!
//! Just as with the Factory, it is possible to use a closure as a simple Handler. The closure must
//! take a Message as it's only argument, and it may close over variables that exist in
//! the Factory. For example, in the above examples using `listen` and `connect`, the closure
//! Factory returns another closure as the Handler for the new connection. This closure closes over
//! the variable `out`, which is the Sender, representing the output of the WebSocket, so that it
//! can use that sender later to send a Message. Closure Handlers generally need to take ownership of the variables
//! that they close over because the Factory may be called multiple times. Think of Handlers as
//! though they are threads and they should make sense within Rust's memory model. Closure Handlers must return
//! a `Result<()>`, in order to handle errors without panicking.
//!
//! In the above examples, `out.close` and `out.send` both actually return a `Result<()>` indicating
//! whether they were able to schedule the requested command (either `close` or `send`) with the
//! EventLoop.
//!
//! *It is important that your Handler does not panic carelessly because a handler that panics will
//! disconnect every other connection that is using that WebSocket. Don't panic unless you want all
//! connections to immediately fail.*
//!
//! Guide
//! -----
//!
//! You may have noticed in the usage examples that the client example calls `unwrap` when sending the first
//! message, which will panic in the factory if the Message can't be sent for some reason. Also,
//! sending messages before a handler is returned means that the message will be queued before
//! the WebSocket handshake is complete. The handshake could fail for some reason, and then the
//! queued message would be wasted effort. Sending messages in the Factory is not bad for simple,
//! short-lived, or toy projects, but let's explore writing a handler that is better for
//! long-running applications.
//!
//! In order to solve the problem of sending a message immediately when a WebSocket connection is
//! established, you will need to write a Handler that implements the `on_open` method. For
//! example:
//!
//! ```no_run
//! use ws::{connect, Handler, Sender, Handshake, Result, Message, CloseCode};
//!
//! // Our Handler struct.
//! // Here we explicity indicate that the Client needs a Sender,
//! // whereas a closure captures the Sender for us automatically.
//! struct Client {
//!     out: Sender,
//! }
//!
//! // We implement the Handler trait for Client so that we can get more
//! // fine-grained control of the connection.
//! impl Handler for Client {
//!
//!     // `on_open` will be called only after the WebSocket handshake is successful
//!     // so at this point we know that the connection is ready to send/receive messages.
//!     // We ignore the `Handshake` for now, but you could also use this method to setup
//!     // Handler state or reject the connection based on the details of the Request
//!     // or Response, such as by checking cookies or Auth headers.
//!     fn on_open(&mut self, _: Handshake) -> Result<()> {
//!         // Now we don't need to call unwrap since `on_open` returns a `Result<()>`.
//!         // If this call fails, it will only result in this connection disconnecting.
//!         self.out.send("Hello WebSocket")
//!     }
//!
//!     // `on_message` is roughly equivalent to the Handler closure. It takes a `Message`
//!     // and returns a `Result<()>`.
//!     fn on_message(&mut self, msg: Message) -> Result<()> {
//!         // Close the connection when we get a response from the server
//!         println!("Got message: {}", msg);
//!         self.out.close(CloseCode::Normal)
//!     }
//! }
//!
//! // Now, instead of a closure, the Factory returns a new instance of our Handler.
//! connect("ws://127.0.0.1:3012", |out| { Client { out: out } }).unwrap()
//! ```
//!
//! That is a big increase in verbosity in order to accomplish the same effect as the
//! original example, but this way is more flexible and gives you access to more of the underlying
//! details of the WebSocket connection.
//!
//! It's also important to note that using `on_open` allows you to tie in to the lifecycle of the
//! WebSocket. If the opening handshake is successful and `on_open` is called, the WebSocket is now
//! open and alive. Until that point, it is not guaranteed that the connection will be
//! upgraded. So, if you have important state that you need to tear down, or if
//! you have some state that tracks closely the lifecycle of the WebScoket connection, it is best to
//! set that up in the `on_open` method rather than when your handler is first created.
//! If `on_open` returns Ok, then you are guaranteed that `on_close` will run when the WebSocket
//! connection is about to go down, unless a panic has occurred.
//!
//! Therefore you will probably want to implement `on_close`. This method is called anytime
//! the WebSocket connection will close. The `on_close` method implements the closing handshake of
//! the WebSocket protocol. Using `on_close` gives you a mechanism for informing the user regarding
//! why the WebSocket connection may have been closed even if no errors were encountered.
//! It also gives you an opportunity to clean up any resources or state
//! that may be dependent on the connection that is now about to disconnect.
//!
//! An example server might use this as follows:
//!
//! ```no_run
//! use ws::{listen, Handler, Sender, Result, Message, CloseCode};
//!
//! struct Server {
//!     out: Sender,
//! }
//!
//! impl Handler for Server {
//!
//!     fn on_message(&mut self, msg: Message) -> Result<()> {
//!         // Echo the message back
//!         self.out.send(msg)
//!     }
//!
//!     fn on_close(&mut self, code: CloseCode, reason: &str) {
//!         // The WebSocket protocol allows for a utf8 reason for the closing state after the
//!         // close code. WS-RS will attempt to interpret this data as a utf8 description of the
//!         // reason for closing the connection. I many cases, `reason` will be an empty string.
//!         // So, you may not normally want to display `reason` to the user,
//!         // but let's assume that we know that `reason` is human-readable.
//!         match code {
//!             CloseCode::Normal => println!("The client is done with the connection."),
//!             CloseCode::Away   => println!("The client is leaving the site."),
//!             _ => println!("The client encountered an error: {}", reason),
//!         }
//!     }
//! }
//!
//! listen("127.0.0.1:3012", |out| { Server { out: out } }).unwrap()
//! ```
//!
//! When errors occur, your handler will be informed via the `on_error` method. Depending on the
//! type of the error, the connection may or may not be about to go down. If the error is such that
//! the connection needs to close, your handler's `on_close` method will be called and WS-RS will
//! send the appropriate close code to the other endpoint if possible.
//!
//! A server that tracks state related to the life of the WebSocket connection
//! and informs the user of errors might be as follows:
//!
//! ```no_run
//!
//! use std::rc::Rc;
//! use std::cell::Cell;
//!
//! use ws::{listen, Handler, Sender, Result, Message, Handshake, CloseCode, Error};
//!
//! struct Server {
//!     out: Sender,
//!     count: Rc<Cell<u32>>,
//! }
//!
//! impl Handler for Server {
//!
//!     fn on_open(&mut self, _: Handshake) -> Result<()> {
//!         // We have a new connection, so we increment the connection counter
//!         Ok(self.count.set(self.count.get() + 1))
//!     }
//!
//!     fn on_message(&mut self, msg: Message) -> Result<()> {
//!         // Tell the user the current count
//!         println!("The number of live connections is {}", self.count.get());
//!
//!         // Echo the message back
//!         self.out.send(msg)
//!     }
//!
//!     fn on_close(&mut self, code: CloseCode, reason: &str) {
//!         match code {
//!             CloseCode::Normal => println!("The client is done with the connection."),
//!             CloseCode::Away   => println!("The client is leaving the site."),
//!             CloseCode::Abnormal => println!(
//!                 "Closing handshake failed! Unable to obtain closing status from client."),
//!             _ => println!("The client encountered an error: {}", reason),
//!         }
//!
//!         // The connection is going down, so we need to decrement the count
//!         self.count.set(self.count.get() - 1)
//!     }
//!
//!     fn on_error(&mut self, err: Error) {
//!         println!("The server encountered an error: {:?}", err);
//!     }
//!
//! }
//! // Cell gives us interior mutability so we can increment
//! // or decrement the count between handlers.
//! // Rc is a reference-counted box for sharing the count between handlers
//! // since each handler needs to own its contents.
//! let count = Rc::new(Cell::new(0));
//! listen("127.0.0.1:3012", |out| { Server { out: out, count: count.clone() } }).unwrap()
//! ```
//!
//! There are other Handler methods that allow even more fine-grained access, but most applications
//! will usually only need these four methods.
#![deny(
    missing_copy_implementations,
    trivial_casts, trivial_numeric_casts,
    unstable_features,
    unused_import_braces)]

extern crate httparse;
extern crate mio;
extern crate sha1;
extern crate rand;
extern crate url;
#[cfg(all(not(windows), feature="ssl"))] extern crate openssl;
#[macro_use] extern crate log;

mod result;
mod connection;
mod handler;
mod factory;
mod frame;
mod message;
mod handshake;
mod protocol;
mod communication;
mod io;
mod stream;

pub mod util;

pub use factory::Factory;
pub use handler::Handler;

pub use result::{Result, Error};
pub use result::Kind as ErrorKind;
pub use message::Message;
pub use communication::Sender;
pub use frame::Frame;
pub use protocol::{CloseCode, OpCode};
pub use handshake::{Handshake, Request, Response};

use std::fmt;
use std::default::Default;
use std::net::ToSocketAddrs;
use mio::EventLoopConfig;
use std::borrow::Borrow;

/// A utility function for setting up a WebSocket server.
///
/// # Safety
///
/// This function blocks until the EventLoop finishes running. Avoid calling this method within
/// another WebSocket handler.
///
/// # Examples
///
/// ```no_run
/// use ws::listen;
///
/// listen("127.0.0.1:3012", |out| {
///     move |msg| {
///        out.send(msg)
///    }
/// }).unwrap()
/// ```
///
pub fn listen<A, F, H>(addr: A, factory: F) -> Result<()>
    where
        A: ToSocketAddrs + fmt::Debug,
        F: FnMut(Sender) -> H,
        H: Handler,
{
    let ws = try!(WebSocket::new(factory));
    try!(ws.listen(addr));
    Ok(())
}

/// A utility function for setting up a WebSocket client.
///
/// # Safety
///
/// This function blocks until the EventLoop finishes running. Avoid calling this method within
/// another WebSocket handler. If you need to establish a connection from inside of a handler,
/// use the `connect` method on the Sender.
///
/// # Examples
///
/// ```no_run
/// use ws::{connect, CloseCode};
///
/// connect("ws://127.0.0.1:3012", |out| {
///     out.send("Hello WebSocket").unwrap();
///
///     move |msg| {
///         println!("Got message: {}", msg);
///         out.close(CloseCode::Normal)
///     }
/// }).unwrap()
/// ```
///
pub fn connect<U, F, H>(url: U, factory: F) -> Result<()>
    where
        U: Borrow<str>,
        F: FnMut(Sender) -> H,
        H: Handler
{
    let mut ws = try!(WebSocket::new(factory));
    let parsed = try!(
        url::Url::parse(url.borrow())
            .map_err(|err| Error::new(
                ErrorKind::Internal,
                format!("Unable to parse {} as url due to {:?}", url.borrow(), err))));
    try!(ws.connect(parsed));
    try!(ws.run());
    Ok(())
}

/// WebSocket settings
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The maximum number of connections that this WebSocket will support.
    /// The default setting is low and should be increased when expecting more
    /// connections because this is a hard limit and no new connections beyond
    /// this limit can be made until an old connection is dropped.
    /// Default: 100
    pub max_connections: usize,
    /// Whether to panic when unable to establish a new TCP connection.
    /// Default: false
    pub panic_on_new_connection: bool,
    /// Whether to panic when a shutdown of the WebSocket is requested.
    /// Default: false
    pub panic_on_shutdown: bool,
    /// The maximum number of fragments the connection can handle without reallocating.
    /// Default: 10
    pub fragments_capacity: usize,
    /// Whether to reallocate when `fragments_capacity` is reached. If this is false,
    /// a Capacity error will be triggered instead.
    /// Default: true
    pub fragments_grow: bool,
    /// The maximum length of outgoing frames. Messages longer than this will be fragmented.
    /// Default: 65,535
    pub fragment_size: usize,
    /// The size of the incoming buffer. A larger buffer uses more memory but will allow for fewer
    /// reallocations.
    /// Default: 2048
    pub in_buffer_capacity: usize,
    /// Whether to reallocate the incoming buffer when `in_buffer_capacity` is reached. If this is
    /// false, a Capacity error will be triggered instead.
    /// Default: true
    pub in_buffer_grow: bool,
    /// The size of the outgoing buffer. A larger buffer uses more memory but will allow for fewer
    /// reallocations.
    /// Default: 2048
    pub out_buffer_capacity: usize,
    /// Whether to reallocate the incoming buffer when `out_buffer_capacity` is reached. If this is
    /// false, a Capacity error will be triggered instead.
    /// Default: true
    pub out_buffer_grow: bool,
    /// Whether to panic when an Internal error is encountered. Internal errors should generally
    /// not occur, so this setting defaults to true as a debug measure, whereas production
    /// applications should consider setting it to false.
    /// Default: true
    pub panic_on_internal: bool,
    /// Whether to panic when a Capacity error is encountered.
    /// Default: false
    pub panic_on_capacity: bool,
    /// Whether to panic when a Protocol error is encountered.
    /// Default: false
    pub panic_on_protocol: bool,
    /// Whether to panic when an Encoding error is encountered.
    /// Default: false
    pub panic_on_encoding: bool,
    /// Whether to panic when an Io error is encountered.
    /// Default: false
    pub panic_on_io: bool,
    /// Whether to panic when a Timer error is encountered.
    /// Default: false
    pub panic_on_timeout: bool,
    /// The WebSocket protocol requires frames sent from client endpoints to be masked as a
    /// security and sanity precaution. Enforcing this requirement, which may be removed at some
    /// point may cause incompatibilities. If you need the extra security, set this to true.
    /// Default: false
    pub masking_strict: bool,
    /// The WebSocket protocol requires clients to verify the key returned by a server to ensure
    /// that the server and all intermediaries can perform the protocol. Verifying the key will
    /// consume processing time and other resources with the benifit that we can fail the
    /// connection early. The default in WS-RS is to accept any key from the server and instead
    /// fail late if a protocol error occurs. Change this setting to enable key verification.
    /// Default: false
    pub key_strict: bool,
    /// The WebSocket protocol requires clients to perform an opening handshake using the HTTP
    /// GET method for the request. However, since only WebSockets are supported on the connection,
    /// verifying the method of handshake requests is not always necessary. To enforce the
    /// requirement that handshakes begin with a GET method, set this to true.
    /// Default: false
    pub method_strict: bool,
    /// Indicate whether server connections should use ssl encryption when accepting connections.
    /// Setting this to true means that clients should use the `wss` scheme to connect to this
    /// server. Note that using this flag will in general necessitate overriding the
    /// `Handler::build_ssl` method in order to provide the details of the ssl context. It may be
    /// simpler for most users to use a reverse proxy such as nginx to provide server side
    /// encryption.
    ///
    /// Note: This setting is not supported on Windows.
    /// Default: false
    pub encrypt_server: bool,
}

impl Default for Settings {

    fn default() -> Settings {
        Settings {
            max_connections: 100,
            panic_on_new_connection: false,
            panic_on_shutdown: false,
            fragments_capacity: 10,
            fragments_grow: true,
            fragment_size: u16::max_value() as usize,
            in_buffer_capacity: 2048,
            in_buffer_grow: true,
            out_buffer_capacity: 2048,
            out_buffer_grow: true,
            panic_on_internal: true,
            panic_on_capacity: false,
            panic_on_protocol: false,
            panic_on_encoding: false,
            panic_on_io: false,
            panic_on_timeout: false,
            masking_strict: false,
            key_strict: false,
            method_strict: false,
            encrypt_server: false,
        }
    }
}


/// The WebSocket struct. A WebSocket can support multiple incoming and outgoing connections.
pub struct WebSocket<F>
    where F: Factory
{
    event_loop: io::Loop<F>,
    handler: io::Handler<F>,
}

impl<F> WebSocket<F>
    where F: Factory
{
    /// Create a new WebSocket using the given Factory to create handlers.
    pub fn new(factory: F) -> Result<WebSocket<F>> {
        let settings = Settings::default();
        let mut config = EventLoopConfig::default();
        config.notify_capacity(settings.max_connections * 5);  // every handler can do 5 things at once
        Ok(WebSocket {
            event_loop: try!(io::Loop::configured(config)),
            handler: io::Handler::new(factory, settings),
        })
    }

    /// Create a new WebSocket with a Factory and use the event loop config to
    /// configure the event loop.
    pub fn with_config(factory: F, config: EventLoopConfig) -> Result<WebSocket<F>> {
        warn!("The with_config method is deprecated and will be removed in a future version.");
        Ok(WebSocket {
            event_loop: try!(io::Loop::configured(config)),
            handler: io::Handler::new(factory, Settings::default()),
        })
    }

    /// Consume the WebSocket and listen for new connections on the specified address.
    ///
    /// # Safety
    ///
    /// This method will block until the event loop finishes running.
    pub fn listen<A>(mut self, addr_spec: A) -> Result<WebSocket<F>>
        where A: ToSocketAddrs + fmt::Debug
    {
        let mut result = Err(Error::new(ErrorKind::Internal, format!("Unable to listen on {:?}", addr_spec)));

        for addr in try!(addr_spec.to_socket_addrs()) {
            result = self.handler.listen(&mut self.event_loop, &addr).map(|_| ());
            if result.is_ok() {
                info!("Listening for new connections on {}.", addr);
                return self.run()
            }
        }

        result.map(|_| self)
    }

    /// Queue an outgoing connection on this WebSocket. This method may be called multiple times,
    /// but the actuall connections will not be established until after `run` is called.
    pub fn connect(&mut self, url: url::Url) -> Result<&mut WebSocket<F>> {
        let sender = Sender::new(io::ALL, self.event_loop.channel());
        info!("Queuing connection to {}", url);
        try!(sender.connect(url));
        Ok(self)
    }

    /// Run the WebSocket. This will run the encapsulated event loop blocking until the WebSocket
    /// is shutdown.
    pub fn run(mut self) -> Result<WebSocket<F>> {
        try!(self.event_loop.run(&mut self.handler));
        Ok(self)
    }

    /// Get a Sender that can be used to send messages on all connections.
    /// Calling `send` on this Sender is equivalent to calling `broadcast`.
    /// Calling `shutdown` on this Sender will shudown the WebSocket even if no connections have
    /// been established.
    #[inline]
    pub fn broadcaster(&self) -> Sender {
        Sender::new(io::ALL, self.event_loop.channel())
    }
}

/// Utility for constructing a WebSocket from various settings.
#[allow(missing_copy_implementations)]
#[derive(Debug)]
pub struct Builder {
    event_config: Option<EventLoopConfig>,
    settings: Settings,
}

// TODO: add convenience methods for each setting
impl Builder {
    /// Create a new Builder with default settings.
    pub fn new() -> Builder {
        Builder {
            event_config: None,
            settings: Settings::default(),
        }
    }

    /// Build a WebSocket using this builder and a factory.
    /// It is possible to use the same builder to create multiple WebSockets.
    pub fn build<F>(&self, factory: F) -> Result<WebSocket<F>>
        where F: Factory
    {
        let mut event_config: EventLoopConfig;

        if let Some(ref config) = self.event_config {
            event_config = config.clone();
        } else {
            event_config = EventLoopConfig::default();
            event_config.notify_capacity(self.settings.max_connections * 5);
        }
        Ok(WebSocket {
            event_loop: try!(io::Loop::configured(event_config)),
            handler: io::Handler::new(factory, self.settings),
        })
    }

    /// Set the EventLoopConfig to use with this WebSocket. If this is not set
    /// the builder will use a default EventLoopConfig based on other settings.
    pub fn with_config(&mut self, config: EventLoopConfig) -> &mut Builder {
        self.event_config = Some(config);
        self
    }

    /// Set the WebSocket settings to use.
    pub fn with_settings(&mut self, settings: Settings) -> &mut Builder {
        self.settings = settings;
        self
    }
}
