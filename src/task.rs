use std::future::Future;
use std::os::fd::RawFd;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use crate::syscall;

// Task.
pub(crate) struct Task {
    // Unique id
    pub id:         u64,
    // To wake the executor.
    pub waker:      Waker,
    // Future to run.
    future:         Pin<Box<dyn Future<Output=()>>>,
}

impl Task {
    // Create a new Task.
    pub fn new<F, T>(id: u64, tx: RawFd, fut: F) -> (Self, JoinHandle<T>)
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        let join_handle = JoinHandle::new(id);

        // Wrap the future with a Future<Output=()> so that Task doesn't have to be generic.
        let join_handle2 = join_handle.clone();
        let trampoline = async move {
            let res = fut.await;
            join_handle2.set_result(res);
        };

        // Store id, future and waker in the Task struct nice and cosy together.
        // Note that in the current implementation, `tx` is in blocking mode!
        let task = Task {
            id,
            future: Box::pin(trampoline),
            waker: Arc::new(TaskWaker{ id, tx }).into(),
        };

        (task, join_handle)
    }

    // Poll the Task.
    pub fn poll(&mut self) -> Poll<()> {
        let mut cx = Context::from_waker(&self.waker);
        self.future.as_mut().poll(&mut cx)
    }
}

// The task waker makes sure the task gets queued and run by the executor.
struct TaskWaker {
    id:         u64,
    // The below for cross-thread waking.
    tx:         RawFd,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        crate::executor::EXECUTOR.with_borrow(|e| {
            if let Some(executor) = e.upgrade() {
                // If we're on the same thread as the executor, queue directly.
                executor.queue(self.id);
            } else {
                // We're on another thread, so send task id over pipe.
                // Note that self.tx is a _blocking_ file descriptor.
                let _ = syscall::write(self.tx, &self.id.to_ne_bytes()[..]);
            }
        })
    }
}

#[derive(Debug)]
pub struct JoinError;
impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JoinError")
    }
}
impl std::error::Error for JoinError {}

// spawn() and spawn_blocking return a JoinHandle, which can be awaited on,
// and which will return the return value of the spawned task.
pub struct JoinHandle<T> {
    pub(crate) id: u64,
    pub(crate) inner: Arc<Mutex<JoinInner<T>>>,
}

pub(crate) struct JoinInner<T> {
    pub result: Option<T>,
    pub waker: Option<Waker>,
}

impl<T> JoinHandle<T> {
    // Create new, empty JoinHandle.
    pub(crate) fn new(id: u64) -> JoinHandle<T> {
        let inner = JoinInner { result: None, waker: None };
        JoinHandle { id, inner: Arc::new(Mutex::new(inner)) }
    }

    // non-public clone().
    pub(crate) fn clone(&self) -> JoinHandle<T> {
        JoinHandle { id: self.id, inner: self.inner.clone() }
    }

    // store the result and wake the task that is waiting on this handle.
    pub(crate) fn set_result(&self, res: T) {
        let mut inner = self.inner.lock().unwrap();
        inner.result = Some(res);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
    }

    pub(crate) fn get_result(&self) -> Option<T> {
        self.inner.lock().unwrap().result.take()
    }
}

// A JoinHandle can be awaited.
impl <T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.lock().unwrap();
        match inner.result.take() {
            None => {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            },
            Some(res) => Poll::Ready(Ok(res)),
        }
    }
}

pub fn spawn_blocking<F: FnOnce() -> R + Send + 'static, R: Send + 'static>(f: F) -> JoinHandle<R> {
    crate::executor::EXECUTOR.with_borrow(move |e| {
        let executor = e.upgrade().unwrap();
        executor.pool.spawn(f)
    })
}

pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(fut: F) -> JoinHandle<T> {
    crate::executor::EXECUTOR.with_borrow(|e| {
        let executor = e.upgrade().unwrap();
        executor.spawn(fut)
    })
}
