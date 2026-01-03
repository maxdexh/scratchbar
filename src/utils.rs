use std::{borrow::Borrow, collections::HashMap, sync::Arc};

use futures::Stream;
use tokio::sync::broadcast;

#[track_caller]
pub fn broadcast_stream<T: Clone>(rx: broadcast::Receiver<T>) -> impl Stream<Item = T> {
    broadcast_stream_base(rx, |n| {
        log::warn!(
            "Lagged {n} items on lossy stream ({})",
            std::any::type_name::<T>()
        );
        None
    })
}
pub fn broadcast_stream_base<T: Clone>(
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

struct BasicTaskMapInner<K, T> {
    tasks: HashMap<K, tokio::task::JoinHandle<T>>,
}
impl<K, T> Default for BasicTaskMapInner<K, T> {
    fn default() -> Self {
        Self {
            tasks: Default::default(),
        }
    }
}
impl<K, T> Drop for BasicTaskMapInner<K, T> {
    fn drop(&mut self) {
        for (_, handle) in self.tasks.drain() {
            handle.abort();
        }
    }
}
pub struct BasicTaskMap<K, T> {
    inner: Arc<std::sync::Mutex<BasicTaskMapInner<K, T>>>,
}
impl<K, T> Clone for BasicTaskMap<K, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl<K, T> BasicTaskMap<K, T> {
    pub fn new() -> Self {
        Self {
            inner: Default::default(),
        }
    }

    pub fn insert_spawn<Fut>(&self, key: K, fut: Fut)
    where
        K: std::hash::Hash + Eq + Send + 'static + Clone,
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let weak = Arc::downgrade(&self.inner);

        let key_clone = key.clone();
        let handle = tokio::spawn(async move {
            let res = fut.await;
            if let Some(tasks) = weak.upgrade()
                && let Some(id) = tokio::task::try_id()
            {
                let mut inner = tasks.lock().unwrap_or_else(|poi| poi.into_inner());
                if inner.tasks.get(&key_clone).is_some_and(|it| it.id() == id) {
                    inner.tasks.remove(&key_clone);
                }
            }
            res
        });

        let mut inner = self.inner.lock().unwrap_or_else(|poi| poi.into_inner());
        if let Some(handle) = inner.tasks.insert(key, handle) {
            handle.abort();
        }
    }

    pub fn cancel<Q>(&self, key: &Q)
    where
        K: Borrow<Q> + std::hash::Hash + Eq,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        let mut inner = self.inner.lock().unwrap_or_else(|poi| poi.into_inner());
        if let Some(handle) = inner.tasks.remove(key) {
            handle.abort();
        }
    }
}

pub struct ReloadRx {
    rx: broadcast::Receiver<()>,
}
impl ReloadRx {
    pub async fn wait(&mut self) -> Option<()> {
        // if we get Err(Lagged) or Ok, it means a reload request was issued.
        match self.rx.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Some(()),
            _ => None,
        }
    }
    pub fn into_stream(self) -> impl Stream<Item = ()> {
        broadcast_stream_base(self.rx, |_| Some(()))
    }

    // TODO: debounce/rate limit this heavily: after accepting a reload request, merge all
    // requests from the next few seconds into one.
    // TODO: Cause extra reloads from time to time.
    pub fn new() -> (impl Clone + Fn(), Self) {
        let (tx, rx) = broadcast::channel(1);
        (
            move || {
                _ = tx.send(());
            },
            Self { rx },
        )
    }

    pub fn resubscribe(&self) -> Self {
        Self {
            rx: self.rx.resubscribe(),
        }
    }
}
