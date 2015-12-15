/// An example demonstrating how to send and recieve a custom ping/pong frame.
/// This example also shows using a separate thread to represent another
/// component of your system as well as how to use a custom Factory.
extern crate ws;
extern crate env_logger;
extern crate time;

use std::thread;
use std::thread::sleep;
use std::time::Duration;
use std::str::from_utf8;
use std::sync::mpsc::channel;
use std::sync::mpsc::Sender as ThreadOut;

use ws::{Builder, CloseCode, OpCode, Sender, Frame, Factory, Handler, Message, Result};

fn main () {

    // Setup logging
    env_logger::init().unwrap();

    // Set timer channel
    let (tx, rx) = channel();

    // Create WebSocket
    let socket = ws::Builder::new().build(ServerFactory { timer: tx }).unwrap();

    // Get broadcaster for timer
    let all = socket.broadcaster();

    // Start timer thread
    let timer = thread::spawn(move || {
        loop {
            // Test latency every 5 seconds
            sleep(Duration::from_secs(5));

            // Check if timer needs to go down
            if let Ok(_) = rx.try_recv() {
                println!("Timer is going down.");
                break;
            }

            // Ping all connections with current time
            if let Err(err) = all.ping(time::precise_time_ns().to_string().into()) {
                println!("Unable to ping connections: {:?}", err);
                println!("Timer is going down.");
                break;
            }
        }
    });

    // Start WebSocket server
    socket.listen("127.0.0.1:3012").unwrap();

    // Once the server is done, wait for the timer to shutdown as well
    timer.join().unwrap();
}

struct ServerFactory {
    timer: ThreadOut<usize>,
}

impl Factory for ServerFactory {
    type Handler = Server;

    fn connection_made(&mut self, out: Sender) -> Self::Handler {
        Server { out: out }
    }

    fn on_shutdown(&mut self) {
        if let Err(err) = self.timer.send(0) {
            println!("Unable to shut down timer: {:?}", err)
        }
    }

}

// For accessing the default handler implementation
struct DefaultHandler;

impl Handler for DefaultHandler {}

// Server WebSocket handler
struct Server {
    out: Sender,
}

impl Handler for Server {

    fn on_message(&mut self, msg: Message) -> Result<()> {
        println!("Server got message '{}'. ", msg);
        self.out.send(msg)
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {
        println!("WebSocket closing for ({:?}) {}", code, reason);
        println!("Shutting down server after first connection closes.");
        self.out.shutdown().unwrap();
    }

    fn on_frame(&mut self, frame: Frame) -> Result<Option<Frame>> {
        if frame.opcode() == OpCode::Pong {
            if let Ok(pong) = try!(from_utf8(frame.payload())).parse::<u64>() {
                let now = time::precise_time_ns();
                println!("Latency is {:.3}ms.", (now - pong) as f64 / 1_000_000f64);
            } else {
                println!("Received bad pong.");
            }
        }

        // Run default frame validation
        DefaultHandler.on_frame(frame)
    }
}

