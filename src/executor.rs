use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::task::Context;

use crate::reactor::Reactor;
use crate::task::{ErasedTask, JoinHandle, Task};
use crate::time::Timer;

pub(crate) struct Executor {
    // Reactor
    pub reactor: Reactor,
    // Timer.
    pub timer: Timer,
    // send wakeups here.
    tx: mpsc::Sender<usize>,
    // receive wakeups here.
    rx: mpsc::Receiver<usize>,
    // store tasks here.
    tasks: RefCell<HashMap<usize, Box<dyn ErasedTask>>>,
    // unique id
    next_id: AtomicUsize,
}

impl Executor {
    pub fn new(reactor: Reactor, timer: Timer) -> Self {
        let (tx, rx) = mpsc::channel();
        let tasks = RefCell::new(HashMap::new());
        let next_id = AtomicUsize::new(1);
        Executor { tx, rx, tasks, reactor, timer, next_id }
    }

    pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> JoinHandle<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (task, handle) = Task::new(id, self.tx.clone(), fut);
        self.tasks.borrow_mut().insert(id, Box::new(task));
        self.tx.send(id).unwrap();
        handle
    }

    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        // Spawn the initial task.
        let handle = self.spawn(fut);

        // This is the entire scheduler.
        loop {

            // Loop over the wake up messages in the queue.
            while let Ok(task_id) = self.rx.try_recv() {
                let task = self.tasks.borrow_mut().remove(&task_id);
                if let Some(mut task) = task {

                    // create a Context and poll the future.
                    //
                    // storing the waker in the task is nice, but it does mean
                    // we have to clone the waker because `cx` borrows `task`
                    // read-only, and `task.poll` borrows it mutably.
                    let waker = task.waker().clone();
                    let mut cx = Context::from_waker(&waker);
                    if task.poll(&mut cx).is_ready() {
                        //
                        // If this was the initial task, return right away.
                        //
                        if task.id() == handle.id {
                            return handle.inner.lock().unwrap().result.take().unwrap();
                        }
                    } else {
                        //
                        // Put the future back.
                        //
                        self.tasks.borrow_mut().insert(task.id(), task);
                    }
                }
            }

            // Wait for I/O.
            let timeout = self.timer.next_deadline();
            self.reactor.react(timeout);

            // Run timers.
            self.timer.run();
        }
    }
}
