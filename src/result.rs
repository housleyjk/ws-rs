use std::fmt;
use std::io;
use std::borrow::Cow;
use std::str::Utf8Error;
use std::result::Result as StdResult;
use std::error::Error as StdError;
use std::convert::{From, Into};

use httparse;
use mio;

use communication::Command;

pub type Result<T> = StdResult<T, Error>;

#[derive(Debug)]
pub enum Kind {
    Internal,  // will attempt to send 1011 close code
    Capacity, // out of space, will attempt to to send 1009
    Protocol, // data doesn't match protocol, will attempt to send 1002 or drop the connection immediately, if the handshake is not yet complete, this will trigger a 400
    Encoding(Utf8Error), // data is not valid utf8, will attempt to send 1007
    Io(io::Error), // failure to perform io, instant shutdown
    Parse(httparse::Error), // failure to parse http request/response, will attempt to send 500
    Queue(mio::NotifyError<Command>), // unable to send message, instant shutdown
    Custom(Box<StdError>), // custom error to allow library clients to return errors from deep within the application, no default behavior
}

pub struct Error {
    pub kind: Kind,
    pub details: Cow<'static, str>,
}

impl Error {

    pub fn new<I>(kind: Kind, details: I) -> Error
        where I: Into<Cow<'static, str>>
    {
        Error {
            kind: kind,
            details: details.into(),
        }
    }

    pub fn into_box(self) -> Box<StdError> {
        match self.kind {
            Kind::Custom(err) => err,
            _ => Box::new(self),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.details.len() > 0 {
            write!(f, "WS Error <{:?}>: {}", self.kind, self.details)
        } else {
            write!(f, "WS Error <{:?}>", self.kind)
        }
    }
}

impl fmt::Display for Error {

    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.details.len() > 0 {
            write!(f, "{}: {}", self.description(), self.details)
        } else {
            write!(f, "{}", self.description())
        }
    }
}

impl StdError for Error {

    fn description(&self) -> &str {
        match self.kind {
            Kind::Internal          => "Internal Application Error",
            Kind::Capacity          => "WebSocket at Capacity",
            Kind::Protocol          => "WebSocket Protocol Error",
            Kind::Encoding(ref err) => err.description(),
            Kind::Io(ref err)       => err.description(),
            Kind::Parse(_)          => "Unable to parse HTTP",
            Kind::Queue(_)          => "Unable to send signal on event loop",
            Kind::Custom(ref err)   => err.description(),
        }
    }

    fn cause(&self) -> Option<&StdError> {
        match self.kind {
            Kind::Encoding(ref err) => Some(err),
            Kind::Io(ref err)       => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::new(Kind::Io(err), "")
    }
}

impl From<httparse::Error> for Error {

    fn from(err: httparse::Error) -> Error {
        // TODO: match on parse error to determine details
        Error::new(Kind::Parse(err), "")
    }

}

impl From<mio::NotifyError<Command>> for Error {

    fn from(err: mio::NotifyError<Command>) -> Error {
        match err {
            mio::NotifyError::Io(err) => Error::from(err),
            _ => Error::new(Kind::Queue(err), "")
        }
    }

}

impl From<Utf8Error> for Error {
    fn from(err: Utf8Error) -> Error {
        Error::new(Kind::Encoding(err), "")
    }
}

impl From<Box<StdError>> for Error {
    fn from(err: Box<StdError>) -> Error {
        Error::new(Kind::Custom(err), "")
    }
}
