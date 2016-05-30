use std::convert::Into;
use std::borrow::Cow;
use std::net::SocketAddr;
use url;
use mio;
use mio::Token;

use message;
use result::{Result, Error};
use protocol::CloseCode;
use io::ALL;

#[derive(Clone)]
pub enum Signal {
    Message(message::Message),
    Close(CloseCode, Cow<'static, str>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Connect(url::Url),
    Shutdown,
    Timeout {
        delay: u64,
        token: Token,
    },
    Cancel(mio::Timeout),
}

#[derive(Clone)]
pub struct Command {
    token: Token,
    signal: Signal,
}

impl Command {
    pub fn token(&self) -> Token {
        self.token
    }

    pub fn into_signal(self) -> Signal {
        self.signal
    }
}

/// A representation of the output of the WebSocket connection. Use this to send messages to the
/// other endpoint.
#[derive(Debug, Clone)]
pub struct Sender {
    token: Token,
    channel: mio::Sender<Command>,
    addr: Option<SocketAddr>
}

impl Sender {

    #[doc(hidden)]
    #[inline]
    pub fn new(token: Token, channel: mio::Sender<Command>) -> Sender {
        Sender {
            token: token,
            channel: channel,
            addr:None
        }
    }

    #[inline]
    pub fn set_addr(&mut self, addr:SocketAddr){
        self.addr = Some(addr);
    }

    #[inline]
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// A Token identifying this sender within the WebSocket.
    #[inline]
    pub fn token(&self) -> Token {
        self.token
    }

    /// Send a message over the connection.
    #[inline]
    pub fn send<M>(&self, msg: M) -> Result<()>
        where M: Into<message::Message>
    {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Message(msg.into()),
        }).map_err(Error::from)
    }

    /// Send a message to the endpoints of all connections.
    ///
    /// Be careful with this method because it
    /// does not discriminate between client and server connections, only connections that are not
    /// part of *this* side of the WebSocket. If your WebSocket is only functioning as a server,
    /// then usage is simple, however if you have a WebSocket that is listening for
    /// connections and is also connected to another WebSocket, this method will broadcast a
    /// message to all the clients connected and to the server at the other end of the single
    /// client connection.
    #[inline]
    pub fn broadcast<M>(&self, msg: M) -> Result<()>
        where M: Into<message::Message>
    {
        self.channel.send(Command {
            token: ALL,
            signal: Signal::Message(msg.into()),
        }).map_err(Error::from)
    }

    /// Send a close code to the other endpoint.
    #[inline]
    pub fn close(&self, code: CloseCode) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Close(code, "".into()),
        }).map_err(Error::from)
    }

    /// Send a close code and provide a descriptive reason for closing.
    #[inline]
    pub fn close_with_reason<S>(&self, code: CloseCode, reason: S) -> Result<()>
        where S: Into<Cow<'static, str>>
    {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Close(code, reason.into()),
        }).map_err(Error::from)
    }

    /// Send a ping to the other endpoint with the given test data.
    #[inline]
    pub fn ping(&self, data: Vec<u8>) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Ping(data),
        }).map_err(Error::from)
    }

    /// Send a pong to the other endpoint responding with the given test data.
    #[inline]
    pub fn pong(&self, data: Vec<u8>) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Pong(data),
        }).map_err(Error::from)
    }

    /// Queue a new connection on this WebSocket to the specified URL.
    #[inline]
    pub fn connect(&self, url: url::Url) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Connect(url),
        }).map_err(Error::from)
    }

    /// Request that all connections terminate and that the WebSocket stop running.
    #[inline]
    pub fn shutdown(&self) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Shutdown,
        }).map_err(Error::from)
    }

    /// Schedule a `token` to be sent to the WebSocket Handler's `on_timeout` method
    /// after `ms` milliseconds
    #[inline]
    pub fn timeout(&self, ms: u64, token: Token) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Timeout {
                delay: ms,
                token: token,
            },
        }).map_err(Error::from)
    }

    /// Queue the cancellation of a previously scheduled timeout.
    ///
    /// This method is not guaranteed to prevent the timeout from occuring, because it is
    /// possible to call this method after a timeout has already occured. It is still necessary to
    /// handle spurious timeouts.
    #[inline]
    pub fn cancel(&self, timeout: mio::Timeout) -> Result<()> {
        self.channel.send(Command {
            token: self.token,
            signal: Signal::Cancel(timeout),
        }).map_err(Error::from)
    }

}
