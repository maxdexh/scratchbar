use std::{borrow::Borrow, collections::HashMap, sync::Arc};

use futures::Stream;
use tokio::sync::broadcast;

use crate::data::Location;

pub fn rect_center(
    rect: ratatui::layout::Rect,
    (font_w, font_h): ratatui_image::FontSize,
) -> Location {
    let font_w = u32::from(font_w);
    let font_h = u32::from(font_h);
    Location {
        x: u32::from(rect.x) * font_w + u32::from(rect.width) * font_w / 2,
        y: u32::from(rect.y) * font_h + u32::from(rect.height) * font_h / 2,
    }
}

#[track_caller]
pub fn fused_lossy_stream<T: Clone>(rx: broadcast::Receiver<T>) -> impl Stream<Item = T> {
    let on_lag = |n| {
        log::warn!(
            "Lagged {n} items on lossy stream ({})",
            std::any::type_name::<T>()
        )
    };
    let base = futures::stream::unfold(rx, move |mut rx| async move {
        let t = rx.recv().await;
        match t {
            Ok(value) => Some((Some(value), rx)),
            Err(broadcast::error::RecvError::Closed) => None,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                on_lag(n);
                Some((None, rx))
            }
        }
    });
    tokio_stream::StreamExt::filter_map(base, |it| it)
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
    pub async fn wait(&mut self) {
        // if we get Err(Lagged) or Ok, it means a reload request was issued.
        while let Err(broadcast::error::RecvError::Closed) = self.rx.recv().await {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
    pub fn into_stream(self) -> impl Stream<Item = ()> {
        futures::stream::unfold(self, |mut this| async move {
            this.wait().await;
            Some(((), this))
        })
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
