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

impl Runtime {
    /// Create a new nara Runtime.
    pub fn new() -> io::Result<Runtime> {
        let reactor = Reactor::new();
        let timer = Timer::new();
        let executor = Executor::new(reactor, timer);
        let inner = Rc::new(InnerRuntime { executor });
        Ok(Runtime { inner })
    }

    /// Run a future on the executor.
    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        let _guard = self.enter();
        self.inner.executor.block_on(fut)
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

// Creating an EnterGuard puts a Weak pointer to the inner runtime in
// the thread-local RUNTIME. As soon as the EnterGuard is dropped the
// reference is removed again. So only when holding an EnterGuard, or
// when calling block_on(), is the runtime context active.
pub struct EnterGuard<'a> {
    lifetime: std::marker::PhantomData<&'a Runtime>,
}

impl<'a> EnterGuard<'a> {
    // The EnterGuard has a lifetime that's tied to the Runtime.
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
        runtime.inner.executor.activate();
        EnterGuard {
            lifetime: std::marker::PhantomData,
        }
    }
}

// This makes sure all resources get released.
impl<'a> Drop for EnterGuard<'a> {
    fn drop(&mut self) {
        RUNTIME.with_borrow_mut(|rt| {
            let runtime = rt.upgrade().unwrap();
            runtime.executor.deactivate();
            *rt = rc::Weak::new();
        });
    }
}
