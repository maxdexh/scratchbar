pub extern crate image;
pub extern crate log;

mod api;
mod controller;
mod inst;
mod logging;
mod monitors;
pub mod tui;
pub mod utils;

pub use api::*;

pub fn init_driver_logger() {
    logging::init_logger("DRIVER".into());
}
