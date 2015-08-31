use std::default::Default;

use log::LogLevel::Error as ErrorLevel;

use message::Message;
use frame::Frame;
use protocol::CloseCode;
use handshake::{Handshake, Request, Response};
use result::{Result, Error, Kind};


/// all of these methods are called when receiving data from another endpoint not when data is sent
/// from this endpoint
pub trait Handler {

    // general

    #[inline]
    fn settings(&mut self) -> Settings {
        Settings::default()
    }

    #[inline]
    fn on_shutdown(&mut self) {
        debug!("Handler received WebSocket shutdown request.");
    }

    // WebSocket events

    fn on_open(&mut self, shake: Handshake) -> Result<()> {
        trace!("Connection opened with {:?}", shake);
        Ok(())
    }

    fn on_message(&mut self, msg: Message) -> Result<()> {
        debug!("Received message {:?}", msg);
        Ok(())
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {
        debug!("Connection closing due to ({:?}) {}", code, reason);
    }

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

    /// return an Error to reject the request
    #[inline]
    fn on_request(&mut self, req: &Request) -> Result<()> {
        trace!("Handler received request: {:?}", req);
        Ok(())
    }

    /// return an Error to reject the handshake response
    #[inline]
    fn on_response(&mut self, res: &Response) -> Result<()> {
        trace!("Handler received response: {:?}", res);
        Ok(())
    }

    // frame events

    /// return Some(ping) to allow a pong to be sent for you
    /// return None to takeover processing the ping, you will need to send a pong yourself
    #[inline]
    fn on_ping_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received ping frame: {:?}", frame);
        Ok(Some(frame))
    }

    /// return Some(pong) to pass the pong back, which will mean that default validation may run
    /// return None to takeover processing the pong yourself
    #[inline]
    fn on_pong_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received pong frame: {:?}", frame);
        Ok(Some(frame))
    }

    /// return Some(frame) to pass the close frame back, which will send a closing response frame and
    /// trigger on_close. This method is only called when the connection is still open, if the
    /// connection is already closing (because you sent a close frame or an error occured on your
    /// side), this method will not be called
    /// return None to takeover processing the close yourself
    #[inline]
    fn on_close_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received close frame: {:?}", frame);
        Ok(Some(frame))
    }

    /// This method is only called when the message is not fragmented
    /// return Some(frame) to pass the frame back and continue processing it into a message
    /// return None to takeover processing the frame yourself
    #[inline]
    fn on_binary_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received binary frame: {:?}", frame);
        Ok(Some(frame))
    }

    /// This method is only called when the message is not fragmented
    /// return Ok(frame) to pass the frame back and continue processing it into a message
    /// return Ok(None) to takeover processing the frame yourself
    #[inline]
    fn on_text_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        trace!("Handler received text frame: {:?}", frame);
        Ok(Some(frame))
    }

    /// This method is only called when the message is fragmented, and it should be called with
    /// each fragment in order
    /// return Ok(frame) to pass the frame back and continue processing it into a message
    /// return Ok(None) to takeover processing the frame yourself, if you do this, you must do it
    /// for all fragments of the message, otherwise the message will be incomplete when sent to the
    /// message handler
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

pub struct Settings {
    pub fragments_capacity: usize,
    pub fragments_grow: bool,
    pub fragment_size: usize,
    pub in_buffer_capacity: usize,
    pub in_buffer_grow: bool,
    pub out_buffer_capacity: usize,
    pub out_buffer_grow: bool,
    pub masking_strict: bool,
    pub panic_on_internal: bool,
    pub panic_on_capacity: bool,
    pub panic_on_protocol: bool,
    pub panic_on_encoding: bool,
    pub panic_on_io: bool,
}

impl Default for Settings {

    fn default() -> Settings {
        Settings {
            fragments_capacity: 10,
            fragments_grow: true,
            fragment_size: u16::max_value() as usize,
            in_buffer_capacity: 2048,
            in_buffer_grow: true,
            out_buffer_capacity: 2048,
            out_buffer_grow: true,
            masking_strict: true,
            panic_on_internal: true,
            panic_on_capacity: false,
            panic_on_protocol: false,
            panic_on_encoding: false,
            panic_on_io: false,
        }
    }
}

mod test {
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
        h.on_message(message::Message::Text("testme".to_string())).unwrap();
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

