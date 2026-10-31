use std::process::ExitCode;

use anyhow::Context as _;

use crate::utils::ResultExt as _;

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

    let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel();

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

    let exit_tx_clone = exit_tx.clone();
    runtime.spawn(async move {
        let code =
            crate::controller::run(crate::host::HostUpdateSender { tx: update_tx }, event_rx).await;
        exit_tx_clone.send(code).ok_or_debug();
    });

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

    runtime.block_on(exit_rx.recv())
}
