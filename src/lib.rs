pub extern crate log; // FIXME: Feature flag

macro_rules! warn_non_exhaustive {
    () => {
        "This hidden field is not part of the public API. It only serves to make it non_exhaustive while allowing struct update syntax."
    };
}
pub mod host;
pub mod tui;

mod host_ctrl_ipc;
mod logging;
mod utils;

#[cfg(feature = "__bin")]
mod bins;
#[cfg(feature = "__bin")]
pub use bins::__scratchbar_bin_main;
