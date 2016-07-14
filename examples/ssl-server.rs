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
use std::rc::Rc;
#[cfg(feature="ssl")]
use openssl::ssl::{Ssl, SslContext, SslMethod};
#[cfg(feature="ssl")]
use openssl::x509::X509FileType;

#[cfg(feature="ssl")]
struct Server {
    out: ws::Sender,
    ssl: Rc<SslContext>,
}

#[cfg(feature="ssl")]
impl ws::Handler for Server {

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        self.out.send(msg) // simple echo
    }

    fn build_ssl(&mut self) -> ws::Result<Ssl> {
        Ssl::new(&self.ssl).map_err(ws::Error::from)
    }
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

    let mut context = SslContext::new(SslMethod::Tlsv1).unwrap();
    context.set_certificate_file(matches.value_of("CERT").unwrap(), X509FileType::PEM).unwrap();
    context.set_private_key_file(matches.value_of("KEY").unwrap(), X509FileType::PEM).unwrap();

    let context_rc = Rc::new(context);

    ws::Builder::new().with_settings(ws::Settings {
        encrypt_server: true,
        ..ws::Settings::default()
    }).build(|out: ws::Sender| {
        Server {
            out: out,
            ssl: context_rc.clone(),
        }
    }).unwrap().listen(matches.value_of("ADDR").unwrap()).unwrap();
}

#[cfg(not(feature="ssl"))]
fn main() {}
