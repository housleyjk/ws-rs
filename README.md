# WS-RS

Lightweight, event-driven WebSockets for [Rust](https://www.rust-lang.org).
```rust

/// A WebSocket echo server
listen("127.0.0.1:3012", |out| {
    move |msg| {
        out.send(msg)
    }
})
```

Introduction
------------
[![Build Status](https://travis-ci.org/housleyjk/ws-rs.svg?branch=stable)](https://travis-ci.org/housleyjk/ws-rs)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Crate](http://meritbadge.herokuapp.com/ws)](https://crates.io/crates/ws)

**[Homepage](https://github.com/housleyjk/ws-rs/)**

**[API Documentation](https://docs.rs/ws/latest/ws/index.html)**

This library provides an implementation of WebSockets,
[RFC6455](https://tools.ietf.org/html/rfc6455) using [MIO](https://github.com/carllerche/mio). It
allows for handling multiple connections on a single thread, and even spawning new client
connections on the same thread. This makes for very fast and resource efficient WebSockets. The API
design abstracts away the menial parts of the WebSocket protocol and allows you to focus on
application code without worrying about protocol conformance. However, it is also possible to get
low-level access to individual WebSocket frames if you need to write extensions or want to optimize
around the WebSocket protocol.

Getting Started
---------------

Check out the [examples](https://github.com/housleyjk/ws-rs/blob/master/examples/server.rs).


Features
--------

WS-RS provides a complete implementation of the WebSocket specification. There is also support for
[ssl](https://github.com/housleyjk/ws-rs/blob/master/examples/ssl-server.rs) and
[permessage-deflate](https://github.com/housleyjk/ws-rs/blob/master/examples/autobahn-server.rs).

Contributing
------------

Please report bugs and make feature requests [here](https://github.com/housleyjk/ws-rs/issues).
