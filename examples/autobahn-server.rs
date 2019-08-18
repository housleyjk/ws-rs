//! WebSocket server used for testing against the Autobahn Test Suite. This is basically the server
//! example without printing output or comments.

use env_logger;
use ws;
#[cfg(feature = "permessage-deflate")]
use ws::deflate::DeflateHandler;

#[cfg(not(feature = "permessage-deflate"))]
fn main() {
    env_logger::init();

    ws::listen("127.0.0.1:3012", |out| move |msg| out.send(msg)).unwrap()
}

#[cfg(feature = "permessage-deflate")]
fn main() {
    env_logger::init();

    ws::listen("127.0.0.1:3012", |out| {
        DeflateHandler::new(move |msg| out.send(msg))
    })
    .unwrap();
}
