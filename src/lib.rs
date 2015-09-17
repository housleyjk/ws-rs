//! Usage
//! -----
//!
//! For simple applications, use one of the utility functions `listen` and `connect`:
//!
//! `listen` accpets a string that represents a socket address and a Factory, see
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
//! though they are threads and Rust's memory model should make sense. Closure Handlers must return
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
//! You may have noticed in the usage exmaples that the client example calls `unwrap` when sending the first
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
//! Another method you will probably want to implement is `on_close`. This method is called anytime
//! the other side of the WebSocket connection attempts to close the connection. Implementing
//! `on_close` gives you a mechanism for informing the user regarding why the WebSocket connection
//! may have been closed, and it also gives you an opportunity to clean up any resources or state
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
//! Errors don't just occur on the other side of the connection, sometimes your code will encounter
//! an exceptional state too. You can access errors by implementing `on_error`. By implementing
//! `on_error` you can inform the user of an error and tear down any resources that you may have
//! setup for the connection, but which are not owned by the Handler. Also, note that certain kinds
//! of errors have certain ramifications within the WebSocket protocol. WS-RS will take care of
//! sending the appropriate close code.
//!
//! A server that tracks state outside of the handler might be as follows:
//!
//! ```no_run
//!
//! use std::rc::Rc;
//! use std::cell::RefCell;
//!
//! use ws::{listen, Handler, Sender, Result, Message, Handshake, CloseCode, Error};
//!
//! struct Server {
//!     out: Sender,
//!     count: Rc<RefCell<usize>>,
//! }
//!
//! impl Handler for Server {
//!
//!     fn on_open(&mut self, _: Handshake) -> Result<()> {
//!         // We have a new connection, so we increment the connection counter
//!         Ok(*self.count.borrow_mut() += 1)
//!     }
//!
//!     fn on_message(&mut self, msg: Message) -> Result<()> {
//!         // Tell the user the current count
//!         println!("The number of live connections is {}", *self.count.borrow());
//!
//!         // Echo the message back
//!         self.out.send(msg)
//!     }
//!
//!     fn on_close(&mut self, code: CloseCode, reason: &str) {
//!         match code {
//!             CloseCode::Normal => println!("The client is done with the connection."),
//!             CloseCode::Away   => println!("The client is leaving the site."),
//!             _ => println!("The client encountered an error: {}", reason),
//!         }
//!
//!         // The connection is going down, so we need to decrement the count
//!         *self.count.borrow_mut() -= 1
//!     }
//!
//!     fn on_error(&mut self, err: Error) {
//!         println!("The server encountered an error: {:?}", err);
//!
//!         // The connection is going down, so we need to decrement the count
//!         *self.count.borrow_mut() -= 1
//!     }
//!
//! }
//! // RefCell enforces Rust borrowing rules at runtime.
//! // Calling borrow_mut will panic if the count being borrowed,
//! // but we know already that only one handler at a time will ever try to change the count.
//! // Rc is a reference-counted box for sharing the count between handlers
//! // since each handler needs to own its contents.
//! let count = Rc::new(RefCell::new(0));
//! listen("127.0.0.1:3012", |out| { Server { out: out, count: count.clone() } }).unwrap()
//! ```
//!
//! There are other Handler methods that allow even more fine-grained access, but most applications
//! will usually only need these four methods.
//!

extern crate httparse;
extern crate mio;
extern crate sha1;
extern crate rand;
extern crate url;
#[macro_use] extern crate log;

mod result;
mod connection;
mod frame;
mod message;
mod handshake;
mod protocol;
mod communication;
mod io;

pub use connection::factory::Factory;
pub use connection::factory::Settings as WebSocketSettings;
pub use connection::handler::Handler;
pub use connection::handler::Settings as ConnectionSettings;

pub use result::{Result, Error};
pub use result::Kind as ErrorKind;
pub use message::Message;
pub use communication::Sender;
pub use protocol::CloseCode;
pub use handshake::{Handshake, Request, Response};

use std::fmt;
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
    pub fn new(mut factory: F) -> Result<WebSocket<F>> {
        let max = factory.settings().max_connections;
        WebSocket::with_config(
            factory,
            EventLoopConfig {
                notify_capacity: max + 1000,
                .. EventLoopConfig::default()
            },
        )
    }

    /// Create a new WebSocket with a Factory and use the event loop config to provide settings for
    /// the event loop.
    pub fn with_config(factory: F, config: EventLoopConfig) -> Result<WebSocket<F>> {
        Ok(WebSocket {
            event_loop: try!(io::Loop::configured(config)),
            handler: io::Handler::new(factory),
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
                return self.run()
            }
        }

        result.map(|_| self)
    }

    /// Queue an outgoing connection on this WebSocket. This method may be called multiple times,
    /// but the actuall connections will not be established until after `run` is called.
    pub fn connect(&mut self, url: url::Url) -> Result<&mut WebSocket<F>> {
        let sender = Sender::new(io::ALL, self.event_loop.channel());
        try!(sender.connect(url));
        Ok(self)
    }

    /// Run the WebSocket. This will run the encapsulated event loop blocking until the WebSocket
    /// is shutdown.
    pub fn run(mut self) -> Result<WebSocket<F>> {
        try!(self.event_loop.run(&mut self.handler));
        Ok(self)
    }
}

