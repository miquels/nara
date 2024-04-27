use std::io;
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::str::FromStr;

use socket2::{Socket, Domain, Type};
use crate::reactor::Registration;

//
// First, an async ToSocketAddrs trait, plus a bunch of implementations.
//
pub trait ToSocketAddrs {
    #[allow(async_fn_in_trait)]
    async fn to_socket_addrs(&self) -> io::Result<Vec<SocketAddr>>;
}

// Helper for DNS lookups.
async fn to_socket_addrs<A>(addr: A) -> io::Result<Vec<SocketAddr>>
where
    A: std::net::ToSocketAddrs + Send + 'static ,
{
    crate::task::spawn_blocking(move || {
        let a = std::net::ToSocketAddrs::to_socket_addrs(&addr)?.collect::<Vec<_>>();
        Ok::<_, io::Error>(a)
    }).await.unwrap()
}

impl ToSocketAddrs for str {
    async fn to_socket_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        // First try to parse as literal IP address and port.
        if let Ok(addr) = SocketAddr::from_str(self) {
            return Ok(vec![ addr ]);
        }
        // Contains a hostname, so do DNS lookup.
        to_socket_addrs(self.to_string()).await
    }
}

impl ToSocketAddrs for (&str, u16) {
    async fn to_socket_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        if let Ok(ip) = std::net::IpAddr::from_str(self.0) {
            return Ok(vec![SocketAddr::new(ip, self.1)]);
        }
        to_socket_addrs((self.0.to_string(), self.1)).await
    }
}

// implementation for slices.
impl<'a> ToSocketAddrs for &'a [std::net::SocketAddr] {
    async fn to_socket_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        Ok(self.to_vec())
    }
}

// factory macro for the rest of the types.
macro_rules! to_socket_addrs_impl {
    ($type:ty) => {
        impl $crate::net::ToSocketAddrs for $type {
            async fn to_socket_addrs(&self) -> std::io::Result<Vec<std::net::SocketAddr>> {
                Ok(std::net::ToSocketAddrs::to_socket_addrs(self)?.collect::<Vec<_>>())
            }
        }
    }
}
to_socket_addrs_impl!((std::net::IpAddr, u16));
to_socket_addrs_impl!((std::net::Ipv4Addr, u16));
to_socket_addrs_impl!((std::net::Ipv6Addr, u16));
to_socket_addrs_impl!(std::net::SocketAddr);
to_socket_addrs_impl!(std::net::SocketAddrV4);
to_socket_addrs_impl!(std::net::SocketAddrV6);

/// An unconnected TCP socket.
pub struct TcpSocket {
    sock:   Socket,
    regfd:  Registration,
}

impl TcpSocket {
    fn new(dom: Domain) -> io::Result<TcpSocket> {
        let sock = Socket::new(dom, Type::STREAM, None)?;
        let _ = sock.set_nonblocking(true);
        Ok(TcpSocket { regfd: Registration::new(sock.as_raw_fd()), sock })
    }

    /// New IPv4 TcpSocket.
    pub fn new_v4() -> io::Result<TcpSocket> {
        Self::new(Domain::IPV4)
    }

    /// New IPv6 TcpSocket.
    pub fn new_v6() -> io::Result<TcpSocket> {
        Self::new(Domain::IPV6)
    }

    /// Connect to a remote host.
    async fn connect(self, addr: SocketAddr) -> io::Result<TcpStream> {
        let addr = addr.into();
        loop {
            match self.sock.connect(&addr) {
                Ok(()) => return Ok(TcpStream { strm: self.sock.into(), regfd: self.regfd }),
                Err(e) => {
                    if e.raw_os_error() != Some(libc::EINPROGRESS) &&
                       e.raw_os_error() != Some(libc::EALREADY) {
                        return Err(e);
                    }
                    self.regfd.write_ready().await;
                },
            }
        }
    }
}

/// A TCP stream.
pub struct TcpStream {
    strm:   std::net::TcpStream,
    regfd:  Registration,
}

impl TcpStream {
    /// Construct a nara::TcpStream from a std::net::TcpStream.
    pub fn from_std(stream: std::net::TcpStream) -> io::Result<TcpStream> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(TcpStream {
            strm: stream,
            regfd: Registration::new(fd),
        })
    }

    /// Connect to a remote host.
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
        let addrs = addr.to_socket_addrs().await?;
        let mut err: io::Error = io::ErrorKind::NotFound.into();
        for addr in addrs.into_iter() {
            let sock = if addr.is_ipv4() { TcpSocket::new_v4() } else { TcpSocket::new_v6() };
            match sock?.connect(addr).await {
                Ok(strm) => return Ok(strm),
                Err(e) => err = e,
            }
        }
        Err(err)
    }

    /// Shutdown the write part of the socket.
    pub fn shutdown(&self) -> io::Result<()> {
        self.strm.shutdown(std::net::Shutdown::Write)
    }
}

crate::io::impl_async_read!(TcpStream, strm, regfd);
crate::io::impl_async_write!(TcpStream, strm, regfd, shutdown);
