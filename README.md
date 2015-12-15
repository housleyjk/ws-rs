# WS-RS

Lightweight, event-driven WebSockets for [Rust](http://www.rust-lang.org).
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

This library provides an implementation of WebSockets, [RFC6455](https://tools.ietf.org/html/rfc6455) using [MIO](https://github.com/carllerche/mio).
It allows for handling multiple connections on a single thread, and even spawning new client connections on the same thread. This makes for very fast
and resource efficient WebSockets. The API design abstracts away the menial parts of the WebSocket protocol, masking and unmasking frames,
which allows you to focus on application code and rely on WS-RS to handle protocol conformance. However, it is also possible to get low-level
access to individual WebSocket frames if you need to write extensions or want to optimize around the WebSocket protocol.

**[API documentation](http://housleyjk.github.io/ws-rs/ws)**


Examples
--------
* [Server](https://github.com/housleyjk/ws-rs/tree/stable/examples/server.rs).
A simple WebSocket echo server using closures.
* [Client](https://github.com/housleyjk/ws-rs/tree/stable/examples/client.rs)
A simple WebSocket client for connecting to an echo server using closures.
* [Single Threaded](https://github.com/housleyjk/ws-rs/tree/stable/examples/shared.rs)
An example of an echo client and an echo server on one thread using closures.
* [Threads](https://github.com/housleyjk/ws-rs/tree/stable/examples/threaded.rs)
An example of an echo client and an echo server on separate threads. This demonstrates using a struct as a WebSocket handler.
* [Channels](https://github.com/housleyjk/ws-rs/tree/stable/examples/channel.rs)
A more complex example using channels to communicate with a WebSocket handler to accomplish a separate task.
* [Pong](https://github.com/housleyjk/ws-rs/tree/stable/examples/pong.rs)
An example demonstrating how to send and recieve a custom ping/pong frame.

Stability and Testing
---------------------

WS-RS passes both the client and server sides of the [Autobahn Testsuite](http://autobahn.ws/testsuite/).
To run the tests yourself, install the [Autobahn Testsuite](http://autobahn.ws/testsuite/).
For example:

```
mkvirtualenv wstest --python=/usr/bin/python2
pip install autobahntestsuite
```

To run the client tests, start the test server in one window:
```
cd tests
wstest -m fuzzingserver
```
And run the WS-RS client in the other:
```
cargo run --example autobahn-client
```

To run the server tests, start the WS-RS server in one window:
```
cargo run --example autobahn-server
```
And run the test client in the other:
```
cd tests
wstest -m fuzzingclient
```

SSL
---
WS-RS supports WebSocket connections using SSL (e.g. `wss://mysecure/websocket`). To enable the ssl feature, require WS-RS in your `Cargo.toml`
file as so:

``` TOML
[dependencies.ws]
version = "*"
features = ["ssl"]
```

Then simply specify the `wss` scheme to use an encypted connection.

```rust
/// An encypted Websocket echo client
connect("wss://localhost:3012", |ws| {
    move |msg| {
        ws.send(msg)
    }
})
```

Note: The ssl feature is currently not available on Windows.


Contributing
------------

Please report bugs and make feature requests [here](https://github.com/housleyjk/ws-rs/issues). I welcome pull requests.
Unit tests for bugs are greatly appreciated.

Comparison with other WebSocket libraries
-----------------------------------------

You don't need to use this library to the exclusion of other WebSocket libraries, in Rust or other languages,
but if you find yourself considering which library to use, here is my opinion: choose the one that has the API
you like the most. The API is generally more important than performance. The library API is what you will have to deal with as
you build your WebSocket application. I've written WS-RS to have an API that fits my needs. If you don't like it,
please make a [feature request](https://github.com/housleyjk/ws-rs/issues) to improve it or use another library
that causes you less problems.

However, if performance is what you value most and you want a WebSocket library in Rust, I would choose this one.
Here is how it stacks up against some other common frameworks using the example [benchmark tool](https://github.com/housleyjk/ws-rs/tree/stable/examples/bench.rs)
to open 10,000 simultaneous connections and send 10 messages. These results are **not** reliable as a serious benchmark:

Library | Time (ms)
--------| ---------
<a href="https://github.com/housleyjk/ws-rs">WS-RS</a> | 1,709
<a href="https://libwebsockets.org/trac/libwebsockets">libwebsockets</a> | 2,067
<a href="https://github.com/cyderize/rust-websocket">rust-websocket</a> | 8,950
<a href="http://aaugustin.github.io/websockets/">\* websockets CPython 3.4.3</a> | 12,638
<a href="http://autobahn.ws/python/">Autobahn CPython 2.7.10</a> | 48,902
<a href="https://github.com/websockets/ws">\*\* NodeJS via ws</a> | 127,635

\* websockets encountered a few (3) broken pipe errors<br>
\*\* NodeJS encountered several (229) connection timeout errors

Your results will vary. The system specs for this test were as follows:

Intel(R) Core(TM) i3 CPU 560 @ 3.33GHz, 8GB RAM
