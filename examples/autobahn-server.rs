/// WebSocket server used for testing against the Autobahn Test Suite. This is basically the server
/// example without printing output or comments.

extern crate ws;

fn main () {
    ws::listen("127.0.0.1:3012", |out| {
        move |msg| {
            out.send(msg)
        }
    }).unwrap()
}
