#![allow(clippy::collapsible_if)]

mod controller2;
mod data;
mod logging;
mod modules;
mod monitors;
mod panels;
mod procs;
mod terminals;
mod tui;
mod utils;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    use crate::logging::{ProcKindForLogger, init_logger};
    use crate::utils::ResultExt as _;

    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some(crate::terminals::INTERNAL_ARG) => {
            let Some(term_id) = std::env::var(crate::terminals::TERM_ID_VAR).ok_or_log() else {
                return;
            };
            let term_id = crate::terminals::TermId::from_str(&term_id);
            log::info!("Term process {term_id:?} started");
            init_logger(ProcKindForLogger::Panel(term_id.clone()));
            crate::terminals::term_proc_main(term_id).await
        }
        None => {
            init_logger(ProcKindForLogger::Controller);

            controller2::main().await
        }
        _ => log::error!("Bad arguments"),
    }
}
