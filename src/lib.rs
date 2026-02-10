pub extern crate image; // FIXME: Remove
pub extern crate log; // FIXME: Feature flag

macro_rules! warn_non_exhaustive {
    () => {
        "This hidden field is not part of the public API. It only serves to make it non_exhaustive while allowing struct update syntax."
    };
}
pub mod api;
pub mod tui;

mod controller;
mod inst;
mod logging;
mod monitors;
mod utils;

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
