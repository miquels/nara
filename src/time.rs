use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::Waker;
use std::time::{Duration, Instant};

pub(crate) struct Timer {
    inner:  Rc<InnerTimer>,
}

pub(crate) struct InnerTimer {
    timers: RefCell<BTreeMap::<Sleep, Option<Waker>>>,
    next_id: Cell<usize>,
}

thread_local! {
    // Valid after Timer::activate(), invalid after Timer::deactivate()
    static TIMER: RefCell<Weak<InnerTimer>> = RefCell::new(Weak::new());
}

impl Timer {
    // Return a new Timer.
    pub fn new() -> Timer {
        let inner = Rc::new(InnerTimer {
            timers: RefCell::new(BTreeMap::new()),
            next_id: Cell::new(1),
        });
        Timer { inner }
    }

    // Activate the thread-local reference.
    pub fn activate(&self) {
        TIMER.with_borrow_mut(|t| *t = Rc::downgrade(&self.inner));
    }

    // De-activate (and free) the thread-local reference.
    pub fn deactivate(&self) {
        TIMER.with_borrow_mut(|t| *t = Weak::new());
    }

    // Return how long it will take until the next timer goes off.
    // This is used by the reactor as a timeout.
    pub fn next_deadline(&self) -> Option<Duration> {
        let timers = self.inner.timers.borrow();
        let (first, _) = timers.first_key_value()?;
        let now = Instant::now();
        Some(first.deadline.checked_duration_since(now).unwrap_or(Duration::ZERO))
    }

    // Wake waiters on epired timers.
    pub fn tick(&self) {
        let mut timers = self.inner.timers.borrow_mut();
        let now = Instant::now();
        while let Some(entry) = timers.first_entry() {
            if entry.key().deadline > now {
                break;
            }
            let (_, mut waker) = entry.remove_entry();
            waker.take().map(|w| w.wake());
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Sleep {
    deadline:   Instant,
    id:         usize,
}

impl Sleep {
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn is_elapsed(&self) -> bool {
        Instant::now() >= self.deadline
    }

    fn clone(&self) -> Self {
        Sleep { deadline: self.deadline, id: self.id }
    }
}

pub fn sleep_until(deadline: Instant) -> Sleep {
    TIMER.with_borrow(|t| {
        let timer = t.upgrade().unwrap();
        let id = timer.next_id.get();
        timer.next_id.set(id + 1);
        let key = Sleep { deadline, id };
        timer.timers.borrow_mut().insert(key.clone(), None);
        key
    })
}

pub fn sleep(duration: Duration) -> Sleep {
    sleep_until(Instant::now() + duration)
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // check for initial poll, or spurious wakeup.
        if !self.is_elapsed() {
            TIMER.with_borrow(|t| {
                let timer = t.upgrade().unwrap();
                timer.timers.borrow_mut().insert(self.clone(), Some(cx.waker().clone()));
            });
            return Poll::Pending;
        }
        Poll::Ready(())
    }
}
