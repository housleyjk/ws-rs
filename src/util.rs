//! The util module rexports some tools from mio in order to facilitate handling timeouts.

#[cfg(any(feature = "ssl", feature = "nativetls"))]
/// TcpStream underlying the WebSocket
pub use mio::tcp::TcpStream;
/// Used to identify some timed-out event.
pub use mio::Token;
/// A handle to a specific timeout.
pub use mio_extras::timer::Timeout;
