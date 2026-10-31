pub(crate) trait ResultExt {
    type Ok;
    fn ok_or_log(self) -> Option<Self::Ok>;
    fn ok_or_debug(self) -> Option<Self::Ok>;
}

impl<T, E: Into<anyhow::Error>> ResultExt for Result<T, E> {
    type Ok = T;
    #[track_caller]
    #[inline]
    fn ok_or_log(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(err) => {
                log::error!("{:?}", err.into());
                None
            }
        }
    }

    #[track_caller]
    #[inline]
    fn ok_or_debug(self) -> Option<T> {
        match self {
            Ok(val) => Some(val),
            Err(err) => {
                log::debug!("{:?}", err.into());
                None
            }
        }
    }
}

pub fn set_process_name(mut name: String) {
    if name.as_bytes().contains(&0) {
        log::error!("Failed to set process name: Illegal nul bytes in '{name}'");
        return;
    }
    if name.len() > 15 {
        log::warn!("Process name '{name}' longer than 15 bytes may be truncated.");
    }
    name.push('\0');

    // SAFETY: Args are correct, `pname` is nul-terminated
    let result = unsafe { libc::prctl(libc::PR_SET_NAME, name.as_ptr(), 0, 0, 0) };

    if result == -1 {
        name.pop();
        log::error!(
            "{:#}",
            anyhow::Error::from(std::io::Error::last_os_error())
                .context(format!("Failed to set process name to '{name}'",))
        )
    }
}
