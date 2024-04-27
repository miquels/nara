use std::sync::{Arc, Mutex};
use std::sync::mpsc::{TryRecvError, TrySendError};
use std::task::{Context, Poll, Waker};

// Re-exports.
pub use std::sync::mpsc::{RecvError, SendError};

#[derive(Clone)]
pub struct Sender<T> {
    sender: std::sync::mpsc::SyncSender<T>,
    tx_waker: Arc<Mutex<Option<Waker>>>,
    rx_waker: Arc<Mutex<Option<Waker>>>,
}

pub struct Receiver<T> {
    receiver: std::sync::mpsc::Receiver<T>,
    tx_waker: Arc<Mutex<Option<Waker>>>,
    rx_waker: Arc<Mutex<Option<Waker>>>,
}

pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (sender, receiver) = std::sync::mpsc::sync_channel::<T>(capacity);
    let tx_waker = Arc::new(Mutex::new(None));
    let rx_waker = Arc::new(Mutex::new(None));
    let tx = Sender { sender, tx_waker: tx_waker.clone(), rx_waker: rx_waker.clone() };
    let rx = Receiver { receiver, tx_waker, rx_waker };
    (tx, rx)
}

impl<T> Sender<T> {
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut store = Some(value);
        std::future::poll_fn(move |cx: &mut Context<'_>| {
            let mut set_waker = false;
            let res = loop {

                // Try to send.
                let value = store.take().unwrap();
                match self.sender.try_send(value) {
                    Ok(()) => {
                        self.rx_waker.lock().unwrap().take().map(|w| w.wake());
                        break Ok(());
                    },
                    Err(TrySendError::Disconnected(v)) => break Err(SendError(v)),
                    Err(TrySendError::Full(v)) => store.replace(v),
                };

                // Second time through the loop?
                if set_waker {
                    return Poll::Pending;
                }

                // Set a waker, then call `try_send()` once more to prevent
                // a race condition with the receiver.
                let mut tx_waker = self.tx_waker.lock().unwrap();
                if let Some(w) = tx_waker.as_mut() {
                    w.clone_from(cx.waker());
                } else {
                    tx_waker.replace(cx.waker().clone());
                }
                set_waker = true;
            };

            // We're ready. If we did set a waker we can remove it now.
            if set_waker {
                let mut tx_waker = self.tx_waker.lock().unwrap();
                tx_waker.take();
            }
            Poll::Ready(res)
        }).await
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.rx_waker.lock().unwrap().take().map(|w| w.wake());
    }
}

impl<T> Receiver<T> {
    pub async fn recv(&self) -> Option<T> {
        std::future::poll_fn(move |cx: &mut Context<'_>| {
            let mut set_waker = false;
            let res = loop {

                // Try to receive.
                match self.receiver.try_recv() {
                    Ok(val) => {
                        self.tx_waker.lock().unwrap().take().map(|w| w.wake());
                        break Some(val);
                    },
                    Err(TryRecvError::Disconnected) => break None,
                    Err(TryRecvError::Empty) => {},
                };

                // Second time through the loop?
                if set_waker {
                    return Poll::Pending;
                }

                // Set a waker, then call `try_recv()` once more to prevent
                // a race condition with the sender.
                let mut rx_waker = self.rx_waker.lock().unwrap();
                if let Some(w) = rx_waker.as_mut() {
                    w.clone_from(cx.waker());
                } else {
                    rx_waker.replace(cx.waker().clone());
                }
                set_waker = true;
            };

            // We're ready. If we did set a waker we can remove it now.
            if set_waker {
                let mut rx_waker = self.rx_waker.lock().unwrap();
                rx_waker.take();
            }
            Poll::Ready(res)
        }).await
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.tx_waker.lock().unwrap().take().map(|w| w.wake());
    }
}
