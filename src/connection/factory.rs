
use super::handler::Handler;
use communication::Sender;

pub trait Factory {
    type Handler: Handler;

    /// indicates a tcp connection made, so now we need to build the websocket connection handler
    fn connection_made(&mut self, Sender) -> Self::Handler;

    #[inline]
    fn settings(&mut self) {
        // TODO: setup websocket settings
    }

    #[inline]
    fn on_shutdown(&mut self) {
        // TODO: add panic on shutdown factory setting
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


mod test {
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
