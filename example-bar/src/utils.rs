// TODO: Add ability to pause updates for when the bar is hidden

use anyhow::Context as _;
use tokio::sync::watch;

#[derive(Clone)]
pub struct ReloadRx {
    rx: watch::Receiver<()>,
}
impl ReloadRx {
    #[must_use]
    pub async fn wait(&mut self) -> Option<()> {
        let opt = self.rx.changed().await.ok();
        self.rx.mark_unchanged();
        opt
    }
}
#[derive(Clone)]
pub struct ReloadTx {
    tx: watch::Sender<()>,
}
impl ReloadTx {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            tx: watch::Sender::new(()),
        }
    }
    pub fn reload(&mut self) {
        self.tx.send_replace(());
    }
    pub fn subscribe(&self) -> ReloadRx {
        ReloadRx {
            rx: self.tx.subscribe(),
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
