
use log::LogLevel::Error as ErrorLevel;

use message::Message;
use frame::Frame;
use protocol::CloseCode;
use handshake::{Handshake, Request, Response};
use result::{Result, Error, Kind};


/// The core trait of this library.
/// Implementing this trait provides the business logic of the WebSocket application.
pub trait Handler {

    // general

    /// Called when a request to shutdown all connections has been received.
    #[inline]
    fn on_shutdown(&mut self) {
        debug!("Handler received WebSocket shutdown request.");
    }

    // WebSocket events

    /// Called when the WebSocket handshake is successful and the connection is open for sending
    /// and receiving messages.
    fn on_open(&mut self, shake: Handshake) -> Result<()> {
        trace!("Connection opened with {:?}", shake);
        Ok(())
    }

    /// Called on incoming messages.
    fn on_message(&mut self, msg: Message) -> Result<()> {
        debug!("Received message {:?}", msg);
        Ok(())
    }

    /// Called when the other endpoint is asking to close the connection.
    fn on_close(&mut self, code: CloseCode, reason: &str) {
        debug!("Connection closing due to ({:?}) {}", code, reason);
    }

    /// Called when an error occurs on the WebSocket.
    fn on_error(&mut self, err: Error) {
        // Ignore connection reset errors by default, but allow library clients to see them by
        // overriding this method if they want
        if let Kind::Io(ref err) = err.kind {
            if let Some(104) = err.raw_os_error() {
                return
            }
        }

        error!("{:?}", err);
        if !log_enabled!(ErrorLevel) {
            println!("Encountered an error: {}\nEnable a logger to see more information.", err);
        }
    }

    // handshake events

    /// A method for handling the low-level workings of the request portion of the WebSocket
    /// handshake. Implementors can inspect the Request and select an appropriate string protocol
    /// and/or extensions to be used. This method will not be called when the handler represents
    /// a client endpoint.
    #[inline]
    fn on_request(&mut self, req: &Request) -> Result<(Option<&str>, Option<&str>)> {
        trace!("Handler received request: {:?}", req);
        Ok((None, None))
    }

    /// A method for handling the low-level workings of the response portion of the WebSocket
    /// handshake. Implementors can inspect the Response and choose to faile the connection by
    /// returning an error. This method will not be called when the handler represents a server
    /// endpoint.
    #[inline]
    fn on_response(&mut self, res: &Response) -> Result<()> {
        trace!("Handler received response: {:?}", res);
        Ok(())
    }

    // frame events

    // A method for handling ping frames.  Returning Ok(None) indicates that the handler will
    // takeover processing the frame. Implementors are then responsible for sending an appropriate
    // Pong frame.
    #[inline]
    fn on_ping_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received ping frame: {:?}", frame);
        Ok(Some(frame))
    }

    // return Some(pong) to pass the pong back, which will mean that default validation may run
    // return None to takeover processing the pong yourself
    #[inline]
    fn on_pong_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received pong frame: {:?}", frame);
        Ok(Some(frame))
    }

    // return Some(frame) to pass the close frame back, which will send a closing response frame and
    // trigger on_close. This method is only called when the connection is still open, if the
    // connection is already closing (because you sent a close frame or an error occured on your
    // side), this method will not be called
    // return None to takeover processing the close yourself
    #[inline]
    fn on_close_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received close frame: {:?}", frame);
        Ok(Some(frame))
    }

    // This method is only called when the message is not fragmented
    // return Some(frame) to pass the frame back and continue processing it into a message
    // return None to takeover processing the frame yourself
    #[inline]
    fn on_binary_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received binary frame: {:?}", frame);
        Ok(Some(frame))
    }

    // This method is only called when the message is not fragmented
    // return Ok(frame) to pass the frame back and continue processing it into a message
    // return Ok(None) to takeover processing the frame yourself
    #[inline]
    fn on_text_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received text frame: {:?}", frame);
        Ok(Some(frame))
    }

    // This method is only called when the message is fragmented, and it should be called with
    // each fragment in order
    // return Ok(frame) to pass the frame back and continue processing it into a message
    // return Ok(None) to takeover processing the frame yourself, if you do this, you must do it
    // for all fragments of the message, otherwise the message will be incomplete when sent to the
    // message handler
    #[inline]
    fn on_fragmented_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received fragment: {:?}", frame);
        Ok(Some(frame))
    }

}

impl<F> Handler for F
    where F: Fn(Message) -> Result<()>
{
    fn on_message(&mut self, msg: Message) -> Result<()> {
        self(msg)
    }
}

mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use super::*;
    use mio;
    use handshake::{Request, Response, Handshake};
    use protocol::CloseCode;
    use frame;
    use message;
    use result::Result;

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
    fn test_handler() {
        struct H;

        impl Handler for H {

            fn on_open(&mut self, mut shake: Handshake) -> Result<()> {
                Ok(assert!(shake.request.buffer().capacity() == 2048))
            }

            fn on_message(&mut self, msg: message::Message) -> Result<()> {
                Ok(assert_eq!(msg, message::Message::Text(String::from("testme"))))
            }

            fn on_close(&mut self, code: CloseCode, _: &str) {
                assert_eq!(code, CloseCode::Normal)
            }

        }

        let mut h = H;
        h.on_open(Handshake::default()).unwrap();
        h.on_message(message::Message::Text("testme".to_owned())).unwrap();
        h.on_close(CloseCode::Normal, "");
    }

    #[test]
    fn test_closure_handler() {
        let mut close = |msg| {
            assert_eq!(msg, message::Message::Binary(vec![1, 2, 3]));
            Ok(())
        };

        close.on_message(message::Message::Binary(vec![1, 2, 3])).unwrap();
    }
}

