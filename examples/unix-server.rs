extern crate env_logger;
/// Simple WebSocket server with error handling. It is not necessary to setup logging, but doing
/// so will allow you to see more details about the connection by using the RUST_LOG env variable.
extern crate ws;

use std::path::Path;

use ws::{listen_unix, Handler, Response, Sender};


struct Behavior {
    sender: Sender,
}

impl Handler for Behavior {

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        println!("Get meg: {}", msg);
        self.sender.send(msg).unwrap();
        Ok(())
    }

    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        println!("close reason: {}", reason);
    }

    fn on_open(&mut self, shake: ws::Handshake) -> ws::Result<()> {
        println!("weboscket open from {}", shake.peer_addr.expect("handshake failed"));
        Ok(())
    }

    fn on_error(&mut self, err: ws::Error) {
        println!("error occuried: {}", err);
    }

    fn on_request(&mut self, req: &ws::Request) -> ws::Result<ws::Response> {
        println!("get request!!!");
        for i in req.protocols().unwrap().iter() {
            println!("{}", *i);
        }
        Response::from_request(req)        
    }

    fn on_shutdown(&mut self) {
        println!("shutdown");
    }

    // fn on_frame(&mut self, frame: ws::Frame) -> ws::Result<Option<ws::Frame>> {
    //     println!("Handler received: {}", frame);
    //     Ok(Some(frame))
    // }
}

fn main() {
    // Setup logging
    env_logger::init();


    let path = "/home/flash/project/ws-rs/examples/websocket.sock";
    if Path::new(path).exists() {
        std::fs::remove_file(path).unwrap();
    }

    // Listen on an address and call the closure for each connection
    if let Err(error) = listen_unix(path, |sender| {
        return Behavior { sender }; 
    }) {
        // Inform the user of failure
        println!("Failed to create WebSocket due to {:?}", error);
    }
}
