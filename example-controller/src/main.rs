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

    scratchbar::host::init_controller_logger();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;
    let _guard = runtime.enter();

    let (ctrl_upd_tx, mut ctrl_upd_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ctrl_ev_tx, ctrl_ev_rx) = tokio::sync::mpsc::unbounded_channel();

    let mut required_tasks = tokio::task::JoinSet::new();

    required_tasks.spawn(async move { Some(control::control_main(ctrl_upd_tx, ctrl_ev_rx).await) });
    required_tasks.spawn(async move {
        scratchbar::host::run_host_connection(
            move |ev| ctrl_ev_tx.send(ev).ok(),
            async move || ctrl_upd_rx.recv().await,
        )
        .await
        .context("Failed to connect to controller")
        .ok_or_log()?;

        Some(std::process::ExitCode::SUCCESS)
    });

    runtime.block_on(async move { required_tasks.join_next().await?.ok_or_log().flatten() })
}
