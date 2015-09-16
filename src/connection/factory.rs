use std::default::Default;

use super::handler::Handler;
use communication::Sender;

/// A trait for creating new WebSocket handlers.
pub trait Factory {
    type Handler: Handler;

    /// Called when a TCP connection is made
    fn connection_made(&mut self, Sender) -> Self::Handler;

    #[inline]
    fn settings(&mut self) -> Settings {
        Settings::default()
    }

    #[inline]
    fn on_shutdown(&mut self) {
        debug!("Factory received WebSocket shutdown request.");
    }

}

impl<F, H> Factory for F
    where H: Handler, F: FnMut(Sender) -> H
{
    type Handler = H;

    fn connection_made(&mut self, out: Sender) -> H {
        self(out)
    }

}

/// Settings that apply to multiple connections.
pub struct Settings {
    /// The maximum number of connections that this WebSocket will support.
    /// Default: 10,000
    pub max_connections: usize,
    /// Whether to panic when unable to establish a new TCP connection.
    /// Default: false
    pub panic_on_new_connection: bool,
    /// Whether to panic when a shutdown of the WebSocket is requested.
    /// Default: false
    pub panic_on_shutdown: bool,
    /// A protocol string representing the subprotocols that this WebSocket can support. This will
    /// be sent in Requests to server endpoints to help determine a subprotocol if any for the
    /// connection.
    /// Default: None
    pub protocols: Option<&'static str>,
    /// A WebSocket extension string indicating the extensions that this WebSocket can support.
    /// Default: None
    pub extensions: Option<&'static str>,
}

impl Default for Settings {

    fn default() -> Settings {
        Settings {
            max_connections: 10_000,
            panic_on_new_connection: false,
            panic_on_shutdown: false,
            protocols: None,
            extensions: None,
        }
    }
}


mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use super::*;
    use mio;
    use communication::{Command, Sender};
    use handshake::{Request, Response, Handshake};
    use protocol::CloseCode;
    use frame;
    use message;
    use connection::handler::Handler;
    use result::Result;

    struct S;

    impl mio::Handler for S {
        type Message = Command;
        type Timeout = ();
    }

    #[derive(Debug, Eq, PartialEq)]
    struct M;
    impl Handler for M {
        fn on_message(&mut self, _: message::Message) -> Result<()> {
            Ok(println!("dude"))
        }

        fn on_ping_frame(&mut self, f: frame::Frame) -> Result<Option<frame::Frame>> {
            Ok(None)
        }
    }

    #[test]
    fn test_impl_factory() {

        struct X;

        impl Factory for X {
            type Handler = M;
            fn connection_made(&mut self, _: Sender) -> M {
                M
            }
        }

        let event_loop = mio::EventLoop::<S>::new().unwrap();

        let mut x = X;
        let m = x.connection_made(
            Sender::new(mio::Token(0), event_loop.channel())
        );
        assert_eq!(m, M);
    }

    #[test]
    fn test_closure_factory() {
        let event_loop = mio::EventLoop::<S>::new().unwrap();

        let mut factory = |_| {
            |_| {Ok(())}
        };

        factory.connection_made(
            Sender::new(mio::Token(0), event_loop.channel())
        );
    }
}
