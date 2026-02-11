pub extern crate log; // FIXME: Feature flag

macro_rules! warn_non_exhaustive {
    () => {
        "This hidden field is not part of the public API. It only serves to make it non_exhaustive while allowing struct update syntax."
    };
}
pub mod api;
pub mod tui;

pub struct WrappedError(anyhow::Error);
impl std::fmt::Debug for WrappedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::fmt::Display for WrappedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::error::Error for WrappedError {}

mod driver_ipc;
mod logging;
mod utils;

#[cfg(feature = "__bin")]
mod controller;
#[cfg(feature = "__bin")]
mod inst;
#[cfg(feature = "__bin")]
mod monitors;

#[doc(hidden)]
#[cfg(feature = "__bin")]
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
