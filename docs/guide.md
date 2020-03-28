# Guide

### [Home](index.md)

### Installation

To start using WS-RS simply add it to your ```Cargo.toml``` file.

```
[dependencies]
ws = "*"
```
Using ```"*"``` will give you the latest stable version. If you want the development version, link to the master branch of the WS-RS respository.

```
  [dependencies]
  ws = { version = "*", git = "https://github.com/housleyjk/ws-rs"}
```

### Usage
For simple applications, use one of the utility functions ```listen``` and ```connect```:

```listen``` accepts a string representation of a socket address and a ```Factory```.

```rust
// A WebSocket echo server

extern crate ws;

use ws::listen;

fn main() {
  listen("127.0.0.1:3012", |out| {
      move |msg| {
         out.send(msg)
      }
  }).unwrap()
}
```

```connect``` accepts a string that represents a WebSocket URL (i.e. one that starts with ws:// or wss://), and it will attempt to connect to a WebSocket server at that location. It also accepts a ```Factory```.

```rust
// A WebSocket client that sends one message then closes
extern crate ws;

use ws::{connect, CloseCode};

fn main() {
  connect("ws://127.0.0.1:3012", |out| {
      out.send("Hello WebSocket").unwrap();

      move |msg| {
          println!("Got message: {}", msg);
          out.close(CloseCode::Normal)
      }
  }).unwrap()
}
```

Each of these functions encapsulates a mio EventLoop, creating and running a WebSocket in the current thread. These are blocking functions, so they will only return after the encapsulated WebSocket has been shutdown.

## Architecture

A WebSocket requires two basic components: a Factory and a Handler. A Factory is any struct that implements the ```Factory``` trait. WS-RS already provides an implementation of ```Factory``` for closures that take a ```Sender``` as the first argument, so it is possible to pass a closure as a Factory to either of the utility functions. Your Factory will be called each time the underlying TCP connection has been successfully established, and it will need to return a Handler that will handle the new WebSocket connection.

Factories can be used to manage state that applies to multiple WebSocket connections, whereas Handlers manage the state of individual connections. Most of the time, a closure Factory is sufficient, and you will only need to focus on writing your Handler. Your Factory will be passed a ```Sender``` struct that represents the output of the WebSocket. The Sender allows the Handler to send messages, initiate a WebSocket closing handshake by sending a close code, and other useful actions. If you need to send messages from other parts of your application it is possible to clone and send the Sender across threads allowing other code to send messages on the WebSocket without blocking the event loop.

Just as with the Factory, it is possible to use a closure as a simple Handler. The closure must take a ```Message``` as it's only argument, and it may close over variables that exist in the Factory. For example, in getting started examples with ```listen``` and ```connect```, the closure Factory returns another closure as the Handler for the new connection. This handler closure closes over the variable ```out```, which is the Sender, representing the output of the WebSocket, so that it can use that sender later to send a Message. Closure Handlers generally need to take ownership of the variables that they close over because the Factory may be called multiple times. Think of Handlers as though they were running on separate threads and they should make sense within Rust's memory model. Closure Handlers must return a ```Result<()>```, in order to handle errors without panicking.

Sender methods, such as ```close``` and ```send``` both actually return a ```Result<()>``` indicating whether they were able to schedule the requested command (either ```close``` or ```send```) with the EventLoop.

**It is important that your Handler does not panic carelessly because a handler that panics will disconnect every other connection that is using that WebSocket. Don't panic unless you want all connections to immediately fail.**

## Implementing a header

You may have noticed in the usage examples that the client example calls ```unwrap``` when sending the first message, which will panic in the factory if the Message can't be sent for some reason. Also, sending messages before a handler is returned means that the message will be queued before the WebSocket handshake is complete. The handshake could fail for some reason, and then the queued message would be wasted effort. Sending messages in the Factory is not bad for simple, short-lived, or toy projects, but let's explore writing a handler that is better for long-running applications. In order to solve the problem of sending a message immediately when a WebSocket connection is established, you will need to write a Handler that implements the ```on_open``` method. For example:

```rust
extern crate ws;

use ws::{connect, Handler, Sender, Handshake, Result, Message, CloseCode};

// Our Handler struct.
// Here we explicity indicate that the Client needs a Sender,
// whereas a closure captures the Sender for us automatically.
struct Client {
    out: Sender,
}

// We implement the Handler trait for Client so that we can get more
// fine-grained control of the connection.
impl Handler for Client {

    // `on_open` will be called only after the WebSocket handshake is successful
    // so at this point we know that the connection is ready to send/receive messages.
    // We ignore the `Handshake` for now, but you could also use this method to setup
    // Handler state or reject the connection based on the details of the Request
    // or Response, such as by checking cookies or Auth headers.
    fn on_open(&mut self, _: Handshake) -> Result<()> {
        // Now we don't need to call unwrap since `on_open` returns a `Result<()>`.
        // If this call fails, it will only result in this connection disconnecting.
        self.out.send("Hello WebSocket")
    }

    // `on_message` is roughly equivalent to the Handler closure. It takes a `Message`
    // and returns a `Result<()>`.
    fn on_message(&mut self, msg: Message) -> Result<()> {
        // Close the connection when we get a response from the server
        println!("Got message: {}", msg);
        self.out.close(CloseCode::Normal)
    }
}

fn main() {
  // Now, instead of a closure, the Factory returns a new instance of our Handler.
  connect("ws://127.0.0.1:3012", |out| Client { out: out } ).unwrap()
}
```

That is a big increase in verbosity in order to accomplish the same effect as the original example, but this way is more flexible and gives you access to more of the underlying details of the WebSocket connection.

It's also important to note that using ```on_open``` allows you to tie in to the lifecycle of the WebSocket. If the opening handshake is successful and ```on_open``` is called, the WebSocket is now open and alive. Until that point, it is not guaranteed that the connection will be upgraded. So, if you have important state that you need to tear down, or if you have some state that tracks closely the lifecycle of the WebScoket connection, it is best to set that up in the ```on_open``` method rather than when your handler is first created. If ```on_open``` returns Ok, then you are guaranteed that ```on_close``` will run when the WebSocket connection is about to go down, unless a panic has occurred.

Therefore you will probably want to implement ```on_close```. This method is called anytime the WebSocket connection will close. The ```on_close``` method implements the closing handshake of the WebSocket protocol. Using ```on_close``` gives you a mechanism for informing the user regarding why the WebSocket connection may have been closed even if no errors were encountered. It also gives you an opportunity to clean up any resources or state that may be dependent on the connection that is now about to disconnect. An example server might use this as follows: 

```rust
extern crate ws;

use ws::{listen, Handler, Sender, Result, Message, CloseCode};

struct Server {
    out: Sender,
}

impl Handler for Server {

    fn on_message(&mut self, msg: Message) -> Result<()> {
        // Echo the message back
        self.out.send(msg)
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {
        // The WebSocket protocol allows for a utf8 reason for the closing state after the
        // close code. WS-RS will attempt to interpret this data as a utf8 description of the
        // reason for closing the connection. I many cases, `reason` will be an empty string.
        // So, you may not normally want to display `reason` to the user,
        // but let's assume that we know that `reason` is human-readable.
        match code {
            CloseCode::Normal => println!("The client is done with the connection."),
            CloseCode::Away   => println!("The client is leaving the site."),
            _ => println!("The client encountered an error: {}", reason),
        }
    }
}

fn main() {
  listen("127.0.0.1:3012", |out| Server { out: out } ).unwrap()
}
```

When errors occur, your handler will be informed via the ```on_error``` method. Depending on the type of the error, the connection may or may not be about to go down. If the error is such that the connection needs to close, your handler's ```on_close``` method will be called and WS-RS will send the appropriate close code to the other endpoint if possible. A server that tracks state related to the life of the WebSocket connection and informs the user of errors might be as follows:

```rust
extern crate ws;

use std::rc::Rc;
use std::cell::Cell;

use ws::{listen, Handler, Sender, Result, Message, Handshake, CloseCode, Error};

struct Server {
    out: Sender,
    count: Rc<Cell<u32>>,
}

impl Handler for Server {

    fn on_open(&mut self, _: Handshake) -> Result<()> {
        // We have a new connection, so we increment the connection counter
        Ok(self.count.set(self.count.get() + 1))
    }

    fn on_message(&mut self, msg: Message) -> Result<()> {
        // Tell the user the current count
        println!("The number of live connections is {}", self.count.get());

        // Echo the message back
        self.out.send(msg)
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {
        match code {
            CloseCode::Normal => println!("The client is done with the connection."),
            CloseCode::Away   => println!("The client is leaving the site."),
            CloseCode::Abnormal => println!(
                "Closing handshake failed! Unable to obtain closing status from client."),
            _ => println!("The client encountered an error: {}", reason),
        }

        // The connection is going down, so we need to decrement the count
        self.count.set(self.count.get() - 1)
    }

    fn on_error(&mut self, err: Error) {
        println!("The server encountered an error: {:?}", err);
    }

}

fn main() {
  // Cell gives us interior mutability so we can increment
  // or decrement the count between handlers.
  // Rc is a reference-counted box for sharing the count between handlers
  // since each handler needs to own its contents.
  let count = Rc::new(Cell::new(0));
  listen("127.0.0.1:3012", |out| { Server { out: out, count: count.clone() } }).unwrap()
}
```

There are other Handler methods that allow even more fine-grained access, but most applications will usually only need these four methods.

### [Home](index.md)