extern crate ws;

use std::thread;
use std::time::Duration;

struct Server {
    out: ws::Sender,
}

impl ws::Handler for Server {

    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        self.out.send("Die")
    }

    fn on_close(&mut self, close: ws::CloseCode, _: &str) {
        assert_eq!(close, ws::CloseCode::Abnormal);
        self.out.shutdown().unwrap();
    }

    fn on_error(&mut self, err: ws::Error) {
        panic!("{}", err)
    }

}

#[test]
fn hang_up() {

    let t = thread::spawn(move || {
        ws::listen("localhost:3012", move |sender| {
            Server { out: sender }
        }).unwrap()
    });

    thread::sleep(Duration::from_millis(500));

    let t2 = thread::spawn(move || {
        ws::connect("ws://localhost:3012", move |_| {
            move |_| {
                panic!("Should panic to simulate abnormal closure.")
            }
        }).unwrap();
    });
    assert!(t2.join().is_err());
    assert!(t.join().is_ok());

}
