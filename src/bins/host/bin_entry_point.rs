use std::process::ExitCode;

use anyhow::Context as _;
use tokio_util::time::FutureExt as _;

use crate::{ctrl_ipc, utils::ResultExt as _};

pub(super) fn host_main_inner() -> Option<ExitCode> {
    crate::logging::init_logger("HOST".into());

    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::unbounded_channel();

    {
        let exit_tx_clone = exit_tx.clone();

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            hook(info);
            log::error!("{info}");
            exit_tx_clone.send(ExitCode::FAILURE).ok_or_debug();
        }));
    }

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

    let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel();

    let exit_tx_clone = exit_tx.clone();
    let (opts, event_tx) = ctrl_ipc::connect_from_host(
        ctrl_socket,
        |init| {
            let ctrl_ipc::HostCtrlInit { version, opts } = init;
            check_version(&version)?;
            Ok((ctrl_ipc::HostInitResponse {}, opts))
        },
        move |upd| update_tx.send(upd).ok(),
        move |res| {
            exit_tx_clone
                .send(if res.ok_or_log().is_some() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
                .ok_or_debug();
        },
    )
    .ok_or_log()?;

    let crate::host::HostConnectOpts {
        #[expect(deprecated)]
            __non_exhaustive_struct_update: (),
    } = opts;

    {
        type SK = tokio::signal::unix::SignalKind;

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
            let exit_tx = exit_tx.clone();
            runtime.spawn(async move {
                if let Some(()) = signal.recv().await {
                    let code = kind.as_raw_value().wrapping_add(128);
                    exit_tx.send(ExitCode::from(code as u8)).ok_or_debug();
                }
            });
        }
    }

    let exit_tx_clone = exit_tx.clone();
    runtime.spawn(async move {
        let code = super::run_host(
            futures::stream::poll_fn(move |cx| update_rx.poll_recv(cx)),
            event_tx,
        )
        .await;

        exit_tx_clone.send(code).ok_or_debug();
    });

    let exit_task = runtime.spawn(async move {
        let host_code;
        let ctrl_status;
        tokio::select! {
            res = ctrl_child.wait() => {
                host_code = ExitCode::SUCCESS;
                ctrl_status = res.ok_or_log();
            },
            Some(code) = exit_rx.recv() => {
                host_code = code;
                let res = ctrl_child
                    .wait()
                    .timeout(std::time::Duration::from_secs(5))
                    .await
                    .context("Controller failed to exit on its own")
                    .ok_or_log();

                if let Some(res) = res {
                    ctrl_status = res.ok_or_log();
                } else {
                    ctrl_status = None;

                    if ctrl_child.start_kill().context("Failed to kill controller").ok_or_log().is_some() {
                       ctrl_child.wait().await.ok_or_log();
                    }
                }
            },
        };
        let ctrl_code = ctrl_status.map_or(ExitCode::FAILURE, |status| {
            ExitCode::from(status.code().unwrap_or(0) as u8)
        });

        // Prefer the controller's code if it did not exit correctly
        if ctrl_code != ExitCode::SUCCESS {
            ctrl_code
        } else {
            host_code
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
