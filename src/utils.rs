use std::ops::ControlFlow;

use futures::Stream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[track_caller]
pub fn lossy_broadcast<T: Clone>(rx: broadcast::Receiver<T>) -> impl Stream<Item = T> {
    broadcast_stream(rx, |n| {
        log::warn!(
            "Lagged {n} items on lossy stream ({})",
            std::any::type_name::<T>()
        );
        None
    })
}
pub fn broadcast_stream<T: Clone>(
    mut rx: broadcast::Receiver<T>,
    mut on_lag: impl FnMut(u64) -> Option<T>,
) -> impl Stream<Item = T> {
    stream_from_fn(async move || {
        loop {
            match rx.recv().await {
                Ok(value) => break Some(value),
                Err(broadcast::error::RecvError::Closed) => break None,
                Err(broadcast::error::RecvError::Lagged(n)) => match on_lag(n) {
                    Some(item) => break Some(item),
                    None => continue,
                },
            }
        }
    })
}
pub fn stream_from_fn<T>(f: impl AsyncFnMut() -> Option<T>) -> impl Stream<Item = T> {
    tokio_stream::StreamExt::filter_map(
        futures::stream::unfold(f, |mut f| async move { Some((f().await, f)) }),
        |it| it,
    )
}

pub struct ReloadRx {
    // FIXME: tokio::sync::Notify
    rx: broadcast::Receiver<()>,
    //last_reload: Option<std::time::Instant>,
    //min_delay: std::time::Duration,
}
impl ReloadRx {
    // FIXME: Return Result
    pub fn blocking_wait(&mut self) -> Option<()> {
        match self.rx.blocking_recv() {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Some(()),
            _ => None,
        }
    }
    pub async fn wait(&mut self) -> Option<()> {
        // if we get Err(Lagged) or Ok, it means a reload request was issued.
        match self.rx.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Some(()),
            _ => None,
        }
    }
    pub fn into_stream(mut self) -> impl Stream<Item = ()> {
        stream_from_fn(async move || self.wait().await)
    }

    pub fn resubscribe(&self) -> Self {
        Self {
            rx: self.rx.resubscribe(),
        }
    }
}
#[derive(Clone)]
pub struct ReloadTx {
    tx: broadcast::Sender<()>,
}
impl ReloadTx {
    pub fn new() -> Self {
        Self {
            tx: broadcast::Sender::new(1),
        }
    }
    pub fn reload(&mut self) {
        _ = self.tx.send(());
    }
    pub fn subscribe(&self) -> ReloadRx {
        ReloadRx {
            rx: self.tx.subscribe(),
        }
    }
}

pub fn unb_chan<T>() -> (UnbTx<T>, UnbRx<T>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (tx, UnbRx::new(rx))
}

// FIXME: Return Result
pub trait Emit<T> {
    #[track_caller]
    fn emit(&mut self, val: T) -> ControlFlow<()>;
}
pub async fn dump_stream<T>(mut emit: impl Emit<T>, stream: impl Stream<Item = T>) {
    tokio::pin!(stream);
    while let Some(item) = tokio_stream::StreamExt::next(&mut stream).await {
        if emit.emit(item).is_break() {
            break;
        }
    }
}
impl<T, F: FnMut(T) -> ControlFlow<()>> Emit<T> for F {
    #[track_caller]
    fn emit(&mut self, val: T) -> ControlFlow<()> {
        self(val)
    }
}
#[track_caller]
fn handle_sender_res<E: std::fmt::Display>(res: Result<(), E>) -> ControlFlow<()> {
    match res {
        Ok(()) => ControlFlow::Continue(()),
        Err(err) => {
            log::warn!("Failed to send: {err}");
            ControlFlow::Break(())
        }
    }
}
impl<T> Emit<T> for tokio::sync::mpsc::UnboundedSender<T> {
    #[track_caller]
    fn emit(&mut self, val: T) -> ControlFlow<()> {
        handle_sender_res(self.send(val))
    }
}
impl<T> Emit<T> for std::sync::mpsc::Sender<T> {
    #[track_caller]
    fn emit(&mut self, val: T) -> ControlFlow<()> {
        handle_sender_res(self.send(val))
    }
}
pub trait SharedEmit<T>: Emit<T> + Clone + 'static + Send {}
impl<S: Emit<T> + Clone + 'static + Send, T> SharedEmit<T> for S {}

pub trait ResultExt {
    type Ok;
    #[track_caller]
    fn ok_or_log(self) -> Option<Self::Ok>;
}
impl<T, E: Into<anyhow::Error>> ResultExt for Result<T, E> {
    type Ok = T;
    #[track_caller]
    fn ok_or_log(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(err) => {
                log::error!("{:?}", err.into());
                None
            }
        }
    }
}

pub type WatchTx<T> = tokio::sync::watch::Sender<T>;
pub type WatchRx<T> = tokio::sync::watch::Receiver<T>;
pub type UnbTx<T> = tokio::sync::mpsc::UnboundedSender<T>;
pub type UnbRx<T> = tokio_stream::wrappers::UnboundedReceiverStream<T>;
pub fn watch_chan<T>(init: T) -> (WatchTx<T>, WatchRx<T>) {
    tokio::sync::watch::channel(init)
}
// FIXME: after changing to return result, use impl Emit
pub fn fused_watch_tx<T>(tx: WatchTx<T>) -> impl Emit<T> + Clone {
    move |x| handle_sender_res(tx.send(x))
}

pub struct CancelDropGuard {
    pub inner: CancellationToken,
}
impl Drop for CancelDropGuard {
    fn drop(&mut self) {
        self.inner.cancel();
    }
}
impl CancelDropGuard {
    pub fn new() -> Self {
        Self {
            inner: CancellationToken::new(),
        }
    }
    pub fn disarm(self) -> CancellationToken {
        let disarm = std::mem::ManuallyDrop::new(self);
        // SAFETY: `disarm` is not used after this, including by drop impls,
        // so this copy is effectively a move out of the field.
        unsafe { std::ptr::read(&disarm.inner) }
    }
}
impl From<CancellationToken> for CancelDropGuard {
    fn from(inner: CancellationToken) -> Self {
        //tokio::sync::watch::Receiver::changed;
        //tokio::sync::watch::Sender::send;
        Self { inner }
    }
}
