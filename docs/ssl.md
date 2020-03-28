# SSL feature

WS-RS supports WebSocket connections using SSL (e.g. `wss://my/secure/websocket`). To enable the ssl feature, require WS-RS in your ```Cargo.toml``` with the feature listed:

```
[dependencies.ws]
version = "*"
features = ["ssl"]
```

With the ssl feature enabled, you can connect to an encrypted socket by using the ```wss``` scheme.

```rust
// An encypted Websocket echo client
extern crate ws;

fn main() {
  ws::connect("wss://localhost:3012", |out| {
      move |msg| {
          out.send(msg)
      }
  })
}
```

### [Home](index.md)