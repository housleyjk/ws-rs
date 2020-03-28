## Lightweight, event-driven WebSockets for Rust

#### [Repository](https://github.com/housleyjk/ws-rs)

### About
This library provides an implementation of WebSockets (RFC6455) in Rust. WS-RS uses [MIO](https://github.com/carllerche/mio) to leverage asynchronous IO to allow for handling multiple WebSocket connections on a single thread. WS-RS embraces the bidirectional nature of WebSockets allowing for both client and server connections to coexist as part of one WebSocket component. The WS-RS API aims to keep simple WebSocket applications simple and make advanced WebSocket programming possible in [Rust](https://rust-lang.org/) by abstracting away the menial parts of the WebSocket protocol while still providing enough low-level access to allow for custom extensions and subpotocols. For example, WS-RS supports the [permessage-deflate extension](deflate.md) as an optional feature. This library also supports SSL encrypted websockets using the [ssl feature](ssl.md). WS-RS is regularly tested and there are several [examples](examples.md) demonstrating various tasks that can be solved with WebSockets. Make sure to check out the [guide](guide.md) to help get started. The documentation is also available as a static page [here](https://docs.rs/ws/0.9.1/ws/).

### Comparison with other WebSocket libraries
You don't need to use this library to the exclusion of other WebSocket libraries, in Rust or other languages, but if you find yourself considering which library to use: choose the one that has the API you like the most. The API is generally more important than performance. The library API is what you will have to deal with as you build your WebSocket application. The design of WS-RS aims to provide a clean, consistent API. If you identify possible improvements please make a [feature request](https://github.com/housleyjk/ws-rs/issues).

However, if performance is what you value most and you want a WebSocket library in Rust please consider WS-RS. Here is how it stacks up against some other common frameworks using the example [benchmark tool](https://github.com/housleyjk/ws-rs/tree/stable/examples/bench.rs) to open 10,000 simultaneous connections and send 10 messages. These results are **not** reliable as a serious benchmark:

Library | Time 
:--- |:---
WS-RS | 1,709
libwebsockets | 2,067
rust-websocket | 8,950
\* websockets CPython 3.4.3 | 12,638
Autobahn CPython 2.7.10 | 48,902
\*\* NodeJS via ws | 127,635

\* websockets encountered a few (3) broken pipe errors  
\*\* NodeJS encountered several (229) connection timeout errors  
Your results will vary. The system specs for this test were as follows: Intel(R) Core(TM) i3 CPU 560 @ 3.33GHz, 8GB RAM