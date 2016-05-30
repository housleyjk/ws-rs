extern crate ws;

struct Server {
    ws: ws::Sender,
}

impl ws::Handler for Server {

    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        println!("Remote addres {:?}", self.ws.remote_addr() );
        Ok(())
    }

    fn on_message(&mut self, _: ws::Message) -> ws::Result<()> {
        self.ws.close(ws::CloseCode::Normal)
    }
}

fn main () {
    ws::listen("127.0.0.1:3012", |out| {
        Server { ws: out }
    }).unwrap()
}
