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

    let connect = scratchbar::host::connect(scratchbar::host::HostConnectOpts {
        ..Default::default()
    })
    .ok_or_log()?;

    Some(runtime.block_on(control::control_main(connect)))
}
