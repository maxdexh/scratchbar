use serde::{Deserialize, Serialize};

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use tokio_util::{sync::CancellationToken, time::FutureExt};

use crate::{tui, utils::ResultExt as _};

#[derive(Debug, Serialize, Deserialize)]
pub struct BarTuiState {
    // FIXME: Use Option<Elem> to hide, start hidden
    pub by_monitor: HashMap<Arc<str>, tui::Elem>,
    pub fallback: tui::Elem,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerUpdate {
    BarMenu(BarMenuUpdate),
    BarTui(BarTuiState),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BarMenuUpdate {
    pub tag: tui::InteractTag,
    pub kind: tui::InteractKind,
    pub menu: Option<tui::OpenMenu>,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerEvent {
    Interact(TuiInteract),
    ReloadRequest,
}
#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TuiInteract {
    pub kind: tui::InteractKind,
    pub tag: tui::InteractTag,
}

#[doc(hidden)]
pub fn __main() -> std::process::ExitCode {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(crate::inst::INTERNAL_INST_ARG))
    {
        crate::inst::inst_main()
    } else {
        crate::controller::ctrl_main()
    }
    .unwrap_or(std::process::ExitCode::FAILURE)
}

pub struct Error(anyhow::Error);
const _: () = {
    use std::fmt;

    impl fmt::Debug for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            fmt::Debug::fmt(&self.0, f)
        }
    }
    impl fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            fmt::Debug::fmt(&self.0, f)
        }
    }
    impl std::error::Error for Error {}
};

pub async fn run_driver_connection(
    tx: impl Fn(ControllerEvent) -> Option<()> + 'static + Send,
    rx: impl AsyncFnMut() -> Option<ControllerUpdate> + 'static + Send,
) -> Result<(), Error> {
    let sock_path = std::env::var_os(crate::controller::CONTROLLER_SOCK_PATH_VAR)
        .context("Missing socket path env var")
        .map_err(Error)?;
    let socket = std::os::unix::net::UnixStream::connect(sock_path)
        .context("Failed to connect to controller socket")
        .map_err(Error)?;

    run_ipc_connection(socket, tx, rx).await.map_err(Error)
}

pub(crate) async fn run_ipc_connection<
    T: Serialize + Send + 'static,
    R: serde::de::DeserializeOwned + Send + 'static,
>(
    socket: std::os::unix::net::UnixStream,
    tx: impl Fn(R) -> Option<()> + 'static + Send,
    mut rx: impl AsyncFnMut() -> Option<T> + 'static + Send,
) -> anyhow::Result<()> {
    #[derive(Clone)]
    struct Shared {
        socket: Arc<std::os::unix::net::UnixStream>,
        err_slot: Arc<std::sync::Mutex<Option<anyhow::Error>>>,
        on_stop: CancellationToken,
    }
    impl Shared {
        fn set_err(&self, err: anyhow::Error) {
            let mut lock = crate::utils::lock_mutex(&self.err_slot);
            if lock.is_some() {
                drop(lock);
                log::error!("{err:?}");
                return;
            }
            *lock = Some(err);
        }
    }
    impl Drop for Shared {
        fn drop(&mut self) {
            if let Err(err) = self.socket.shutdown(std::net::Shutdown::Both) {
                log::error!("{err}");
            }
            self.on_stop.cancel();
        }
    }
    let shared = Shared {
        socket: Arc::new(socket),
        err_slot: Default::default(),
        on_stop: CancellationToken::new(),
    };

    let (writer_tx, writer_rx) = std::sync::mpsc::channel();
    let writer_shared = shared.clone();
    std::thread::spawn(move || {
        if let Err(err) = run_ipc_writer(&*writer_shared.socket, &writer_rx) {
            writer_shared.set_err(err);
        }
    });

    let reader_shared = shared.clone();
    std::thread::spawn(move || {
        if let Err(err) = run_ipc_reader(&*reader_shared.socket, tx) {
            reader_shared.set_err(err);
        }
    });

    while let Some(Some(val)) = rx().with_cancellation_token(&shared.on_stop).await
        && writer_tx.send(val).is_ok()
    {}

    match crate::utils::lock_mutex(&shared.err_slot).take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}
fn run_ipc_reader<R: serde::de::DeserializeOwned>(
    read: impl std::io::Read,
    mut tx: impl FnMut(R) -> Option<()>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let mut read = std::io::BufReader::new(read);

    loop {
        if std::io::BufRead::read_until(&mut read, 0, &mut buf)? == 0 {
            break;
        }

        let Some(val) = postcard::from_bytes_cobs(&mut buf)
            .context("Failed to deserialize")
            .ok_or_log()
        else {
            continue;
        };
        if tx(val).is_none() {
            break;
        }
        buf.clear();
    }
    Ok(())
}

fn run_ipc_writer<T: serde::Serialize>(
    write: impl std::io::Write,
    rx: &std::sync::mpsc::Receiver<T>,
) -> anyhow::Result<()> {
    use std::io::Write as _;

    let mut write = std::io::BufWriter::new(write);

    while let Ok(ready) = rx.recv() {
        let vals = std::iter::chain(
            std::iter::once(ready),
            std::iter::from_fn(|| rx.try_recv().ok()),
        );
        for val in vals {
            if let Some(buf) = postcard::to_stdvec_cobs(&val).ok_or_log() {
                write.write_all(&buf)?;
            }
        }
        write.flush()?;
    }
    Ok(())
}
