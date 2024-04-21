// A simple reactor that uses poll(2) to react to I/O events.
//
// Why poll(2)? Because it's ubiquitous, works on any unix variant.
//
// This module contains one unsafe block - to call poll(2).
//
use std::cell::RefCell;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::rc::{Rc, Weak};
use std::task::Waker;
use std::thread::{self, ThreadId};
use std::time::Duration;

use crate::syscall;

// Reactor handle.
pub struct Reactor {
    inner:      Rc<RefCell<InnerReactor>>,
}

// Actual reactor.
pub struct InnerReactor {
    wake_rx:    File,
    wake_tx:    File,
    pollfds:    Vec<libc::pollfd>,
    fd_waiters: Vec<FdWaiters>,
}

thread_local! {
    // Valid after Reactor::activate(), invalid after Reactor::deactivate()
    static REACTOR: RefCell<Weak<RefCell<InnerReactor>>> = RefCell::new(Weak::new());
}

// Interest.
#[repr(i16)]
#[derive(Clone, Copy, Debug)]
pub enum Interest {
    Read = libc::POLLIN as i16,
    Write = libc::POLLOUT as i16,
}

// One waiter.
#[derive(Debug)]
struct FdWaiter {
    interest:   Interest,
    waker:      Waker,
}

// A list of waiters on a fd.
#[derive(Default, Debug)]
struct FdWaiters {
    refcount:   usize,
    waiters:    Vec<FdWaiter>,
}

impl FdWaiters {
    // Calculate the event mask for poll() for this fd.
    fn poll_bits(&self) -> i16 {
        self.waiters.iter()
            .map(|w| w.interest as i16)
            .fold(0, |mask, i| mask | i) as i16
    }
}

// A ReactorWaker is used to send 1 byte of data over a filedescriptor
// that is being watched by the Reactor. This is needed for wakers
// from other threads, from spawn_blocking() / JoinHandle.
#[derive(Clone)]
pub struct ReactorWaker {
    reactor_thread: ThreadId,
    wake_tx: RawFd,
}

impl ReactorWaker {
    pub fn wake(&self) {
        if thread::current().id() != self.reactor_thread {
            let _ = syscall::write(self.wake_tx, &b"a"[..]);
        }
    }
}

impl Reactor {

    // Create a new reactor.
    pub fn new() -> Reactor {

        // The first slot is reserved for the wake_rx file descriptor.
        let (wake_rx, wake_tx) = syscall::pipe().unwrap();
        let pollfd = libc::pollfd{ fd: wake_rx.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let pollfds = vec![pollfd];
        let fd_waiters = vec![FdWaiters::default()];

        let inner = InnerReactor {
            wake_rx,
            wake_tx,
            pollfds,
            fd_waiters,
        };
        Reactor{ inner: Rc::new(RefCell::new(inner)) }
    }

    // Activate the thread-local reference.
    pub fn activate(&self) {
        REACTOR.with_borrow_mut(|t| *t = Rc::downgrade(&self.inner));
    }

    // De-activate (and free) the thread-local reference.
    pub fn deactivate(&self) {
        REACTOR.with_borrow_mut(|t| *t = Weak::new());
    }

    // Register a file descriptorr to be monitored.
    fn register(&self, fd: RawFd) {
        let mut inner = self.inner.borrow_mut();

        // See if we can find 'fd' already registered.
        let idx = inner.pollfds
            .iter()
            .enumerate()
            .find_map(|(i, f)| if f.fd == fd { Some(i) } else { None });
        if let Some(idx) = idx {
            // Already have it, just increase refcount.
            inner.fd_waiters[idx].refcount += 1;
        } else {
            // Need to add this file descriptor.
            inner.pollfds.push(libc::pollfd{ fd: -fd, events: 0, revents: 0 });
            inner.fd_waiters.push(FdWaiters{ refcount: 1, waiters: Vec::new() });
        }
    }

    // Deregister file descriptor.
    fn deregister(&self, fd: RawFd) {
        let mut inner = self.inner.borrow_mut();

        // Find the matching file descriptor.
        for n in 0 .. inner.pollfds.len() {

            if inner.pollfds[n].fd.abs() == fd {
                // Found it.
                if inner.fd_waiters[n].refcount == 1 {
                    // Last reference, so remove it from the reactor.
                    inner.pollfds.remove(n);
                    inner.fd_waiters.remove(n);
                } else {
                    // Just decrements refcount.
                    inner.fd_waiters[n].refcount -= 1;
                }
                break;
            }
        }
    }

    // Run the reactor.
    pub fn react(&self, timeout: Option<Duration>) {
        // We need to delegate this to impl InnerReactor.
        let mut inner = self.inner.borrow_mut();
        inner.react(timeout)
    }

    pub fn waker(&self) -> ReactorWaker {
        let inner = self.inner.borrow();
        ReactorWaker {
            reactor_thread: thread::current().id(),
            wake_tx: inner.wake_tx.as_raw_fd(),
        }
    }
}

impl InnerReactor {

    // Run the reactor.
    fn react(&mut self, timeout: Option<Duration>) {
        const INTERESTING: u32 = (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) as u32;

        // Run the poll system call.
        let timeout = timeout.map(|t| t.as_millis() as i32).unwrap_or(-1);
        let mut todo = match syscall::poll(&mut self.pollfds, timeout) {
            Ok(n) => n,
            Err(_) => return,
        };

        // Find all waiters with matching interest.
        for i in 0 .. self.pollfds.len() {

            if todo == 0 {
                break;
            }

            let pollfd = &mut self.pollfds[i];
            if pollfd.revents != 0 {
                todo -= 1;

                // First fd is just for wakeup.
                if i == 0 {
                    pollfd.revents = 0;
                    self.drain_wake_rx();
                    continue;
                }

                // An event happened on this fd.
                let fd_waiters = &mut self.fd_waiters[i];

                let waiters = fd_waiters
                    .waiters
                    .drain(..)
                    .filter_map(|w| {
                        // See if this waiter is interested.
                        if (w.interest as u32 | INTERESTING) & pollfd.revents as u32 != 0 {
                            // Yes, wakeup, and remove.
                            w.waker.wake();
                            None
                        } else {
                            // No, keep.
                            Some(w)
                        }
                    }).collect::<Vec<_>>();

                // Put back any left over waiters.
                fd_waiters.waiters = waiters;

                if fd_waiters.waiters.len() > 0 {
                    // We still have waiters, so calculate new events bits.
                    pollfd.events = fd_waiters.poll_bits().try_into().unwrap();
                } else {
                    // No active waiters, so let poll() ignore this fd.
                    pollfd.events = 0;
                    pollfd.fd = -pollfd.fd;
                }
                pollfd.revents = 0;
            }
        }
    }

    // Request to be woken up when event of interest happens on fd.
    fn wake_when(&mut self, fd: RawFd, interest: Interest, waker: Waker) {

        // We iterate over both pollfds and fd_waiters at the same time, to find the fd.
        let pollfds_iter = self.pollfds.iter_mut();
        let fd_waiters_iter = self.fd_waiters.iter_mut();
        pollfds_iter.zip(fd_waiters_iter)
            .find(|(pollfd, _)| pollfd.fd.abs() == fd)
            .map(|(pollfd, fd_waiter)| {

                // Add the waiter to the list, and update events to listen for.
                fd_waiter.waiters.push(FdWaiter{ interest, waker });
                pollfd.events = fd_waiter.poll_bits().try_into().unwrap();
                pollfd.revents = 0;

                // If this entry was inactive. activate it.
                if pollfd.fd < 0 {
                    pollfd.fd = fd;
                }
            });
    }

    fn drain_wake_rx(&mut self) {
        let mut buf: [u8; 256] = [0; 256];
        while let Ok(n) = self.wake_rx.read(&mut buf) {
            if n != 256 {
                break;
            }
        }
    }
}

// A filedescriptor handle with connection to the Reactor.
pub struct Registration {
    fd:         RawFd,
    reactor:    Reactor,
}

impl Registration {
    pub fn new(fd: RawFd) -> Registration {
        let inner = REACTOR.with_borrow(|r| r.upgrade().unwrap());
        let reactor = Reactor { inner };
        reactor.register(fd);
        Registration {
            fd,
            reactor,
        }
    }

    pub fn wake_when(&self, interest: Interest, waker: Waker) {
        let mut inner = self.reactor.inner.borrow_mut();
        inner.wake_when(self.fd, interest, waker)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.reactor.deregister(self.fd);
    }
}

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

// Helper.
macro_rules! impl_async_write_close {
    ($object:ident) => {
        Ok(())
    };
    ($object:ident, $($close:tt)*) => {
        $object.$($close)*
    };
}
pub(crate) use impl_async_write_close;

// A macro that can be used to implement AsyncWrite on a struct '$type'.
//
// That struct needs to have at least two members:
// - $writer: an object that implements std::io::Write, and is set to non-blocking.
// - $registration: a Registration struct.
// - optional: $closer - method on $writer to close the writer.
//
macro_rules! impl_async_write {
    ($type: ty, $writer: ident, $registration: ident $($close:tt)*) => {

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
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            let waker = cx.waker().clone();
                            this.$registration.wake_when($crate::reactor::Interest::Write, waker);
                            std::task::Poll::Pending
                        } else {
                            std::task::Poll::Ready(Err(e))
                        }
                    }
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
                let _writer = &self.as_ref().$writer;
                let res = $crate::reactor::impl_async_write_close!(_writer $($close)*);
                std::task::Poll::Ready(res)
            }
        }
    }
}
pub(crate) use impl_async_write;
