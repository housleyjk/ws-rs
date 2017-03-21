use std::io;
use std::io::ErrorKind::WouldBlock;
use std::net::SocketAddr;

use mio::tcp::TcpStream;
#[cfg(feature="ssl")]
use openssl::ssl::SslStream;
#[cfg(feature="ssl")]
use openssl::ssl::Error as SslError;
use bytes::{Buf, MutBuf};

use result::{Result, Error, Kind};

fn map_non_block<T>(res: io::Result<T>) -> io::Result<Option<T>> {
    match res {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            if let WouldBlock = err.kind() {
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

pub trait TryReadBuf: io::Read {
    fn try_read_buf<B: MutBuf>(&mut self, buf: &mut B) -> io::Result<Option<usize>>
        where Self : Sized
    {
        // Reads the length of the slice supplied by buf.mut_bytes into the buffer
        // This is not guaranteed to consume an entire datagram or segment.
        // If your protocol is msg based (instead of continuous stream) you should
        // ensure that your buffer is large enough to hold an entire segment (1532 bytes if not jumbo
        // frames)
        let res = map_non_block(self.read(unsafe { buf.mut_bytes() }));

        if let Ok(Some(cnt)) = res {
            unsafe { buf.advance(cnt); }
        }

        res
    }
}

pub trait TryWriteBuf: io::Write {
    fn try_write_buf<B: Buf>(&mut self, buf: &mut B) -> io::Result<Option<usize>>
        where Self : Sized
    {
        let res = map_non_block(self.write(buf.bytes()));

        if let Ok(Some(cnt)) = res {
            buf.advance(cnt);
        }

        res
    }
}

impl<T: io::Read> TryReadBuf for T {}
impl<T: io::Write> TryWriteBuf for T {}

use self::Stream::*;
pub enum Stream {
    Tcp(TcpStream),
    #[cfg(feature="ssl")]
    Tls {
        sock: SslStream<TcpStream>,
        negotiating: bool,
    }
}


impl Stream {

    pub fn tcp(stream: TcpStream) -> Stream {
        Tcp(stream)
    }

    #[cfg(feature="ssl")]
    pub fn tls(stream: SslStream<TcpStream>) -> Stream {
        Tls { sock: stream, negotiating: false }
    }

    #[cfg(feature="ssl")]
    pub fn is_tls(&self) -> bool {
        match *self {
            Tcp(_) => false,
            Tls {..} => true,
        }
    }

    pub fn evented(&self) -> &TcpStream {
        match *self {
            Tcp(ref sock) => sock,
            #[cfg(feature="ssl")]
            Tls { ref sock, ..} => sock.get_ref(),
        }
    }

    pub fn is_negotiating(&self) -> bool {
        match *self {
            Tcp(_) => false,
            #[cfg(feature="ssl")]
            Tls { sock: _, ref negotiating } => *negotiating,
        }

    }

    pub fn clear_negotiating(&mut self) -> Result<()> {
        match *self {
            Tcp(_) => Err(Error::new(Kind::Internal, "Attempted to clear negotiating flag on non ssl connection.")),
            #[cfg(feature="ssl")]
            Tls { sock: _, ref mut negotiating } => Ok(*negotiating = false),
        }
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        match *self {
            Tcp(ref sock) => sock.peer_addr(),
            #[cfg(feature="ssl")]
            Tls { ref sock, ..} => sock.get_ref().peer_addr(),
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        match *self {
            Tcp(ref sock) => sock.local_addr(),
            #[cfg(feature="ssl")]
            Tls { ref sock, ..} => sock.get_ref().local_addr(),
        }
    }
}

impl io::Read for Stream {

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            Tcp(ref mut sock) => sock.read(buf),
            #[cfg(feature="ssl")]
            Tls { ref mut sock, ref mut negotiating } => {
                match sock.ssl_read(buf) {
                    Ok(cnt) => Ok(cnt),
                    Err(SslError::WantWrite(_)) => {
                        *negotiating = true;
                        Err(io::Error::new(io::ErrorKind::WouldBlock, "SSL wants writing"))
                    },
                    Err(SslError::WantRead(_)) => Err(io::Error::new(io::ErrorKind::WouldBlock, "SSL wants reading")),
                    Err(err) =>
                        Err(io::Error::new(io::ErrorKind::Other, err)),
                }
            }
        }
    }
}

impl io::Write for Stream {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Tcp(ref mut sock) => sock.write(buf),
            #[cfg(feature="ssl")]
            Tls { ref mut sock, ref mut negotiating } => {

                *negotiating = false;

                match sock.ssl_write(buf) {
                    Ok(cnt) => Ok(cnt),
                    Err(SslError::WantRead(_)) => {
                        *negotiating = true;
                        Err(io::Error::new(io::ErrorKind::WouldBlock, "SSL wants reading"))
                    },
                    Err(SslError::WantWrite(_)) => Err(io::Error::new(io::ErrorKind::WouldBlock, "SSL wants writing")),
                    Err(err) =>
                        Err(io::Error::new(io::ErrorKind::Other, err)),
                }
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Tcp(ref mut sock) => sock.flush(),
            #[cfg(feature="ssl")]
            Tls { ref mut sock, negotiating: _ } => sock.flush(),
        }
    }
}
