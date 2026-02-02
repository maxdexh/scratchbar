extern crate bar_panel_controller as ctrl;

mod clients;
mod desktop;
mod runner;

fn main() -> std::process::ExitCode {
    main_inner().unwrap_or(std::process::ExitCode::FAILURE)
}
fn main_inner() -> Option<std::process::ExitCode> {
    use anyhow::Context as _;
    use ctrl::utils::ResultExt as _;

    ctrl::init_driver_logger();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;
    let _guard = runtime.enter();

    let sock_path = std::env::var_os("SOCK_PATH")
        .context("Missing socket path env var")
        .ok_or_log()?;
    let conn = runtime
        .block_on(tokio::net::UnixStream::connect(sock_path))
        .ok_or_log()?;

    let (read, write) = conn.into_split();

    let (ctrl_upd_tx, ctrl_upd_rx) = ctrl::utils::unb_chan();
    let (ctrl_ev_tx, ctrl_ev_rx) = ctrl::utils::unb_chan();

    let main_task = runtime.spawn(runner::main(ctrl_upd_tx, ctrl_ev_rx));

    runtime.spawn(ctrl::utils::read_cobs(
        tokio::io::BufReader::new(read),
        move |ev| {
            ctrl_ev_tx.send(ev).ok_or_debug();
        },
    ));
    runtime.spawn(ctrl::utils::write_cobs(write, ctrl_upd_rx));

    runtime.block_on(async move { main_task.await.ok_or_log() })
}
