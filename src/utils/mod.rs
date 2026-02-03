use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

mod reload;
pub use reload::*;
mod channels;
pub use channels::*;
mod dbg;
pub use dbg::*;
mod io;
pub use io::*;

pub trait ResultExt {
    type Ok;
    fn ok_or_log(self) -> Option<Self::Ok>;
    fn ok_or_debug(self) -> Option<Self::Ok>;
}

impl<T, E: Into<anyhow::Error>> ResultExt for Result<T, E> {
    type Ok = T;
    #[track_caller]
    #[inline]
    fn ok_or_log(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(err) => {
                log::error!("{:?}", err.into());
                None
            }
        }
    }

    #[track_caller]
    #[inline]
    fn ok_or_debug(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(err) => {
                log::debug!("{:?}", err.into());
                None
            }
        }
    }
}

pub async fn run_or_retry<T, E, A>(
    mut f: impl AsyncFnMut(&mut A) -> Result<T, E>,
    mut args: A,
    mut ctx: impl FnMut(Result<T, E>) -> anyhow::Result<T>,
    timeout: std::time::Duration,
    mut reload_rx: Option<&mut ReloadRx>,
) -> T {
    loop {
        let res = ctx(f(&mut args).await)
            .with_context(|| format!("Failed to run task. Retrying in {}s", timeout.as_secs()))
            .ok_or_log();

        if let Some(init) = res {
            return init;
        }

        // TODO: Once we have a way to manually reload, add exponential backoff
        tokio::select! {
            () = tokio::time::sleep(timeout) => {}
            Some(()) = async {
                let reload_rx = reload_rx.as_deref_mut()?;
                reload_rx.wait().await
            } => {}
        }
    }
}

pub struct CancelDropGuard {
    pub inner: CancellationToken,
}
impl CancelDropGuard {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        CancellationToken::new().into()
    }
}
impl Drop for CancelDropGuard {
    fn drop(&mut self) {
        self.inner.cancel();
    }
}
impl From<CancellationToken> for CancelDropGuard {
    fn from(inner: CancellationToken) -> Self {
        //tokio::sync::watch::Receiver::changed;
        //tokio::sync::watch::Sender::send;
        Self { inner }
    }
}

pub fn lock_mutex<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub async fn read_cobs<T: serde::de::DeserializeOwned>(
    mut read: impl tokio::io::AsyncBufRead + std::marker::Unpin,
    tx: impl Fn(T),
) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt as _;
    loop {
        let mut buf = Vec::new();
        if read.read_until(0, &mut buf).await? == 0 {
            break;
        }

        match postcard::from_bytes_cobs(&mut buf) {
            Err(err) => {
                log::error!(
                    "Failed to deserialize {} from socket: {err}",
                    std::any::type_name::<T>()
                );
            }
            Ok(ev) => {
                tx(ev);
            }
        }

        buf.clear();
    }
    Ok(())
}

pub async fn write_cobs<T: serde::Serialize>(
    mut write: impl tokio::io::AsyncWrite + std::marker::Unpin,
    mut items: impl futures::Stream<Item = T> + std::marker::Unpin,
) -> anyhow::Result<()> {
    let ret = async {
        use futures::StreamExt as _;
        use tokio::io::AsyncWriteExt as _;
        while let Some(item) = items.next().await {
            let Ok(buf) = postcard::to_stdvec_cobs(&item)
                .map_err(|err| log::error!("Failed to serialize update: {err}"))
            else {
                continue;
            };

            write.write_all(&buf).await?;
        }
        Ok(())
    }
    .await;
    write.shutdown().await.ok_or_log();
    ret
}
