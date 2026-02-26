mod clients;
mod control;
mod desktop;
mod utils;
mod xtui;

fn main() -> std::process::ExitCode {
    main_inner().unwrap_or(std::process::ExitCode::FAILURE)
}

fn main_inner() -> Option<std::process::ExitCode> {
    use crate::utils::ResultExt as _;
    use anyhow::Context as _;
    use std::process::ExitCode;

    scratchbar::host::init_controller_logger();

    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::unbounded_channel();

    std::panic::set_hook(Box::new({
        let exit_tx_clone = exit_tx.clone();

        let hook = std::panic::take_hook();

        move |info| {
            hook(info);
            log::error!("{info}");
            exit_tx_clone.send(ExitCode::FAILURE).ok_or_debug();
        }
    }));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;

    let (ev_tx, ev_rx) = tokio::sync::mpsc::unbounded_channel();

    let exit_tx_clone = exit_tx.clone();
    let connect = scratchbar::host::connect(
        scratchbar::host::HostConnectOpts {
            ..Default::default()
        },
        move |ev| ev_tx.send(ev).map_err(|err| err.0),
        move |res| {
            exit_tx_clone
                .send(if res.ok_or_log().is_some() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
                .ok_or_log();
        },
    )
    .ok_or_log()?;

    let _guard = runtime.enter();

    runtime.spawn(async move {
        let code = control::control_main(connect, ev_rx).await;
        exit_tx.send(code).ok_or_log();
    });

    runtime.block_on(exit_rx.recv())
}
