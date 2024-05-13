use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::Waker;
use std::time::{Duration, Instant};

pub(crate) struct Timer {
    inner:  Rc<RefCell<InnerTimer>>,
}

pub(crate) struct InnerTimer {
    timers: BTreeMap::<Sleep, Option<Waker>>,
    next_id: u64,
}

thread_local! {
    // Valid after Timer::activate(), invalid after Timer::deactivate()
    static TIMER: RefCell<Weak<RefCell<InnerTimer>>> = RefCell::new(Weak::new());
}

impl Timer {
    // Return a new Timer.
    pub fn new() -> Timer {
        let inner = Rc::new(RefCell::new(InnerTimer {
            timers: BTreeMap::new(),
            next_id: 1,
        }));
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
        let this = self.inner.borrow();
        let (first, _) = this.timers.first_key_value()?;
        let now = Instant::now();
        Some(first.deadline.checked_duration_since(now).unwrap_or(Duration::ZERO))
    }

    // Wake waiters on epired timers.
    pub fn tick(&self) {
        let mut this = self.inner.borrow_mut();
        let now = Instant::now();
        while let Some(entry) = this.timers.first_entry() {
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
    id:         u64,
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
        let mut this = timer.borrow_mut();
        let id = this.next_id;
        this.next_id += 1;
        let key = Sleep { deadline, id };
        this.timers.insert(key.clone(), None);
        key
    })
}

pub fn sleep(duration: Duration) -> Sleep {
    sleep_until(Instant::now() + duration)
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let timer = TIMER.with_borrow(|t| t.upgrade().unwrap());
        let mut this = timer.borrow_mut();
        // Note, if there is an entry in `timers`, it means that this was
        // a spurious wakeup, not caused by Timer::tick().
        match this.timers.get_mut(self.get_mut()) {
            None => Poll::Ready(()),
            Some(e) => {
                // Only update the entry if it was not set yet.
                e.get_or_insert_with(|| cx.waker().clone());
                Poll::Pending
            },
        }
    }
}
