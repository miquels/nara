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
    // We have 2 references to a inner runtime, the one in the Runtime
    // struct we return from Runtime::new(), and the one we keep in a
    // thread local variable, so we need an Rc.
    pub(crate) inner: Rc<InnerRuntime>,
}

// Inner runtime only holds the executor.
pub(crate) struct InnerRuntime {
    executor: Executor,
}

// thread local reference to the inner runtime.
thread_local! {
    pub(crate) static RUNTIME: RefCell<rc::Weak<InnerRuntime>> = RefCell::new(rc::Weak::new());
}

// helper for the with_ functions below.
fn inner() -> Rc<InnerRuntime> {
    RUNTIME.with_borrow(|rt| rt.upgrade()).unwrap()
}

// Get a temporary reference to the Executor.
pub(crate) fn with_executor<F: FnOnce(&Executor) -> R, R: 'static>(f: F) -> R {
    let inner = inner();
    f(&inner.executor)
}

// Get a temporary reference to the Timer.
pub(crate) fn with_timer<F: FnOnce(&Timer) -> R, R: 'static>(f: F) -> R {
    let inner = inner();
    f(&inner.executor.timer)
}

// Get a temporary reference to the Reactor.
pub(crate) fn with_reactor<F: FnOnce(&Reactor) -> R, R: 'static>(f: F) -> R {
    let inner = inner();
    f(&inner.executor.reactor)
}

impl Runtime {
    // Create a new Runtime.
    pub fn new() -> io::Result<Runtime> {
        let reactor = Reactor::new();
        let timer = Timer::new();
        let executor = Executor::new(reactor, timer);
        let inner = Rc::new(InnerRuntime { executor });
        Ok(Runtime { inner })
    }

    // Activate runtime context.
    pub fn enter(&self) -> EnterGuard {
        EnterGuard::new(self)
    }

    // Run a future on the executor. Runtime gets activated.
    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        let _guard = self.enter();
        self.inner.executor.block_on(fut)
    }
}

// Creating an EnterGuard puts a Weak pointer to the inner runtime in
// the thread-local RUNTINE. As soon as the EnterGuard is dropped the
// reference is removed again. So only when holding an EnterGuard, or
// when calling block_on(), is the runtime context active.
pub struct EnterGuard<'a> {
    lifetime: std::marker::PhantomData<&'a Runtime>,
}

impl<'a> EnterGuard<'a> {
    fn new(runtime: &'a Runtime) -> EnterGuard {
        RUNTIME.with_borrow_mut(|rt| {
            if let Some(rt) = rt.upgrade() {
                if !Rc::ptr_eq(&runtime.inner, &rt) {
                    panic!("already in a runtime context!");
                }
            } else {
                *rt = Rc::downgrade(&runtime.inner);
            }
        });
        EnterGuard {
            lifetime: std::marker::PhantomData,
        }
    }
}

impl<'a> Drop for EnterGuard<'a> {
    fn drop(&mut self) {
        RUNTIME.with_borrow_mut(|rt| *rt = rc::Weak::new());
    }
}
