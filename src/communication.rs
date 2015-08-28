use std::convert::Into;

use url;
use mio;
use mio::Token;

use message;
use result::Result;
use protocol::CloseCode;
use io::ALL;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Signal {
    Message(message::Message),
    Close(CloseCode),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Connect(url::Url),
    Shutdown,
    // Stats
}

#[derive(Debug, PartialEq, Eq, Clone)]
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

#[derive(Debug, Clone)]
pub struct Sender {
    token: Token,
    channel: mio::Sender<Command>,
}

impl Sender {

    #[inline]
    pub fn new(token: Token, channel: mio::Sender<Command>) -> Sender {
        Sender {
            token: token,
            channel: channel,
        }
    }

    #[inline]
    pub fn token(&self) -> Token {
        self.token
    }

    #[inline]
    pub fn send<M>(&self, msg: M) -> Result<()>
        where M: Into<message::Message>
    {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Message(msg.into()),
        })))
    }

    #[inline]
    pub fn broadcast<M>(&self, msg: M) -> Result<()>
        where M: Into<message::Message>
    {
        Ok(try!(self.channel.send(Command {
            token: ALL,
            signal: Signal::Message(msg.into()),
        })))
    }

    #[inline]
    pub fn close(&self, code: CloseCode) -> Result<()> {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Close(code),
        })))
    }

    #[inline]
    pub fn ping(&self, data: Vec<u8>) -> Result<()> {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Ping(data),
        })))
    }

    #[inline]
    pub fn pong(&self, data: Vec<u8>) -> Result<()> {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Pong(data),
        })))
    }

    #[inline]
    pub fn connect(&self, url: url::Url) -> Result<()> {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Connect(url),
        })))
    }

    #[inline]
    pub fn shutdown(&self) -> Result<()> {
        Ok(try!(self.channel.send(Command {
            token: self.token,
            signal: Signal::Shutdown,
        })))
    }

}

