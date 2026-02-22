use std::process::ExitCode;

use anyhow::Context as _;
use tokio_util::time::FutureExt as _;

use crate::{ctrl_ipc, utils::ResultExt as _};

pub(super) fn host_main_inner() -> Option<ExitCode> {
    crate::logging::init_logger("HOST".into());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;

    let _guard = runtime.enter();

    // FIXME: Proper arg parsing
    let ctrl_cmd = std::env::args_os()
        .nth(1)
        .context("Missing controller command")
        .ok_or_log()?;

    let (mut ctrl_child, ctrl_socket) = {
        let socket_dir = tempfile::TempDir::new().ok_or_log()?;
        let sock_path = socket_dir.path().join("host.sock");
        let socket = std::os::unix::net::UnixListener::bind(&sock_path).ok_or_log()?;

        let child = tokio::process::Command::new(ctrl_cmd)
            .kill_on_drop(true)
            .args(std::env::args_os().skip(2))
            .env(ctrl_ipc::HOST_SOCK_PATH_VAR, sock_path)
            .spawn()
            .ok_or_log()?;

        let (conn, _) = socket.accept().ok_or_log()?;

        (child, conn)
    };

    let (ctrl_ipc::HostCtrlInit { version, opts }, event_tx, update_rx) =
        ctrl_ipc::connect_ipc(ctrl_socket, ctrl_ipc::HostInitResponse {}).ok_or_log()?;
    check_version(&version).ok_or_log()?;
    let crate::host::HostConnectOpts {
        #[expect(deprecated)]
            __non_exhaustive_struct_update: (),
    } = opts;

    let signals_task = runtime.spawn(async move {
        type SK = tokio::signal::unix::SignalKind;

        let mut tasks = tokio::task::JoinSet::new();

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        for kind in [
            SK::interrupt(),
            SK::quit(),
            SK::alarm(),
            SK::hangup(),
            SK::pipe(),
            SK::terminate(),
            SK::user_defined1(),
            SK::user_defined2(),
        ] {
            let Some(mut signal) = tokio::signal::unix::signal(kind).ok_or_log() else {
                continue;
            };
            let tx = tx.clone();
            tasks.spawn(async move {
                while let Some(()) = signal.recv().await
                    && tx.send(kind).await.is_ok()
                {}
            });
        }
        drop(tx);

        rx.recv()
            .await
            .context("Failed to receive any signals")
            .map(|kind| {
                log::debug!("Received exit signal {kind:?}");
                let code = 128 + kind.as_raw_value();
                ExitCode::from(code as u8)
            })
            .ok_or_log()
    });
    let signals_task = async move {
        signals_task
            .await
            .context("Signal handler failed")
            .ok_or_log()
            .flatten()
    };

    let mut update_rx = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        std::thread::spawn(move || {
            while let Ok(upd) = update_rx.recv()
                && tx.send(upd).is_ok()
            {}
        });
        rx
    };

    let main_task = tokio_util::task::AbortOnDropHandle::new(runtime.spawn(async move {
        super::run_host(
            futures::stream::poll_fn(move |cx| update_rx.poll_recv(cx)),
            event_tx,
        )
        .await;
        // FIXME: Return exit code
        ExitCode::SUCCESS
    }));

    let exit_task = runtime.spawn(async move {
        let wait_res = tokio::select! {
            it = ctrl_child.wait() => Ok(it),
            join = main_task => {
                let code = join
                    .context("Main task failed")
                    .ok_or_log()
                    .unwrap_or(std::process::ExitCode::FAILURE);
                Err(code)
            },
            Some(code) = signals_task => Err(code),
        };
        let (wait_res, code) = match wait_res {
            Ok(res) => (Some(res), ExitCode::SUCCESS),
            Err(code) => (
                ctrl_child
                    .wait()
                    .timeout(std::time::Duration::from_secs(5))
                    .await
                    .context("Controller process failed to exit on its own")
                    .ok_or_log(),
                code,
            ),
        };
        let child_code = match wait_res {
            Some(res) => res.ok_or_log().map_or(ExitCode::FAILURE, |exit| {
                ExitCode::from(exit.code().unwrap_or(0) as u8)
            }),
            None => ExitCode::FAILURE,
        };
        if code == ExitCode::SUCCESS {
            child_code
        } else {
            code
        }
    });

    runtime.block_on(async move { exit_task.await.ok_or_log() })
}

fn check_version(version: &str) -> anyhow::Result<()> {
    let this_ver = ctrl_ipc::VERSION;
    if version == this_ver {
        Ok(())
    } else {
        anyhow::bail!(
            "Cannot run controller built against version {version:?} under version {this_ver:?}"
        )
    }
}
