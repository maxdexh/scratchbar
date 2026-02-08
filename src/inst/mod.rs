mod ipc;

use crate::tui;
use crate::utils::{CancelDropGuard, ResultExt as _};

use std::ffi::OsString;
use std::process::ExitCode;
use std::{ffi::OsStr, path::Path, time::Duration};

use anyhow::Context as _;
use futures::{Stream, StreamExt as _};
use tokio::task::JoinSet;
use tokio_util::{sync::CancellationToken, time::FutureExt as _};

pub use ipc::{TermEvent, TermUpdate};

pub async fn start_generic_panel(
    sock_path: &Path,
    log_name: &str,
    upd_rx: impl Stream<Item = TermUpdate> + 'static + Send,
    extra_args: impl IntoIterator<Item: AsRef<OsStr>>,
    extra_envs: impl IntoIterator<Item = (OsString, OsString)>,
    term_ev_tx: tokio::sync::mpsc::UnboundedSender<TermEvent>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let socket = tokio::net::UnixListener::bind(sock_path)?;

    let mut child = tokio::process::Command::new("kitten")
        .arg("panel")
        .args(extra_args)
        .arg(std::env::current_exe().context("Failed to get current executable")?)
        .arg(INTERNAL_INST_ARG)
        .envs(extra_envs)
        .env(ipc::SOCK_PATH_VAR, sock_path)
        .env(ipc::PROC_LOG_NAME_VAR, log_name)
        .kill_on_drop(true)
        .stdout(std::io::stderr())
        .spawn()
        .context("Failed to spawn terminal")?;

    let (socket, _) = socket
        .accept()
        .await
        .context("Failed to accept socket connection")?;

    tokio::spawn(async move {
        let mut mgr = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(run_term_inst_mgr(
            socket,
            term_ev_tx,
            upd_rx,
            cancel.clone(),
        )));
        tokio::select! {
            exit_res = child.wait() => {
                exit_res.context("Failed to wait for terminal exit").ok_or_log();
            }
            () = cancel.cancelled() => {}
            run_res = &mut mgr => {
                run_res
                    .context("Terminal instance failed")
                    .ok_or_log();
            }
        };
        cancel.cancel();
        drop(mgr);

        // Child should exit by itself because the socket connection is closed.
        let child_res = child.wait().timeout(Duration::from_secs(10)).await;

        match (|| anyhow::Ok(child_res??))()
            .context("Terminal instance failed to exit after shutdown")
            .ok_or_log()
        {
            Some(status) => {
                if !status.success() {
                    log::error!("Terminal exited with nonzero status {status}");
                }
            }
            None => {
                child
                    .kill()
                    .await
                    .context("Failed to kill terminal")
                    .ok_or_log();
            }
        }
    });

    Ok(())
}
async fn run_term_inst_mgr(
    connection: tokio::net::UnixStream,
    ev_tx: tokio::sync::mpsc::UnboundedSender<TermEvent>,
    updates: impl Stream<Item = TermUpdate> + Send + 'static,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let _auto_cancel = CancelDropGuard::from(cancel.clone());
    let mut tasks = JoinSet::<()>::new();
    // TODO: Await stream

    let (read_half, write_half) = connection.into_split();

    tasks.spawn(ipc::read_cobs_sock::<TermEvent>(
        read_half,
        move |x| {
            ev_tx.send(x).ok_or_debug();
        },
        cancel.clone(),
    ));
    tasks.spawn(ipc::write_cobs_sock::<TermUpdate>(
        write_half,
        updates,
        cancel.clone(),
    ));

    if let Some(Err(err)) = tasks.join_next().await {
        log::error!("Error with task: {err}");
    }
    cancel.cancel();
    tasks.join_all().await;

    Ok(())
}

const KITTY_FWD_VAR: &str = "KITTY_STDIO_FORWARDED";
fn fwd_log_self_exe() -> Option<std::process::Command> {
    let fwd_raw = std::env::var_os(KITTY_FWD_VAR).take_if(|it| !it.is_empty())?;

    let mut cmd = std::process::Command::new(std::env::current_exe().ok_or_log()?);
    cmd.env(KITTY_FWD_VAR, "");

    let fwd_str = fwd_raw
        .into_string()
        .map_err(|s| anyhow::anyhow!("Expected {KITTY_FWD_VAR} to be fd, got {s:?}"))
        .ok_or_log()?;
    let fwd_fd = fwd_str
        .parse::<std::os::fd::RawFd>()
        .with_context(|| format!("{KITTY_FWD_VAR} is not a valid fd"))
        .ok_or_log()?;

    // SAFETY: This is not io safe, but there is no other way to do this
    // (other than /proc/self/fd/fdnum, which just bypasses the unsafe keyword)
    let fwd = unsafe { <std::process::Stdio as std::os::fd::FromRawFd>::from_raw_fd(fwd_fd) };
    cmd.stderr(fwd);

    cmd.args(std::env::args().skip(1));
    Some(cmd)
}

pub const INTERNAL_INST_ARG: &str = "--internal-inst";
pub fn inst_main() -> Option<ExitCode> {
    let (log_name, res) =
        match std::env::var(ipc::PROC_LOG_NAME_VAR).context("Bad log name env var") {
            Ok(name) => (name, Ok(())),
            Err(err) => ("UNKNOWN".into(), Err(err)),
        };

    crate::logging::init_logger(log_name);

    if let Some(mut cmd) = fwd_log_self_exe() {
        let err = anyhow::Error::from(std::os::unix::process::CommandExt::exec(&mut cmd))
            .context("Failed to replace current process");
        log::error!("{:?}", err);
    }

    res.ok_or_log();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;

    let main_handle = runtime.spawn(term_proc_main_inner());
    let main_res = runtime
        .block_on(main_handle)
        .context("Failed to join main")
        .flatten();

    Some(match main_res.ok_or_log() {
        Some(()) => ExitCode::SUCCESS,
        None => ExitCode::FAILURE,
    })
}

async fn term_proc_main_inner() -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    let cancel = CancellationToken::new();
    let (ev_tx, upd_rx);
    {
        let socket = std::env::var_os(ipc::SOCK_PATH_VAR).context("Missing socket path env var")?;
        let socket = tokio::net::UnixStream::connect(socket)
            .await
            .context("Failed to connect to socket")?;
        let (read, write) = socket.into_split();

        let (upd_tx, mut ev_rx);
        (ev_tx, ev_rx) = {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            (tx, rx)
        };
        (upd_tx, upd_rx) = std::sync::mpsc::channel::<TermUpdate>();

        tasks.spawn(ipc::read_cobs_sock(
            read,
            move |x| {
                upd_tx.send(x).ok_or_debug();
            },
            cancel.clone(),
        ));
        tasks.spawn(ipc::write_cobs_sock(
            write,
            futures::stream::poll_fn(move |cx| ev_rx.poll_recv(cx)),
            cancel.clone(),
        ));
    }

    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::cursor::Hide,
        crossterm::event::EnableMouseCapture,
    )?;
    crossterm::terminal::enable_raw_mode()?;

    let Some(init_sizes) = tui::Sizes::query()? else {
        anyhow::bail!("Terminal reported window size of 0. Do not start as hidden!");
    };

    ev_tx
        .send(TermEvent::Sizes(init_sizes))
        .context("Failed to send initial font size while starting panel. Exiting.")?;

    tasks.spawn(async move {
        let events = crossterm::event::EventStream::new()
            .filter_map(async |res| res.context("Crossterm error").ok_or_log());
        tokio::pin!(events);
        while let Some(ev) = events.next().await {
            if let crossterm::event::Event::Resize(_, _) = &ev
                && let Some(sizes) = tui::Sizes::query().ok_or_log()
            {
                if let Some(sizes) = sizes {
                    ev_tx.send(TermEvent::Sizes(sizes)).ok_or_debug();
                } else {
                    log::debug!(
                        "Terminal reported window size of 0 (this is expected if the terminal is hidden)"
                    );
                }
            }
            ev_tx.send(TermEvent::Crossterm(ev)).ok_or_debug();
        }
    });

    fn run_cmd(cmd: &mut std::process::Command) {
        if let Err(err) = (|| {
            let std::process::Output {
                status,
                stdout: _,
                stderr,
            } = cmd.output()?;

            if !status.success() {
                anyhow::bail!(
                    "Exited with status {status}. Stderr:\n{}",
                    String::from_utf8_lossy(&stderr)
                );
            }
            Ok(())
        })() {
            log::error!("Failed to run command {cmd:?}: {err}")
        }
    }

    let cancel_blocking = cancel.clone();
    std::thread::spawn(move || {
        let auto_cancel = CancelDropGuard::from(cancel_blocking);
        use std::io::Write as _;
        let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
        while !auto_cancel.inner.is_cancelled()
            && let Ok(upd) = upd_rx.recv()
        {
            match upd {
                TermUpdate::Print(bytes) => {
                    stdout
                        .write_all(&bytes)
                        .context("Failed to print")
                        .ok_or_log();
                }
                TermUpdate::Flush => {
                    stdout.flush().context("Failed to flush").ok_or_log();
                }
                TermUpdate::RemoteControl(args) => {
                    let Some(listen_on) = std::env::var_os("KITTY_LISTEN_ON")
                        .context("Missing KITTY_LISTEN_ON")
                        .ok_or_log()
                    else {
                        continue;
                    };
                    run_cmd(
                        std::process::Command::new("kitten")
                            .arg("@")
                            .arg("--to")
                            .arg(listen_on)
                            .args(args),
                    );
                }
                TermUpdate::Shell(cmd, args) => {
                    run_cmd(std::process::Command::new(cmd).args(args));
                }
            }
        }
    });

    tokio::select! {
        Some(res) = tasks.join_next() => {
            res.ok_or_log();
        }
        () = cancel.cancelled() => {}
    }

    Ok(())
}
