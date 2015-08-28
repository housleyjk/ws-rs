#![allow(unused_imports, unused_variables, dead_code)]
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
pub use connection::handler::Handler;

#[doc(no_inline)]
pub use result::{Result, Error};
pub use result::Kind as ErrorKind;
pub use message::Message;
pub use communication::Sender;
pub use protocol::CloseCode;
pub use handshake::Handshake;

use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use mio::EventLoopConfig;

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

pub fn connect<F, H>(url: url::Url, factory: F) -> Result<()>
    where F: FnMut(Sender) -> H, H: Handler
{
    let mut ws = try!(WebSocket::new(factory));
    try!(ws.connect(url));
    try!(ws.run());
    Ok(())
}


pub struct WebSocket<F>
    where F: Factory
{
    event_loop: io::Loop<F>,
    handler: io::Handler<F>,
}

impl<F> WebSocket<F>
    where F: Factory
{
    pub fn new(factory: F) -> Result<WebSocket<F>> {
        Ok(try!(WebSocket::with_capacity(factory, 15_000)))
    }

    pub fn with_capacity(factory: F, conns: usize) -> Result<WebSocket<F>> {
        Ok(WebSocket {
            event_loop: try!(io::Loop::configured(
                EventLoopConfig {
                    notify_capacity: 15_000,
                    .. EventLoopConfig::default()
                })),
            handler: io::Handler::with_capacity(factory, conns),
        })
    }

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

    pub fn connect(&mut self, url: url::Url) -> Result<&mut WebSocket<F>> {
        let sender = Sender::new(io::ALL, self.event_loop.channel());
        try!(sender.connect(url));
        Ok(self)
    }

    pub fn run(mut self) -> Result<WebSocket<F>> {
        try!(self.event_loop.run(&mut self.handler));
        Ok(self)
    }
}

