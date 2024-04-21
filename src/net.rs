use std::io;
use crate::reactor::Registration;

// Re-exports.
pub use std::net::ToSocketAddrs;

/// A TCP stream.
pub struct TcpStream {
    strm:   std::net::TcpStream,
    regfd:  Registration,
}

impl TcpStream {
    pub fn from_std(stream: std::net::TcpStream) -> io::Result<TcpStream> {
        use std::os::fd::AsRawFd;
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(TcpStream {
            strm: stream,
            regfd: Registration::new(fd),
        })
    }

    pub async fn connect<A: ToSocketAddrs + Send + 'static>(addr: A) -> io::Result<TcpStream> {
        let stream = crate::task::spawn_blocking(move || {
            std::net::TcpStream::connect(addr)
        }).await.unwrap()?;
        Self::from_std(stream)
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.strm.shutdown(std::net::Shutdown::Write)
    }
}

crate::io::impl_async_read!(TcpStream, strm, regfd);
crate::io::impl_async_write!(TcpStream, strm, regfd, shutdown);
