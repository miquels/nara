use std::cell::RefCell;
use std::thread;
use std::time::Duration;
use std::sync::{mpsc, Arc, Mutex};
use crate::task::JoinHandle;

const MAX_THREADS: usize = 16;
const THREAD_LIFETIME_MS: u64 = 250;

type BoxedFn = Box<dyn FnOnce() -> () + Send + 'static>;

// A threadpool for spawn_blocking().
pub struct ThreadPool {
    tx: mpsc::Sender<BoxedFn>,
    rx: Arc<Mutex<mpsc::Receiver<BoxedFn>>>,
    threads: RefCell<Vec<thread::JoinHandle<()>>>,
}

impl ThreadPool {
    pub fn new() -> ThreadPool {
        // Simply use an unbounded channel so we do not have to implement
        // some Future to wait for a slot to become free. We pay for this
        // in memory usage by Box'ing all the queued functions.
        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        let threads = RefCell::new(Vec::new());
        ThreadPool { threads, rx, tx }
    }

    // Spawn the closure, returning a JoinHandle (which implements Future).
    pub fn spawn<F, T>(&self, f: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let mut threads = self.threads.borrow_mut();

        // Launch more threads, up to MAX_THREADS.
        if threads.len() < MAX_THREADS {
            let rx = self.rx.clone();
            threads.push(thread::spawn(move || worker(rx)));
        }

        // Now move the closure to the ThreadPool executor.
        let handle = JoinHandle::new(0);
        let handle2 = handle.clone();
        let trunk = move || {
            handle2.set_result(f());
        };

        // maybe turn SendError into JoinError?
        let _ = self.tx.send(Box::new(trunk));

        // Garbage collection.
        let ended = threads.iter().any(|t| t.is_finished());
        if ended {
            threads.retain(|t| !t.is_finished());
        }

        // Return JoinHandle.
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
        let work = match rxer.recv_timeout(Duration::from_millis(THREAD_LIFETIME_MS)) {
            Ok(work) => work,
            Err(_) => break,
        };
        drop(rxer);
        work();
    }
}
