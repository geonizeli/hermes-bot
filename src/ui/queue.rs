//! Hands work from the tokio runtime back to an entity on the UI thread.
//!
//! Network work runs on `backend::runtime()`; when it's done it queues a
//! closure that runs with the entity and its context, on the UI thread.

use gpui_kit::Context;
use tokio::sync::mpsc;

pub type Call<T> = Box<dyn FnOnce(&mut T, &mut Context<T>) + Send>;

pub struct Queue<T: 'static>(mpsc::UnboundedSender<Call<T>>);

impl<T: 'static> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Queue(self.0.clone())
    }
}

impl<T: 'static> Queue<T> {
    /// A queue that drains onto the entity `cx` belongs to, for as long as
    /// the entity lives.
    pub fn new(cx: &mut Context<T>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Call<T>>();
        #[cfg(not(test))]
        cx.spawn(async move |this, cx| {
            while let Some(call) = rx.recv().await {
                if this.update(cx, |state, cx| call(state, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        // GPUI's test scheduler rejects wake-ups from other threads, so tests
        // poll on its clock instead of being woken by the network thread.
        #[cfg(test)]
        cx.spawn(async move |this, cx| {
            loop {
                while let Ok(call) = rx.try_recv() {
                    if this.update(cx, |state, cx| call(state, cx)).is_err() {
                        return;
                    }
                }
                cx.background_executor().timer(std::time::Duration::from_millis(10)).await;
            }
        })
        .detach();
        Queue(tx)
    }

    /// Runs `f` on the UI thread. False when the entity is gone.
    pub fn queue(&self, f: impl FnOnce(&mut T, &mut Context<T>) + Send + 'static) -> bool {
        self.0.send(Box::new(f)).is_ok()
    }
}
