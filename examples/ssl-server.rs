/// WebSocket server to demonstrate ssl encryption within an a websocket server.
///
/// The resulting executable takes three arguments:
///   ADDR - The address to listen for incoming connections (e.g. 127.0.0:3012)
///   CERT - The path to the cert PEM (e.g. snakeoil.crt)
///   KEY - The path to the key PEM (e.g. snakeoil.key)
///
/// For more details concerning setting up the SSL context, see rust-openssl docs.
extern crate ws;
extern crate clap;
#[cfg(feature="ssl")]
extern crate openssl;
extern crate env_logger;

#[cfg(feature="ssl")]
use std::io::Read;
#[cfg(feature="ssl")]
use std::fs::File;
#[cfg(feature="ssl")]
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder, SslMethod};
#[cfg(feature="ssl")]
use openssl::pkey::PKey;
#[cfg(feature="ssl")]
use openssl::x509::{X509, X509Ref};

#[cfg(feature="ssl")]
struct Server {
    out: ws::Sender,
    ssl: SslAcceptor,
}

#[cfg(feature="ssl")]
impl ws::Handler for Server {

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        self.out.send(msg) // simple echo
    }

    fn build_ssl_server(&mut self) -> ws::Result<SslAcceptor> {
        Ok(self.ssl.clone())
    }
}

#[cfg(feature="ssl")]
fn read_file(name: &str) -> std::io::Result<Vec<u8>> {
    let mut file = try!(File::open(name));
    let mut buf = Vec::new();
    try!(file.read_to_end(&mut buf));
    Ok(buf)
}

#[cfg(feature="ssl")]
fn main () {
    // Setup logging
    env_logger::init().unwrap();

    // setup command line arguments
    let matches = clap::App::new("WS-RS SSL Server Configuration")
        .version("1.0")
        .author("Jason Housley <housleyjk@gmail.com>")
        .about("Establish a WebSocket server that encrypts and decrypts messages.")
        .arg(clap::Arg::with_name("ADDR")
             .help("Address on which to bind the server.")
             .required(true)
             .index(1))
        .arg(clap::Arg::with_name("CERT")
             .help("Path to the SSL certificate.")
             .required(true)
             .index(2))
        .arg(clap::Arg::with_name("KEY")
             .help("Path to the SSL certificate key.")
             .required(true)
             .index(3))
        .get_matches();

    let cert = {
        let data = read_file(matches.value_of("CERT").unwrap()).unwrap();
        X509::from_pem(data.as_ref()).unwrap()
    };

    let pkey = {
        let data = read_file(matches.value_of("KEY").unwrap()).unwrap();
        PKey::private_key_from_pem(data.as_ref()).unwrap()
    };

    let context = SslAcceptorBuilder::mozilla_intermediate(SslMethod::tls(),
        &pkey, &cert, std::iter::empty::<X509Ref>()).unwrap().build();

    ws::Builder::new().with_settings(ws::Settings {
        encrypt_server: true,
        ..ws::Settings::default()
    }).build(|out: ws::Sender| {
        Server {
            out: out,
            ssl: context.clone(),
        }
    }).unwrap().listen(matches.value_of("ADDR").unwrap()).unwrap();
}

#[cfg(not(feature="ssl"))]
fn main() {}
