use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::collections::VecDeque;

// Re-exports.
pub use std::sync::mpsc::{RecvError, SendError};

// Shared channel struct.
struct Channel<T> {
    queue: VecDeque<T>,
    capacity: usize,
    tx_wakers: VecDeque<(u64, Waker)>,
    rx_waker: Option<Waker>,
    recv_gone: bool,
    last_id: u64,
}

impl<T> Channel<T> {
    fn new(capacity: usize) -> Rc<RefCell<Channel<T>>> {
        Rc::new(RefCell::new(Channel {
            queue: VecDeque::new(),
            capacity,
            tx_wakers: VecDeque::new(),
            rx_waker: None,
            recv_gone: false,
            last_id: 1,
        }))
    }
}

/// Create a new channel.
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let channel = Channel::new(capacity);
    (Sender { id: 1, channel: channel.clone() }, Receiver { channel })
}

/// Create a new unbounded channel
pub fn unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let channel = Channel::new(usize::MAX);
    (UnboundedSender { channel: channel.clone() }, UnboundedReceiver { channel })
}

/// Create a new unbounded channel
pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    unbounded_channel()
}

/// Sender side of the bounded channel. Can have multiple instances.
pub struct Sender<T> {
    id: u64,
    channel: Rc<RefCell<Channel<T>>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let id = {
            let mut channel = self.channel.borrow_mut();
            channel.last_id += 1;
            channel.last_id
        };
        Sender { id, channel: self.channel.clone() }
    }
}

impl<T> Sender<T> {
    /// Send a value to the receiver.
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut store = Some(value);
        std::future::poll_fn(|cx: &mut Context<'_>| {

            // See if the receiver is still there.
            let mut channel = self.channel.borrow_mut();
            if channel.recv_gone {
                return Poll::Ready(Err(SendError(store.take().unwrap())));
            }

            // If under capacity, just push.
            if channel.queue.len() < channel.capacity {
                channel.queue.push_back(store.take().unwrap());
                // Wake receiver.
                channel.rx_waker.take().map(|w| w.wake());
                return Poll::Ready(Ok(()));
            }

            // Arrange for us to be woken when the receiver runs.
            if let Some(w) = channel.tx_wakers.iter_mut().find(|w| w.0 == self.id) {
                w.1.clone_from(cx.waker());
            } else {
                channel.tx_wakers.push_back((self.id, cx.waker().clone()));
            }
            Poll::Pending
        }).await
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut channel = self.channel.borrow_mut();
        // Remove any wakers.
        channel.tx_wakers.retain(|w| w.0 != self.id);
        if Rc::strong_count(&self.channel) == 2 {
            // Last sender, notify receiver.
            channel.rx_waker.take().map(|w| w.wake());
        }
    }
}

/// Sender side of the unbounded channel. Can have multiple instances.
#[derive(Clone)]
pub struct UnboundedSender<T> {
    channel: Rc<RefCell<Channel<T>>>,
}

impl<T> UnboundedSender<T> {
    /// Send a value to the receiver. Never blocks.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        // See if the receiver is still there.
        let mut channel = self.channel.borrow_mut();
        if channel.recv_gone {
            return Err(SendError(value));
        }
        // Push and wake receiver.
        channel.queue.push_back(value);
        channel.rx_waker.take().map(|w| w.wake());
        Ok(())
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        let mut channel = self.channel.borrow_mut();
        if Rc::strong_count(&self.channel) == 2 {
            // Last sender, notify receiver.
            channel.rx_waker.take().map(|w| w.wake());
        }
    }
}

/// A Receiver. There can only be one instance.
pub struct Receiver<T> {
    channel: Rc<RefCell<Channel<T>>>,
}
pub type UnboundedReceiver<T> = Receiver<T>;

impl<T> Receiver<T> {
    /// Receive a message from the channel.
    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(move |cx: &mut Context<'_>| {
            let mut channel = self.channel.borrow_mut();

            // See if there is data.
            if let Some(value) = channel.queue.pop_front() {
                if channel.capacity != usize::MAX {
                    channel.tx_wakers.pop_front().map(|w| w.1.wake());
                }
                return Poll::Ready(Some(value));
            }

            // See if there are any senders left.
            if Rc::strong_count(&self.channel) == 1 {
                return Poll::Ready(None);
            }

            // Set a waker.
            if let Some(w) = channel.rx_waker.as_mut() {
                w.clone_from(cx.waker());
            } else {
                channel.rx_waker.replace(cx.waker().clone());
            }
            Poll::Pending
        }).await
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Notify all senders that we're gone.
        let mut channel = self.channel.borrow_mut();
        channel.recv_gone = true;
        channel.tx_wakers.drain(..).for_each(|w| w.1.wake());
    }
}
