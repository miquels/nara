use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::task::Waker;
use std::time::{Duration, Instant};
use std::cell::{Cell, RefCell};

pub(crate) struct Timer {
    timers: RefCell<BTreeMap::<Sleep, Option<Waker>>>,
    next_id: Cell<u64>,
}

impl Timer {
    // Return a new Timer.
    pub fn new() -> Timer {
        Timer {
            timers: RefCell::new(BTreeMap::new()),
            next_id: Cell::new(1),
        }
    }

    // Return how long it will take until the next timer goes off.
    // This is used by the reactor as a timeout.
    pub fn next_deadline(&self) -> Option<Duration> {
        let timers = self.timers.borrow();
        let (first, _) = timers.first_key_value()?;
        let now = Instant::now();
        Some(first.deadline.checked_duration_since(now).unwrap_or(Duration::ZERO))
    }

    // Wake waiters on epired timers.
    pub fn tick(&self) {
        let mut timers = self.timers.borrow_mut();
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
    crate::runtime::with_timer(move |timer| {
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
            crate::runtime::with_timer(|timer| {
                timer.timers.borrow_mut().insert(self.clone(), Some(cx.waker().clone()));
            });
            return Poll::Pending;
        }
        Poll::Ready(())
    }
}
