use tokio_util::sync::CancellationToken;

mod reload;
pub use reload::*;
mod channels;
pub use channels::*;

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

pub struct CancelDropGuard {
    pub inner: CancellationToken,
}
impl CancelDropGuard {
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
