use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::rc::{self, Rc};
use std::thread_local;

use crate::executor::Executor;
use crate::reactor::Reactor;
use crate::time::Timer;

/// Nara Runtime.
pub struct Runtime {
    pub(crate) executor: Rc<Executor>,
}

// thread local reference to the inner runtime.
thread_local! {
    pub(crate) static EXECUTOR: RefCell<rc::Weak<Executor>> = RefCell::new(rc::Weak::new());
}

impl Runtime {
    /// Create a new nara Runtime.
    pub fn new() -> io::Result<Runtime> {
        let reactor = Reactor::new();
        let timer = Timer::new();
        let executor = Rc::new(Executor::new(reactor, timer));
        Ok(Runtime { executor })
    }

    /// Run a future on the executor.
    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        let _guard = self.enter();
        self.executor.block_on(fut)
    }

    /// Activate the runtime context. Returns an `EnterGuard`.
    ///
    /// This is only needed to initialize objects like `TcpSocket`s that need an
    /// active runtime context while you're not within `Runtime::block_on`.
    /// This context is deactivated once the `EnterGuard is dropped, or after
    /// `Runtime::block_on` exits.
    pub fn enter(&self) -> EnterGuard {
        EnterGuard::new(self)
    }
}

// Creating an EnterGuard puts a Weak pointer to the inner executor in
// the thread-local EXECUTOR. As soon as the EnterGuard is dropped the
// reference is removed again. So only when holding an EnterGuard, or
// when calling block_on(), is the runtime context active.
pub struct EnterGuard<'a> {
    lifetime: std::marker::PhantomData<&'a Runtime>,
}

impl<'a> EnterGuard<'a> {
    // The EnterGuard has a lifetime that's tied to the Runtime.
    fn new(runtime: &'a Runtime) -> EnterGuard {
        EXECUTOR.with_borrow_mut(|rt| {
            if let Some(rt) = rt.upgrade() {
                if !Rc::ptr_eq(&runtime.executor, &rt) {
                    panic!("already in a runtime context!");
                }
            } else {
                *rt = Rc::downgrade(&runtime.executor);
            }
        });
        runtime.executor.activate();
        EnterGuard {
            lifetime: std::marker::PhantomData,
        }
    }
}

// This makes sure all resources get released.
impl<'a> Drop for EnterGuard<'a> {
    fn drop(&mut self) {
        EXECUTOR.with_borrow_mut(|rt| {
            rt.upgrade().map(|rt| rt.deactivate());
            *rt = rc::Weak::new();
        });
    }
}
