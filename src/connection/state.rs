use handshake::Handshake;

#[derive(Debug)]
pub enum State {
    // Tcp connection accepted, waiting for handshake to complete
    Connecting(Handshake),
    // Ready to send/receive messages
    Open,
    // Close frame sent/received
    Closing,
}

/// a little more semantic than a boolean
#[derive(Debug, Eq, PartialEq)]
pub enum Endpoint {
    /// will mask outgoing frames
    Client,
    /// won't mask outgoing frames
    Server,
}

impl State {

    #[inline]
    pub fn is_connecting(&self) -> bool {
        match *self {
            State::Connecting(_) => true,
            _ => false,
        }
    }

    #[allow(dead_code)]
    #[inline]
    pub fn is_open(&self) -> bool {
        match *self {
            State::Open => true,
            _ => false,
        }
    }

    #[inline]
    pub fn is_closing(&self) -> bool {
        match *self {
            State::Closing => true,
            _ => false,
        }
    }
}
