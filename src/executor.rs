use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::mpsc;

use crate::reactor::Reactor;
use crate::task::{JoinHandle, Task};
use crate::time::Timer;

pub(crate) struct Executor {
    // Reactor
    pub reactor: Reactor,
    // Timers.
    pub timer: Timer,
    // send cross-thread wakeups here.
    tx: mpsc::Sender<usize>,
    // receive cross-thread wakeups here.
    rx: mpsc::Receiver<usize>,
    // waiting to run.
    runq: RefCell<VecDeque<Task>>,
    // tasks not currently running.
    tasks: RefCell<HashMap<usize, Task>>,
    // current task.
    current_id: Cell<usize>,
    // current task woken?
    current_woken: Cell<bool>,
    // next unique id
    next_id: Cell<usize>,
}

impl Executor {
    pub fn new(reactor: Reactor, timer: Timer) -> Self {
        let (tx, rx) = mpsc::channel();
        let runq = RefCell::new(VecDeque::new());
        let tasks = RefCell::new(HashMap::new());
        let current_id = Cell::new(0);
        let current_woken = Cell::new(false);
        let next_id = Cell::new(1);
        Executor { tx, rx, tasks, runq, reactor, timer, current_id, current_woken,  next_id }
    }

    // Create a new task and put it on the run queue right away.
    pub fn spawn<F: Future<Output=T> + 'static, T: 'static>(&self, fut: F) -> JoinHandle<T> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let (task, handle) = Task::new(id, self.tx.clone(), fut);
        self.runq.borrow_mut().push_back(task);
        handle
    }

    // Queue a task onto the run queue.
    pub fn queue(&self, task_id: usize) {
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

            // First empty the channel.
            while let Ok(task_id) = self.rx.try_recv() {
                self.queue(task_id);
            }

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

            // Wait for I/O.
            let timeout = self.timer.next_deadline();
            self.reactor.react(timeout);

            // Run timers.
            self.timer.tick();
        }
    }
}
