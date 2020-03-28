# Permessage-Deflate extension

WS-RS supports the [permessage-deflate extension](https://tools.ietf.org/html/rfc7692) which allows for the compression of WebSocket messages. To enable the feature, specify it in your ```Cargo.toml```:

```
[dependencies.ws]
version = "*"
features = ["permessage-deflate"]
```

Once the feature is enabled, you will need to wrap your message handler inside of a ```DeflateHandler```, which will negotiate the extension with the other endpoint and perform the compression and decompression of messages.

```rust
// An echo server that compresses and decompresses messages using the deflate algorithm
extern crate ws;

use ws::deflate::DeflateHandler;

fn main() {
  ws::listen("127.0.0.1:3012", |out| {
      DeflateHandler::new(move |msg| {
          out.send(msg)
      })
  }).expect("Failed to build WebSocket");
}
```
The ```DeflateHandler``` will accept any other valid handler. In other words, any struct that implements the ```Handler``` trait. If you would like to configure the extension, for example if you wanted to limit the size of the sliding window, use the ```DeflateBuilder``` struct and pass in the settings.

```rust
// A WebSocket client that sends a message to an echo server using the permessage-deflate
// extension with a sliding window of 10 bits.
extern crate ws;

use ws::deflate::{DeflateBuilder, DeflateSettings};

fn main() {
  ws::connect("ws://127.0.0.1:3012", |out| {
    DeflateBuilder::new().with_settings(DeflateSettings {
        max_window_bits: 10,
        ..Default::default()
      }).build(Client {
        out: out,
      })
  }).expect("Failed to build WebSocket");
}

struct Client {
  out: ws::Sender,
}

impl ws::Handler for Client {
  fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
    self.out.send("This is the message.
      It will be compressed by the client and sent to the server, which will decompress it
      and send it back (recompressing it) for the client to then decompress and print.")
  }

  fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
    println!("{}", msg);
    self.out.clode(ws::CloseCode::Normal)
  }
}
```


### [Home](index.md)