use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::rc::{self, Rc};
use std::thread_local;

use crate::executor::Executor;
use crate::reactor::Reactor;
use crate::task::JoinHandle;

// Shorthand for Send + 'static
pub trait Static: Send + 'static {}
impl<T: Send + 'static> Static for T {}

pub struct Runtime {
    pub(crate) inner: Rc<InnerRuntime>,
}

pub(crate) struct InnerRuntime {
    pub(crate) executor: Executor,
    pub(crate) reactor: Reactor,
}

impl Drop for InnerRuntime {
    fn drop(&mut self) {
        RUNTIME.with_borrow_mut(|rt| *rt = rc::Weak::new());
    }
}

thread_local! {
    pub(crate) static RUNTIME: RefCell<rc::Weak<InnerRuntime>> = RefCell::new(rc::Weak::new());
}

pub(crate) fn inner() -> Rc<InnerRuntime> {
    RUNTIME.with_borrow(|rt| rt.upgrade()).unwrap()
}

pub fn current() -> Runtime {
    Runtime { inner: inner() }
}

impl Runtime {
    pub fn new() -> io::Result<Runtime> {
        RUNTIME.with_borrow_mut(|rt| {
            if rt.upgrade().is_some() {
                return  Err(io::ErrorKind::AlreadyExists.into());
            }
            let reactor = Reactor::new();
            let executor = Executor::new(reactor.clone());
            let inner = Rc::new(InnerRuntime { executor, reactor });
            *rt = Rc::downgrade(&inner);
            Ok(Runtime { inner })
        })
    }

    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        self.inner.executor.block_on(fut)
    }

    pub fn spawn_blocking<F: FnOnce() -> R + Static, R: Static>(&self, f: F) -> JoinHandle<R> {
        crate::task::spawn_blocking(f)
    }


    pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> JoinHandle<T> {
        self.inner.executor.spawn(fut)
    }
}
