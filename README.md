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

This library provides an implementation of WebSockets, [RFC6455](https://tools.ietf.org/html/rfc6455) using [MIO](https://github.com/carllerche/mio).
To use this library, create a WebSocket struct using one of the helper functions `listen` and `connect`.

A WebSocket requires two basic components: a Factory and a Handler. Your Factory will be called each time the underlying TCP connection has
been successfully established, and it will need to return a Handler that will handle the new WebSocket connection. Factories can be used to
manage state that applies to multiple WebSocket connections, whereas Handlers manage the state of individual connections.

Your Factory will be passed a Sender struct that represents the output of the WebSocket. The Sender allows the Handler to
send messages, initiate a WebSocket closing handshake by sending a close code, and other useful actions. If you need to send messages from other parts
of your application it is possible to clone and send the Sender across threads allowing other code to send messages on the WebSocket without blocking
the event loop.

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


Stability and Testing
---------------------

WS-RS is currently a work in progress. However, it already passes both the client and server sides of the [Autobahn Testsuite](http://autobahn.ws/testsuite/).
In the future I hope to make the same stability guarantees as the [Rust Standard Library](http://www.rust-lang.org) in so far as dependencies will allow.
I am commited to avoiding excessive allocations and costly abstractions.

Contributing
------------

Please report bugs and make feature requests [here](https://github.com/housleyjk/ws-rs/issues). I welcome pull requests.
More tests are appreciated.
