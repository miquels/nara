//
// A simple reactor that uses poll(2) to react to I/O events.
// Why poll(2)? Because it's ubiquitous, works on any unix variant.
//
use std::cell::{Cell, RefCell};
use std::os::fd::RawFd;
use std::rc::{Rc, Weak};
use std::task::Waker;
use std::time::Duration;

use crate::syscall;

// Reactor handle.
pub struct Reactor {
    inner:      Rc<RefCell<InnerReactor>>,
}

// Actual reactor.
pub struct InnerReactor {
    pollfds: Vec<libc::pollfd>,
    fd_info: Vec<FdWaiters>,
    next_id: u64,
}

thread_local! {
    // Valid after Reactor::activate(), invalid after Reactor::deactivate()
    static REACTOR: RefCell<Weak<RefCell<InnerReactor>>> = RefCell::default();
}

// Interest.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interest {
    Read = libc::POLLIN as _,
    Write = libc::POLLOUT as _,
}

// One waiter.
#[derive(Debug)]
struct FdWaiter {
    reg_id:     u64,
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

impl Reactor {

    // Create a new reactor.
    pub fn new() -> Reactor {
        let inner = InnerReactor {
            pollfds: Vec::new(),
            fd_info: Vec::new(),
            next_id: 1,
        };
        Reactor{ inner: Rc::new(RefCell::new(inner)) }
    }

    // Activate the thread-local reference.
    pub fn activate(&self) {
        REACTOR.with_borrow_mut(|r| *r = Rc::downgrade(&self.inner));
    }

    // De-activate (and free) the thread-local reference.
    pub fn deactivate(&self) {
        REACTOR.with_borrow_mut(|r| *r = Weak::new());
    }

    // Like Registration::new(), but optimized for the case
    // where you already have a Reactor handle.
    pub fn registration(&self, fd: RawFd) -> Registration {
        Registration::new_with_reactor(fd, &self.inner)
    }

    // Run the reactor.
    pub fn react(&self, timeout: Option<Duration>) {
        // We need to delegate this to impl InnerReactor.
        let mut inner = self.inner.borrow_mut();
        inner.react(timeout)
    }
}

impl InnerReactor {

    // Run the reactor.
    fn react(&mut self, timeout: Option<Duration>) {
        const INTERESTING: u32 = (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) as u32;

        // Run the poll system call.
        let mut todo = match syscall::poll(&mut self.pollfds, timeout) {
            Ok(n) => n,
            Err(_) => return,
        };

        // Find all waiters with matching interest.
        for i in 0 .. self.pollfds.len() {

            let pollfd = &mut self.pollfds[i];
            if pollfd.revents != 0 {

                // An event happened on this fd.
                let fd_waiters = &mut self.fd_info[i];

                let waiters = fd_waiters
                    .waiters
                    .drain(..)
                    .filter_map(|w| {
                        // See if this waiter is interested.
                        let active = (w.interest as u32 | INTERESTING) & pollfd.revents as u32;
                        if active != 0 {
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

                todo -= 1;
                if todo == 0 {
                    break;
                }
            }
        }
    }

    // Find the filedescriptor, starting at 'reg.fd_index' and then going back
    // from there. This works because we only ever remove entries from the
    // middle of the Vec, and we only ever add them to the end. If the index
    // changed, update the value in `reg` for the next lookup.
    fn fd_index(&self, reg: &Registration, do_update: bool) -> usize {
        let index_hint = reg.fd_index.get();
        let start = std::cmp::min(self.pollfds.len(), index_hint + 1);
        for idx in (0 .. start).rev() {
            if self.pollfds[idx].fd.abs() == reg.fd {
                if idx != index_hint && do_update {
                    reg.fd_index.set(idx);
                }
                return idx;
            }
        }
        panic!("cannot find file descriptor {} registered with the reactor", reg.fd);
    }

    // Register a file descriptor to be monitored.
    fn register_fd(&mut self, fd: RawFd) -> usize {

        // See if we can find 'fd' already registered.
        if let Some((idx, _)) = self.pollfds.iter().enumerate().find(|(_, p)| p.fd == fd) {
            // Already have it, just increase refcount.
            self.fd_info[idx].refcount += 1;
            idx
        } else {
            // Need to add this file descriptor.
            self.pollfds.push(libc::pollfd{ fd: -fd, events: 0, revents: 0 });
            self.fd_info.push(FdWaiters{ refcount: 1, waiters: Vec::new() });
            self.fd_info.len() - 1
        }
    }

    // Deregister file descriptor.
    fn deregister_fd(&mut self, reg: &Registration) {
        let idx = self.fd_index(reg, false);
        if self.fd_info[idx].refcount == 1 {
            // Last reference, so remove it from the reactor.
            self.pollfds.remove(idx);
            self.fd_info.remove(idx);
        } else {
            // Just decrements refcount.
            self.fd_info[idx].refcount -= 1;
        }
    }

    // Request to be woken up when event of interest happens on fd.
    fn add_wake_when(&mut self, reg: &Registration, interest: Interest, waker: Waker) {
        let idx = self.fd_index(reg, true);
        // Add the waiter to the list, and update events to listen for.
        self.fd_info[idx].waiters.push(FdWaiter{ interest, reg_id: reg.id, waker });
        self.pollfds[idx].events = self.fd_info[idx].poll_bits().try_into().unwrap();
        self.pollfds[idx].revents = 0;
        self.pollfds[idx].fd = reg.fd;
    }

    // Remove waker.
    fn remove_wake_when(&mut self, reg: &Registration, interest: Interest) {
        let idx = self.fd_index(reg, true);
        self.fd_info[idx].waiters.retain(|w| w.reg_id != reg.id && w.interest != interest);
        self.pollfds[idx].events = self.fd_info[idx].poll_bits().try_into().unwrap();
    }

    // Check for spurious wakeup.
    fn was_woken(&self, reg: &Registration) -> bool {
        // If we have an entry with our registration id, we weren't woken up!
        let idx = self.fd_index(reg, true);
        self.fd_info[idx].waiters.iter().find(|w| w.reg_id == reg.id).is_none()
    }
}

// A filedescriptor handle with connection to the Reactor.
pub struct Registration {
    id:         u64,
    fd:         RawFd,
    fd_index:   Cell<usize>,
    reactor:    Weak<RefCell<InnerReactor>>,
}

impl Registration {
    pub fn new(fd: RawFd) -> Registration {
        REACTOR.with_borrow(|inner| {
            let inner = inner.upgrade().unwrap();
            Registration::new_with_reactor(fd, &inner)
        })
    }

    fn new_with_reactor(fd: RawFd, inner: &Rc<RefCell<InnerReactor>>) -> Registration {
        let mut inner2 = inner.borrow_mut();
        let id = inner2.next_id;
        inner2.next_id += 1;
        Registration {
            id,
            fd,
            fd_index: Cell::new(inner2.register_fd(fd)),
            reactor: Rc::downgrade(inner),
        }
    }

    pub fn wake_when(&self, interest: Interest, waker: Waker) {
        let inner = self.reactor.upgrade().unwrap();
        inner.borrow_mut().add_wake_when(self, interest, waker);
    }

    pub fn remove_wake_when(&self, interest: Interest) {
        let inner = self.reactor.upgrade().unwrap();
        inner.borrow_mut().remove_wake_when(self, interest);
    }

    pub fn was_woken(&self) -> bool {
        let inner = self.reactor.upgrade().unwrap();
        let res = inner.borrow().was_woken(self);
        res
    }

    pub async fn write_ready(&self) {
        FdReady { reg: self, has_no_waker: true, interest: Interest::Write }.await;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.reactor.upgrade().map(|r| r.borrow_mut().deregister_fd(&self));
    }
}

// Implement as struct, so that we can implement Drop on the struct.
struct FdReady<'a> {
    reg: &'a Registration,
    has_no_waker: bool,
    interest: Interest,
}

use std::task::{Context, Poll};

impl<'a> std::future::Future for FdReady<'a> {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let reactor = this.reg.reactor.upgrade().unwrap();
        let mut reactor = reactor.borrow_mut();
        if !reactor.was_woken(this.reg) {
            return Poll::Pending;
        }
        if std::mem::take(&mut this.has_no_waker) {
            reactor.add_wake_when(this.reg, this.interest, cx.waker().clone());
            return Poll::Pending;
        }
        this.has_no_waker = true;
        Poll::Ready(())
    }
}

impl<'a> Drop for FdReady<'a> {
    fn drop(&mut self) {
        if !self.has_no_waker {
            self.reg.remove_wake_when(self.interest);
        }
    }
}
