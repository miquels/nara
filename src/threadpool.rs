use std::thread;
use std::sync::{mpsc, Arc, Mutex};
use crate::task::JoinHandle;

const NUM_THREADS: usize = 16;

type BoxedFn = Box<dyn FnOnce() -> () + Send + 'static>;

// A threadpool for spawn_blocking().
pub struct ThreadPool {
    tx: mpsc::Sender<BoxedFn>,
    // rx and threads are unused for now, but we'll need them for automatic scaling.
    _rx: Arc<Mutex<mpsc::Receiver<BoxedFn>>>,
    _threads: Vec<thread::JoinHandle<()>>,
}

impl ThreadPool {
    pub fn new() -> ThreadPool {
        // Simply use an unbounded channel so we do not have to implement
        // some Future to wait for a slot to become free. We pay for this
        // in memory usage by Box'ing all the queued functions.
        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        let mut threads = Vec::new();
        for _ in 0 .. NUM_THREADS {
            let rx = rx.clone();
            threads.push(thread::spawn(move || worker(rx)));
        }
        ThreadPool { _threads: threads, _rx: rx, tx }
    }

    // Spawn the closure, returning a JoinHandle (which implements Future).
    pub fn spawn<F, T>(&self, f: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let handle = JoinHandle::new(0);
        let handle2 = handle.clone();
        let trunk = move || {
            handle2.set_result(f());
        };
        // maybe turn SendError into JoinError?
        let _ = self.tx.send(Box::new(trunk));
        handle
    }
}

//
// Simple worker. Lock the Receiver and get one task, then run it and report result.
//
// Too bad that the implementation in `std` is actually `mpsc`, but is
// only exposed as `mpsc`. If it was `mpsc` we wouldn't need the mutex.
//
fn worker(rx: Arc<Mutex<mpsc::Receiver<BoxedFn>>>) {
    while let Ok(rxer) = rx.lock() {
        let work = match rxer.recv() {
            Ok(work) => work,
            Err(_) => break,
        };
        drop(rxer);
        work();
    }
}
