/// Simple WebSocket client with error handling. It is not necessary to setup logging, but doing
/// so will allow you to see more details about the connection by using the RUST_LOG env variable.

extern crate url;
extern crate ws;
extern crate env_logger;

use ws::{connect, CloseCode};

fn main () {

    // Setup logging
    env_logger::init().unwrap();

    // Parse str into a url
    let url = url::Url::parse("ws://127.0.0.1:3012").unwrap();

    // Connect to the url and call the closure
    if let Err(error) = connect(url, |out| {

        // Queue a message to be sent when the websocket is open
        if let Err(_) = out.send("Hello WebSocket") {
            println!("Websocket couldn't queue an initial message.")
        } else {
            println!("Client sent message 'Hello WebSocket'. ")
        }

        // The handler needs to take ownership of out, so we use move
        move |msg| {

            // Handle messages received on this connection
            println!("Client got message '{}'. ", msg);

            // Close the connection
            out.close(CloseCode::Normal)
        }

    }) {
        // Inform the user of any error
        println!("WebSocket failed with {:?}", error);
    }

}
