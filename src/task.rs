use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;

use crate::executor::ErasedFuture;

// Shorthand for Send + 'static
pub trait Static: Send + 'static {}
impl<T: Send + 'static> Static for T {}

// Task.
pub(crate) struct Task<F, T> {
    // Unique id
    id:             usize,
    // Future to run.
    future:         Pin<Box<F>>,
    // To wake the executor.
    waker:          Waker,
    // result when future is done.
    join_handle:    JoinHandle<T>,
}

impl<F, T> Task<F, T>
where
    F: Future<Output = T>,
{
    // Create a new Task.
    pub fn new(id: usize, tx: mpsc::Sender<usize>, fut: F) -> (Self, JoinHandle<T>) {
        let join_handle = JoinHandle::new(id);
        let task = Task {
            id,
            future: Box::pin(fut),
            waker: Arc::new(TaskWaker{ id, tx }).into(),
            join_handle: join_handle.clone(),
        };
        (task, join_handle)
    }
}

impl<F, T> ErasedFuture for Task<F, T>
where
    F: Future<Output = T>,
{
    // Simplified poll function.
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<()> {

        // Poll the inner future.
        let _ = crate::runtime::current();
        match self.future.as_mut().poll(cx) {
            Poll::Ready(res) => {
                // finished. store result in the JoinHandle.
                self.join_handle.set_result(res);
                Poll::Ready(())
            },
            Poll::Pending => Poll::Pending,
        }
    }

    fn id(&self) -> usize {
        self.id
    }

    fn waker(&self) -> Waker {
        self.waker.clone()
    }
}

// The task waker just sends the task id to the executor.
struct TaskWaker {
    tx:     mpsc::Sender<usize>,
    id:     usize,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.tx.send(self.id).unwrap();
    }
}

#[derive(Debug)]
pub struct JoinError;

// spawn() and spawn_blocking return a JoinHandle, which can be awaited on,
// and which will return the return value of the spawned task.
pub struct JoinHandle<T> {
    pub(crate) id: usize,
    pub(crate) inner: Arc<Mutex<JoinInner<T>>>,
}

pub(crate) struct JoinInner<T> {
    pub result: Option<T>,
    pub waker: Option<Waker>,
}

impl<T> JoinHandle<T> {
    // Create new, empty JoinHandle.
    fn new(id: usize) -> JoinHandle<T> {
        let inner = JoinInner { result: None, waker: None };
        JoinHandle { id, inner: Arc::new(Mutex::new(inner)) }
    }

    // non-public clone().
    fn clone(&self) -> JoinHandle<T> {
        JoinHandle { id: self.id, inner: self.inner.clone() }
    }

    // store the result and wake the task that is waiting on this handle.
    fn set_result(&self, res: T) {
        let mut inner = self.inner.lock().unwrap();
        inner.result = Some(res);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
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

pub fn spawn_blocking<F: FnOnce() -> R + Static, R: Static>(f: F) -> JoinHandle<R> {
    let rwaker = crate::runtime::current().inner.reactor.waker();
    let handle = JoinHandle::new(0);
    let handle2 = handle.clone();
    thread::spawn(move || {
        handle2.set_result(f());
        rwaker.wake();
    });
    handle
}

pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(fut: F) -> JoinHandle<T> {
    crate::runtime::inner().executor.spawn(fut)
}
