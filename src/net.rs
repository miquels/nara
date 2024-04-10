use std::io;

pub use std::net::ToSocketAddrs;

use crate::reactor::{self, RegisteredFd};


pub struct TcpStream {
    strm:   std::net::TcpStream,
    regfd:  RegisteredFd,
}

impl TcpStream {
    pub fn from_std(stream: std::net::TcpStream) -> io::Result<TcpStream> {
        use std::os::fd::AsRawFd;
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(TcpStream {
            strm: stream,
            regfd: RegisteredFd::new(fd),
        })
    }

    pub async fn connect<A: ToSocketAddrs + Send + 'static>(addr: A) -> io::Result<TcpStream> {
        let stream = crate::task::spawn_blocking(move || {
            std::net::TcpStream::connect(addr)
        }).await.unwrap()?;
        Self::from_std(stream)
    }
}

reactor::impl_async_read!(TcpStream, strm, regfd);
reactor::impl_async_write!(TcpStream, strm, regfd, shutdown(std::net::Shutdown::Write));

