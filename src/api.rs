use serde::{Deserialize, Serialize};

use std::sync::Arc;

use anyhow::Context;

use crate::{
    tui,
    utils::{ResultExt as _, with_mutex_lock},
};

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerUpdate {
    UpdateBars(BarSelection, BarUpdate),
    SetDefaultTui(SetBarTui),
    RegisterMenu(RegisterMenu),
}

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub enum BarUpdate {
    SetTui(SetBarTui),
    Hide,
    Show,
}
impl From<SetBarTui> for BarUpdate {
    fn from(value: SetBarTui) -> Self {
        Self::SetTui(value)
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SetBarTui {
    pub tui: tui::Elem,
    pub options: SetBarTuiOpts,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SetBarTuiOpts {
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub enum BarSelection {
    All,
    OnMonitor { monitor_name: Arc<str> },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterMenu {
    pub on_tag: tui::InteractTag,
    pub on_kind: tui::InteractKind,
    pub tui: tui::Elem,
    pub menu_kind: MenuKind,
    pub options: RegisterMenuOpts,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RegisterMenuOpts {
    // TODO: Option on whether to apply update to already open tui
    // TODO: Option to set font size of menu / other options temporarily / run commands when menu is shown / hidden?
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MenuKind {
    Tooltip,
    Context,
}
#[cfg(feature = "__bin")]
impl MenuKind {
    pub(crate) fn internal_clone(&self) -> Self {
        match self {
            Self::Tooltip => Self::Tooltip,
            Self::Context => Self::Context,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerEvent {
    Interact(InteractEvent),
    // TODO: Add monitor change event
    // TODO: Menu closed
}
#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InteractEvent {
    pub kind: tui::InteractKind,
    pub tag: tui::InteractTag,
}

pub struct Error(anyhow::Error);
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::error::Error for Error {}

pub async fn run_driver_connection(
    tx: impl Fn(ControllerEvent) -> Option<()> + 'static + Send,
    rx: impl AsyncFnMut() -> Option<ControllerUpdate> + 'static + Send,
) -> Result<(), Error> {
    let sock_path = std::env::var_os(crate::driver_ipc::CONTROLLER_SOCK_PATH_VAR)
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
        abort_fut: futures::future::AbortHandle,
    }
    impl Shared {
        fn set_err(&self, err: anyhow::Error) {
            with_mutex_lock(&self.err_slot, |slot| {
                if slot.is_none() {
                    *slot = Some(err)
                }
            });
        }
    }
    impl Drop for Shared {
        fn drop(&mut self) {
            if let Err(err) = self.socket.shutdown(std::net::Shutdown::Both) {
                log::error!("{err}");
            }
            self.abort_fut.abort();
        }
    }

    let (abort_handle, abort_reg) = futures::future::AbortHandle::new_pair();
    let shared = Shared {
        socket: Arc::new(socket),
        err_slot: Default::default(),
        abort_fut: abort_handle,
    };

    let (writer_tx, writer_rx) = std::sync::mpsc::channel();

    let fut_shared = shared.clone();
    let fut = futures::future::Abortable::new(
        async move {
            let _guard = fut_shared;
            while let Some(val) = rx().await
                && writer_tx.send(val).is_ok()
            {}
        },
        abort_reg,
    );

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

    fut.await
        .map_err(|_| anyhow::anyhow!("controller connection aborted"))
        .ok_or_debug();

    with_mutex_lock(&shared.err_slot, |slot| match slot.take() {
        Some(err) => Err(err),
        None => Ok(()),
    })
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

pub fn init_driver_logger() {
    crate::logging::init_logger("DRIVER".into());
}
