pub(crate) mod host;
pub(crate) mod inst;

#[doc(hidden)]
#[cfg(feature = "__bin")]
pub fn __scratchbar_bin_main() -> std::process::ExitCode {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(crate::bins::inst::INTERNAL_INST_ARG))
    {
        inst::inst_main()
    } else {
        host::host_main()
    }
}
