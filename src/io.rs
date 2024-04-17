// re-exports.
pub use futures_io::{AsyncRead, AsyncWrite};
pub use futures_util::{AsyncReadExt, AsyncWriteExt};

//
// Inner implementation details.
//

// A macro that can be used to implement AsyncRead on a struct '$type'.
//
// That struct needs to have at least two members:
// - $reader: an object that implements std::io::Read, and is set to non-blocking.
// - $registration: a Registration struct.
//
macro_rules! impl_async_read {
    ($type: ty, $reader: ident, $registration: ident) => {
        impl $crate::io::AsyncRead for $type {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &mut [u8]
            ) -> std::task::Poll<std::io::Result<usize>> {
                use std::io::Read;
                let mut this = self.as_mut();
                match this.$reader.read(buf) {
                    Ok(n) => std::task::Poll::Ready(Ok(n)),
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            let waker = cx.waker().clone();
                            this.$registration.wake_when($crate::reactor::Interest::Read, waker);
                            std::task::Poll::Pending
                        } else {
                            std::task::Poll::Ready(Err(e))
                        }
                    }
                }
            }
        }
    }
}
pub(crate) use impl_async_read;

// A macro that can be used to implement AsyncWrite on a struct '$type'.
//
// That struct needs to have at least two members:
// - $writer: an object that implements std::io::Write, and is set to non-blocking.
// - $registration: a Registration struct.
// - optional: $closer - method on $type to shut down the writer.
//
macro_rules! impl_async_write {
    // helpers.
    (@CLOSE $self: ident, _NONE) => {
        Ok(())
    };
    (@CLOSE $self: ident, $closer: ident) => {
        $self.$closer()
    };

    // entrypoint without explicit closer.
    ($type: ty, $writer: ident, $registration: ident) => {
        $crate::io::impl_async_write!($type, $writer, $registration, _NONE);
    };

    // entrypoint with explicit closer.
    ($type: ty, $writer: ident, $registration: ident, $closer: ident) => {

        impl ::futures_io::AsyncWrite for $type {
            fn poll_write(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &[u8]
            ) -> std::task::Poll<std::io::Result<usize>> {
                use std::io::Write;
                let mut this = self.as_mut();
                match this.$writer.write(buf) {
                    Ok(n) => std::task::Poll::Ready(Ok(n)),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        let waker = cx.waker().clone();
                        this.$registration.wake_when($crate::reactor::Interest::Write, waker);
                        std::task::Poll::Pending
                    },
                    Err(e) => std::task::Poll::Ready(Err(e)),
                }
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn poll_close(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>
            ) -> std::task::Poll<std::io::Result<()>> {
                let _this = self.get_mut();
                std::task::Poll::Ready($crate::io::impl_async_write!(@CLOSE _this, $closer))
            }
        }
    }
}
pub(crate) use impl_async_write;
