pub extern crate image;
pub extern crate log;

macro_rules! warn_non_exhaustive {
    () => {
        "This hidden field is not part of the public API. It only serves to make it non_exhaustive while allowing struct update syntax."
    };
}
pub mod api;
mod controller;
mod inst;
mod logging;
mod monitors;
pub mod tui;
mod utils;

pub fn init_driver_logger() {
    logging::init_logger("DRIVER".into());
}

#[doc(hidden)]
pub fn __main() -> std::process::ExitCode {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(crate::inst::INTERNAL_INST_ARG))
    {
        crate::inst::inst_main()
    } else {
        crate::controller::ctrl_main()
    }
    .unwrap_or(std::process::ExitCode::FAILURE)
}
