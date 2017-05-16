extern crate ws;

use std::net::Ipv4Addr;

struct Handler;
impl ws::Handler for Handler {}

#[test]
fn bind_port_zero() {
    let ws = ws::WebSocket::new(|_sender| Handler).unwrap();
    let ws = ws.bind("127.0.0.1:0").unwrap();

    let local_addr = ws.local_addr().unwrap();
    println!("Listening on {}", local_addr);

    assert_eq!(Ipv4Addr::new(127, 0, 0, 1), local_addr.ip());
    assert_ne!(0, local_addr.port());
}
