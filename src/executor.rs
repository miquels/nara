use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::future::Future;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::task::Wake;

use crate::reactor::{Interest, Reactor, Registration};
use crate::syscall;
use crate::task::{JoinHandle, Task};
use crate::time::Timer;

struct ExecutorWaker;

impl Wake for ExecutorWaker {
    fn wake(self: Arc<Self>) {
        crate::runtime::with_executor(move |executor| {
            let mut buf: [u8; 256] = [0; 256];
            let mut fh = &executor.wake_pipe_rx;
            while let Ok(n) = fh.read(&mut buf) {
                if n % 8 != 0 {
                    panic!("read a non-multiple-of-8 from the pipe, expected u64");
                }
                for b in buf[..n].chunks(8) {
                    let id = u64::from_ne_bytes(b.try_into().unwrap());
                    executor.queue(id);
                }
            }
        })
        // We really should re-use 'self' here as a Waker, but we cannot
        // call back into the reactor via Registration at this point
        // because we're being called _from_ the reactor.
    }
}

pub(crate) struct Executor {
    // Reactor
    pub reactor: Reactor,
    // Timers.
    pub timer: Timer,
    // Pipe for cross-thread wakeups.
    wake_pipe: Registration,
    // Read wakeup requests from this file
    wake_pipe_rx: File,
    // Write wkaeup requests to this file.
    wake_pipe_tx: File,
    // waiting to run.
    runq: RefCell<VecDeque<Task>>,
    // tasks not currently running.
    tasks: RefCell<HashMap<u64, Task>>,
    // current task.
    current_id: Cell<u64>,
    // current task woken?
    current_woken: Cell<bool>,
    // next unique id
    next_id: Cell<u64>,
}

impl Executor {
    pub fn new(reactor: Reactor, timer: Timer) -> Self {
        let (rx, tx) = syscall::pipe().unwrap();
        let wake_pipe = reactor.registration(rx.as_raw_fd());
        wake_pipe.wake_when(Interest::Read, Arc::new(ExecutorWaker).into());
        Executor {
            reactor,
            timer,
            wake_pipe,
            wake_pipe_rx: rx,
            wake_pipe_tx: tx,
            runq: RefCell::new(VecDeque::new()),
            tasks: RefCell::new(HashMap::new()),
            current_id: Cell::new(0),
            current_woken: Cell::new(false),
            next_id: Cell::new(1),
        }
    }

    // Create a new task and put it on the run queue right away.
    pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> JoinHandle<T> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let (task, handle) = Task::new(id, self.wake_pipe_tx.as_raw_fd(), fut);
        self.runq.borrow_mut().push_back(task);
        handle
    }

    // Queue a task onto the run queue.
    pub fn queue(&self, task_id: u64) {
        // If we're already the active task, just take a note.
        if self.current_id.get() == task_id {
            self.current_woken.set(true);
            return;
        }
        // Put task on the run queue.
        if let Some(task) = self.tasks.borrow_mut().remove(&task_id) {
            self.runq.borrow_mut().push_back(task);
        }
    }

    pub fn block_on<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> T {
        // Spawn the initial task.
        let handle = self.spawn(fut);

        // This is the entire scheduler.
        loop {

            // Loop over the wake up messages in the queue.
            while let Some(mut task) = self.runq.borrow_mut().pop_back() {

                self.current_id.set(task.id);
                self.current_woken.set(false);

                loop {
                    // Poll the task.
                    if task.poll().is_ready() {

                        // If this was the initial task, return right away.
                        if task.id == handle.id {
                            return handle.get_result().unwrap();
                        }
                        break;
                    }

                    // Stop the loop, _unless_ we woke ourself.
                    if !self.current_woken.replace(false) {
                        // Put the task back.
                        self.tasks.borrow_mut().insert(task.id, task);
                        break;
                    }
                }
            }
            self.current_id.set(0);
            // This is suboptimal, see comment in impl Waker for ExecutorWaker.
            self.wake_pipe.wake_when(Interest::Read, Arc::new(ExecutorWaker).into());

            // Wait for I/O.
            let timeout = self.timer.next_deadline();
            self.reactor.react(timeout);

            // Run timers.
            self.timer.tick();
        }
    }
}
