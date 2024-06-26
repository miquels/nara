use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::future::Future;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::task::Wake;

use crate::reactor::{Interest, Reactor, Registration};
use crate::syscall;
use crate::task::{JoinHandle, Task};
use crate::threadpool::ThreadPool;
use crate::time::Timer;

pub (crate) struct Executor {
    inner: Rc<InnerExecutor>,
}

pub(crate) struct InnerExecutor {
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
    // Threadpool for spawn_nonblocking
    pub pool: ThreadPool,
    // Timers
    pub timer: Timer,
    // Reactor (last because needs to be dropped last)
    pub reactor: Reactor,
}

thread_local! {
    // Valid after Executor::activate(), invalid after Executor::deactivate()
    pub(crate) static EXECUTOR: RefCell<Weak<InnerExecutor>> = RefCell::new(Weak::new());
}

impl Executor {
    pub fn new(reactor: Reactor, timer: Timer) -> Self {
        let (rx, tx) = syscall::pipe().unwrap();
        let wake_pipe = reactor.registration(rx.as_raw_fd());
        let inner = Rc::new(InnerExecutor {
            wake_pipe,
            wake_pipe_rx: rx,
            wake_pipe_tx: tx,
            runq: RefCell::new(VecDeque::new()),
            tasks: RefCell::new(HashMap::new()),
            current_id: Cell::new(0),
            current_woken: Cell::new(false),
            next_id: Cell::new(1),
            pool: ThreadPool::new(),
            reactor,
            timer,
        });
        Executor { inner }
    }

    // Activate the thread-local reference.
    pub fn activate(&self) {
        EXECUTOR.with_borrow_mut(|t| *t = Rc::downgrade(&self.inner));
        self.inner.reactor.activate();
        self.inner.timer.activate();
    }

    // De-activate (and free) the thread-local reference.
    pub fn deactivate(&self) {
        EXECUTOR.with_borrow_mut(|t| *t = Weak::new());
        self.inner.reactor.deactivate();
        self.inner.timer.deactivate();
    }

    fn pop_task(&self) -> Option<Task> {
        self.inner.runq.borrow_mut().pop_back()
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let this = &self.inner;

        // The passed in future does not have to be Send or 'static. All current
        // executors work like this, and the ecosystem expects it.
        //
        // So, we cannot just `spawn()` it. We put an empty Task on the queue that references
        // the passed in `future` and run the future directly instead of the task.
        let mut main_future = std::pin::pin!(future);
        let main_task_id = this.spawn_main();

        // This is the entire scheduler.
        loop {

            // Loop over the wake up messages in the queue.
            while let Some(mut task) = self.pop_task() {

                this.current_id.set(task.id);
                this.current_woken.set(false);

                loop {
                    if task.id == main_task_id {
                        // Poll the main future.
                        use std::task::{Context, Poll};
                        let mut cx = Context::from_waker(&task.waker);
                        if let Poll::Ready(output) = main_future.as_mut().poll(&mut cx) {
                            return output;
                        }
                    } else {
                        // Poll the task.
                        if task.poll().is_ready() {
                            break;
                        }
                    }

                    // Stop the loop, _unless_ we woke ourself.
                    if !this.current_woken.replace(false) {
                        // Put the task back.
                        this.tasks.borrow_mut().insert(task.id, task);
                        break;
                    }
                }
            }
            this.current_id.set(0);

            // This is suboptimal, see comment in impl Waker for ExecutorWaker.
            if this.wake_pipe.was_woken() {
                this.wake_pipe.wake_when(Interest::Read, Arc::new(ExecutorWaker).into());
            }

            // Wait for I/O.
            let timeout = this.timer.next_deadline();
            this.reactor.react(timeout);

            // Run timers.
            this.timer.tick();
        }
    }
}

impl InnerExecutor {

    // Create a new task and put it on the run queue right away.
    pub(crate) fn spawn<F: Future + 'static>(&self, fut: F) -> JoinHandle<F::Output> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let (task, handle) = Task::new(id, self.wake_pipe_tx.as_raw_fd(), fut);
        self.runq.borrow_mut().push_back(task);
        handle
    }

    // Create the main task reference and put it on the run queue right away.
    pub(crate) fn spawn_main(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let task = Task::main_task(id, self.wake_pipe_tx.as_raw_fd());
        self.runq.borrow_mut().push_back(task);
        id
    }

    // Queue a task onto the run queue.
    pub(crate) fn queue(&self, task_id: u64) {
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
}

struct ExecutorWaker;

impl Wake for ExecutorWaker {
    fn wake(self: Arc<Self>) {
        EXECUTOR.with_borrow(|e| {
            let executor = e.upgrade().unwrap();
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
                if n < buf.len() {
                    break;
                }
            }
        })
        // We really should re-use 'self' here as a Waker, but we cannot
        // call back into the reactor via Registration at this point
        // because we're being called _from_ the reactor.
    }
}

