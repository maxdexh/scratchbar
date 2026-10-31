mod clients;
mod control;
mod desktop;
mod utils;
mod xtui;

pub async fn run(
    update_tx: crate::host::HostUpdateSender,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::host::HostEvent>,
) -> std::process::ExitCode {
    control::control_main(update_tx, event_rx).await
}
