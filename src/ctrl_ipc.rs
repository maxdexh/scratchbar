use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc as stdchan;
use std::time::Duration;

use anyhow::Context as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::utils::ResultExt as _;

pub(crate) const HOST_SOCK_PATH_VAR: &str = "BAR_INTERNAL_SOCK_PATH";

#[derive(Serialize, Deserialize)]
pub(crate) struct HostCtrlInit {
    pub version: String,
    pub opts: crate::host::HostConnectOpts,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct HostInitResponse {}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

enum StopStateInner<S> {
    Running { stop: S },
    Stopped { err: bool },
}
struct StopState<S> {
    shared: Arc<std::sync::Mutex<StopStateInner<S>>>,
}
impl<S> Clone for StopState<S> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}
impl<S> StopState<S> {
    fn new(stop: S) -> Self {
        Self {
            shared: Arc::new(std::sync::Mutex::new(StopStateInner::Running { stop })),
        }
    }
}

#[track_caller]
fn try_run<S: FnOnce(anyhow::Result<()>), I, IO>(
    mut io: IO,
    ready: Ready,
    init_tx: stdchan::SyncSender<I>,
    init: impl FnOnce(&mut IO) -> I,
    stop: StopState<S>,
    run: impl FnOnce(&mut IO) -> anyhow::Result<()>,
) {
    let ready_guard = ready.drop_guard();

    if init_tx.send(init(&mut io)).is_err() {
        log::error!("Failed to send initialization result through closed channel");
        return;
    }
    drop(init_tx);

    ready_guard.disable();
    if !ready.wait() {
        return;
    }

    let res = run(&mut io);
    let mut stop_state = stop.shared.lock().unwrap_or_else(|pe| pe.into_inner());
    match std::mem::replace(
        &mut *stop_state,
        StopStateInner::Stopped { err: res.is_err() },
    ) {
        StopStateInner::Running { stop } => {
            drop(stop_state);
            stop(res);
        }
        state @ StopStateInner::Stopped { err } => {
            *stop_state = state;
            drop(stop_state);
            if err {
                res.ok_or_log();
            } else {
                res.ok_or_debug();
            }
        }
    }
}

struct DropGuard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for DropGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}
impl<F: FnOnce()> DropGuard<F> {
    fn new(f: F) -> Self {
        Self(Some(f))
    }
    fn disable(mut self) {
        _ = self.0.take();
    }
}

#[derive(Clone)]
struct SharedSocket {
    socket: Arc<UnixStream>,
}
impl Drop for SharedSocket {
    fn drop(&mut self) {
        drop(socket_guard(self.socket.clone()));
    }
}

fn socket_guard(stream: Arc<UnixStream>) -> DropGuard<impl FnOnce()> {
    DropGuard::new(move || {
        stream
            .shutdown(std::net::Shutdown::Both)
            .context("Failed to shutdown socket")
            .ok_or_log();
    })
}

#[derive(Clone)]
struct Ready {
    state: Arc<std::sync::OnceLock<bool>>,
}
impl Ready {
    fn new() -> Self {
        Self {
            state: Arc::new(std::sync::OnceLock::new()),
        }
    }

    fn set_ready(&self) {
        _ = self.state.set(true);
    }

    #[must_use]
    fn wait(&self) -> bool {
        *self.state.wait()
    }

    fn drop_guard(&self) -> DropGuard<impl FnOnce()> {
        DropGuard::new(|| _ = self.state.set(false))
    }
}

#[cfg(feature = "__bin")]
pub(crate) fn connect_from_host<T>(
    socket: UnixStream,
    mk_response: impl FnOnce(HostCtrlInit) -> anyhow::Result<(HostInitResponse, T)>,
    upd_tx: impl FnMut(crate::host::HostUpdate) -> Option<()> + Send + 'static,
    on_stop: impl FnOnce(anyhow::Result<()>) + Send + 'static,
) -> anyhow::Result<(T, stdchan::Sender<crate::host::HostEvent>)> {
    let socket = Arc::new(socket);
    let sock_init_guard = socket_guard(socket.clone());

    let run_ready = Ready::new();
    let init_ready_guard = run_ready.drop_guard();

    let writer_sock = SharedSocket { socket };
    let writer_stop = StopState::new(on_stop);

    let (ev_tx, ev_rx) = stdchan::channel();
    let (init_res_tx, req_res_rx) = stdchan::sync_channel(0);

    let reader_sock = writer_sock.clone();
    let reader_stop = writer_stop.clone();
    let reader_ready = run_ready.clone();
    std::thread::spawn(move || {
        try_run(
            std::io::BufReader::new(&*reader_sock.socket),
            reader_ready,
            init_res_tx,
            |read| read_once(read).context("Failed to read host connection request"),
            reader_stop,
            |read| run_ipc_reader(read, upd_tx).context("Host update reader failed"),
        );
    });

    let req = req_res_rx
        .recv()
        .context("Failed to receive host connection request")??;

    let (resp, ret) = mk_response(req)?;

    let (resp_res_tx, resp_res_rx) = stdchan::sync_channel(0);
    let writer_ready = run_ready.clone();
    std::thread::spawn(move || {
        try_run(
            std::io::BufWriter::new(&*writer_sock.socket),
            writer_ready,
            resp_res_tx,
            |write| send_once(write, resp).context("Failed to send host connection response"),
            writer_stop,
            |write| run_ipc_writer(write, ev_rx).context("Host event writer failed"),
        );
    });
    () = resp_res_rx
        .recv_timeout(Duration::from_secs(5))
        .context("Failed to handle host connection response")??;

    run_ready.set_ready();
    sock_init_guard.disable();
    init_ready_guard.disable();

    Ok((ret, ev_tx))
}

pub(crate) fn connect_from_ctrl(
    init: HostCtrlInit,
    ev_tx: impl FnMut(crate::host::HostEvent) -> Option<()> + Send + 'static,
    on_stop: impl FnOnce(anyhow::Result<()>) + Send + 'static,
) -> anyhow::Result<(HostInitResponse, stdchan::Sender<crate::host::HostUpdate>)> {
    let sock_path = std::env::var_os(HOST_SOCK_PATH_VAR).context("Missing socket path env var")?;
    let socket =
        Arc::new(UnixStream::connect(sock_path).context("Failed to connect to controller socket")?);
    let sock_init_guard = socket_guard(socket.clone());

    let run_ready = Ready::new();
    let init_ready_guard = run_ready.drop_guard();

    let reader_sock = SharedSocket { socket };
    let reader_stop = StopState::new(on_stop);

    let (upd_tx, upd_rx) = stdchan::channel();

    let (req_res_tx, req_res_rx) = stdchan::sync_channel(0);

    let writer_sock = reader_sock.clone();
    let writer_stop = reader_stop.clone();
    let writer_ready = run_ready.clone();
    std::thread::spawn(move || {
        try_run(
            std::io::BufWriter::new(&*writer_sock.socket),
            writer_ready,
            req_res_tx,
            |write| send_once(write, init).context("Failed to send host connection response"),
            writer_stop,
            |write| run_ipc_writer(write, upd_rx).context("Host event writer failed"),
        );
    });

    () = req_res_rx
        .recv_timeout(Duration::from_secs(5))
        .context("Failed to handle connection request")??;

    let (resp_res_tx, resp_res_rx) = stdchan::sync_channel(0);
    let reader_ready = run_ready.clone();
    std::thread::spawn(move || {
        try_run(
            std::io::BufReader::new(&*reader_sock.socket),
            reader_ready,
            resp_res_tx,
            |read| read_once(read).context("Failed to read host connection request"),
            reader_stop,
            |read| run_ipc_reader(read, ev_tx).context("Host update reader failed"),
        );
    });

    let resp = resp_res_rx
        .recv_timeout(Duration::from_secs(10))
        .context("Failed to receive host connection response")??;

    run_ready.set_ready();
    sock_init_guard.disable();
    init_ready_guard.disable();

    Ok((resp, upd_tx))
}

fn send_once<IT: Serialize>(write: &mut impl Write, init: IT) -> anyhow::Result<()> {
    let init = postcard::to_stdvec_cobs(&init)?;
    write.write_all(&init)?;
    write.flush()?;
    Ok(())
}
fn read_once<IR: DeserializeOwned>(read: &mut impl BufRead) -> anyhow::Result<IR> {
    let mut init = Vec::new();
    read.read_until(0, &mut init)?;
    let init = postcard::from_bytes_cobs(&mut init)?;
    Ok(init)
}

fn run_ipc_reader<R: DeserializeOwned>(
    read: &mut impl BufRead,
    mut tx: impl FnMut(R) -> Option<()>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();

    while read.read_until(0, &mut buf)? > 0 {
        if let Some(val) = postcard::from_bytes_cobs(&mut buf)
            .context("Failed to deserialize")
            .ok_or_log()
            && tx(val).is_none()
        {
            break;
        }
        buf.clear();
    }
    Ok(())
}

fn run_ipc_writer<T: Serialize>(
    write: &mut impl Write,
    rx: stdchan::Receiver<T>,
) -> anyhow::Result<()> {
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
