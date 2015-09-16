use std::fmt;
use std::convert::{From, Into};
use std::str::from_utf8;
use std::result::Result as StdResult;

use protocol::OpCode;
use result::Result;

use self::Message::*;

/// An enum representing the various forms of a WebSocket message.
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message {
    /// A text WebSocket message
	Text(String),
    /// A binary WebSocket message
	Binary(Vec<u8>),
}

impl Message {

    /// Create a new text WebSocket message from a stringable.
    pub fn text<S>(string: S) -> Message
        where S: Into<String>
    {
        Message::Text(string.into())
    }

    /// Create a new binary WebSocket message by converting to Vec<u8>.
    pub fn binary<B>(bin: B) -> Message
        where B: Into<Vec<u8>>
    {
        Message::Binary(bin.into())
    }

    /// Get the length of the WebSocket message.
    pub fn len(&self) -> usize {
        match *self {
            Text(ref string) => string.len(),
            Binary(ref data) => data.len(),
        }
    }

    #[doc(hidden)]
    pub fn opcode(&self) -> OpCode {
        match *self {
            Text(_) => OpCode::Text,
            Binary(_) => OpCode::Binary,
        }
    }

    /// Consume the WebSocket and return it as binary data.
    pub fn into_data(self) -> Vec<u8> {
        match self {
            Text(string) => string.into_bytes(),
            Binary(data) => data,
        }
    }

    /// Attempt to consume the WebSocket message and convert it to a String.
    pub fn into_text(self) -> Result<String> {
        match self {
            Text(string) => Ok(string),
            Binary(data) => Ok(try!(
                String::from_utf8(data).map_err(|err| err.utf8_error()))),
        }
    }

    /// Attempt to get a &str from the WebSocket message,
    /// this will try to convert binary data to utf8.
    pub fn as_text(&self) -> Result<&str> {
        match *self {
            Text(ref string) => Ok(string),
            Binary(ref data) => Ok(try!(from_utf8(data))),
        }
    }
}

impl From<String> for Message {

    fn from(string: String) -> Message {
        Message::text(string)
    }
}

impl<'s> From<&'s str> for Message {

    fn from(string: &'s str) -> Message {
        Message::text(string)
    }
}

impl<'b> From<&'b [u8]> for Message {

    fn from(data: &'b [u8]) -> Message {
        Message::binary(data)
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> StdResult<(), fmt::Error> {
        if let Ok(string) = self.as_text() {
            write!(f, "{}", string)
        } else {
            write!(f, "Binary Data<length={}>", self.len())
        }
    }
}
