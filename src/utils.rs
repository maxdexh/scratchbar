use tokio_util::sync::CancellationToken;

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

pub struct CancelDropGuard {
    pub inner: CancellationToken,
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

pub fn with_mutex_lock<T, U>(mutex: &std::sync::Mutex<T>, f: impl FnOnce(&mut T) -> U) -> U {
    f(&mut mutex.lock().unwrap_or_else(|poison| poison.into_inner()))
}
