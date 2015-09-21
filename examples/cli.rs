extern crate clap;
extern crate term;

use std::io;
use std::io::prelude::*;
use std::sync::mpsc::Sender as TSender;
use std::sync::mpsc::channel;
use std::thread::{spawn, JoinHandle};

use clap::{App, Arg};


fn main() {
    let matches = App::new("WS Command Line Client")
        .version("1.0")
        .author("Jason Housley <housleyjk@gmail.com>")
        .about("Connect to a WebSocket and send messages from the command line.")
        .arg(Arg::with_name("URL")
             .help("The URL of the WebSocket server.")
             .required(true)
             .index(1)).get_matches();

}
