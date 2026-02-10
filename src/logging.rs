use std::sync::{LazyLock, OnceLock};

const COLOR_VAR: &str = "COLOR";

static PROC_NAME: OnceLock<String> = OnceLock::new();

fn format_log(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &log::Record,
) -> Result<(), std::io::Error> {
    struct Format {
        color: bool,
        pid: u32,
        proc_name: &'static str,
    }
    static FORMAT: LazyLock<Format> = LazyLock::new(|| Format {
        pid: std::process::id(),
        color: match std::env::var(COLOR_VAR).as_deref().unwrap_or("auto") {
            "never" | "no" | "off" | "false" => false,
            "always" | "yes" | "on" | "true" => true,
            _ => std::io::IsTerminal::is_terminal(&std::io::stderr()),
        },
        proc_name: PROC_NAME.get().map(|s| &**s).unwrap_or("UNKNOWN"),
    });
    let Format {
        color,
        pid,
        ref proc_name,
    } = *FORMAT;

    let line_display = record.line();
    let line_display = if let Some(line) = &line_display {
        format_args!("{}", *line)
    } else {
        format_args!("?")
    };

    let now_display = now.format("%Y-%m-%d %H:%M:%S");
    let now_display = if color {
        format_args!("\x1b[35m{now_display}\x1b[0m")
    } else {
        format_args!("{now_display}")
    };

    let level = record.level();

    let level_colored;
    let level_display = if color {
        level_colored = flexi_logger::style(level).paint(level.to_string());
        format_args!("{level_colored}")
    } else {
        format_args!("{level}")
    };

    write!(
        w,
        "[{now_display}] {proc_name} ({pid}) {level_display} [{}:{line_display}] {}",
        record.file().unwrap_or("<unknown>"),
        record.args(),
    )
}

pub(crate) fn init_logger(log_name: String) {
    _ = PROC_NAME.set(log_name.clone());
    match try_init_logger() {
        Ok(_) => log::info!("Started logger"),
        Err(err) => {
            let err = err.context(format!(
                "Failed to start logger {:?} (pid {})",
                log_name,
                std::process::id()
            ));
            log::error!("{err:?}");
            eprintln!("{err:?}");
        }
    }
}

fn try_init_logger() -> anyhow::Result<()> {
    use flexi_logger::*;

    // FIXME: Also use env var
    let log_spec: LogSpecification = if cfg!(debug_assertions) {
        flexi_logger::LevelFilter::Debug.into()
    } else {
        flexi_logger::LevelFilter::Info.into()
    };

    let logger_handle = Logger::with(log_spec)
        .format(format_log)
        .log_to_stderr()
        .start()?;
    std::mem::forget(logger_handle);

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("{info}");
        hook(info);
    }));

    Ok(())
}
