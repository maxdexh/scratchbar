pub extern crate log;

extern crate self as scratchbar;

macro_rules! warn_non_exhaustive {
    () => {
        "This hidden field is not part of the public API. It only serves to make it non_exhaustive while allowing struct update syntax."
    };
}

pub mod host;
pub mod tui;

mod controller;
mod logging;
mod utils;

pub mod bins;
