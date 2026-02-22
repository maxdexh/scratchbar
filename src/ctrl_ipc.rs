use std::{sync::Arc, time::Duration};

use anyhow::Context as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::utils::ResultExt as _;

pub(crate) const HOST_SOCK_PATH_VAR: &str = "BAR_INTERNAL_SOCK_PATH";

#[derive(Serialize, Deserialize)]
pub(crate) struct HostCtrlInit {
    pub version: String,
    pub opts: crate::host::HostConnectOpts,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct HostInitResponse {}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

const INIT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn connect_ipc<
    IT: Serialize + Send + 'static,
    IR: DeserializeOwned + Send + 'static,
    T: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
>(
    socket: std::os::unix::net::UnixStream,
    init: IT,
) -> anyhow::Result<(IR, std::sync::mpsc::Sender<T>, std::sync::mpsc::Receiver<R>)> {
    #[derive(Clone)]
    struct Shared {
        socket: Arc<std::os::unix::net::UnixStream>,
    }
    impl Drop for Shared {
        fn drop(&mut self) {
            if let Err(err) = self.socket.shutdown(std::net::Shutdown::Both) {
                log::error!("{err}");
            }
        }
    }
    let shared = Shared {
        socket: Arc::new(socket),
    };

    let (r_tx, r_rx) = std::sync::mpsc::channel();
    let (t_tx, t_rx) = std::sync::mpsc::channel();
    let (i_tx, i_rx) = std::sync::mpsc::channel();

    let writer_shared = shared.clone();
    std::thread::spawn(move || {
        use std::io::Write as _;
        let mut write = std::io::BufWriter::new(&*writer_shared.socket);
        (|| {
            let init = postcard::to_stdvec_cobs(&init).ok_or_log()?;
            write.write(&init).ok_or_log()?;
            write.flush().ok_or_log()?;

            run_ipc_writer(&mut write, t_rx).ok_or_log()?;

            Some(())
        })();
    });

    let reader_shared = shared;
    std::thread::spawn(move || {
        let mut read = std::io::BufReader::new(&*reader_shared.socket);
        (|| {
            i_tx.send({
                let mut init = Vec::new();
                std::io::BufRead::read_until(&mut read, 0, &mut init).ok_or_log()?;
                postcard::from_bytes_cobs(&mut init).ok_or_log()?
            })
            .map_err(|_| anyhow::Error::msg("Failed to send initial"))
            .ok_or_log()?;
            drop(i_tx);

            run_ipc_reader(&mut read, r_tx).ok_or_log()?;

            Some(())
        })();
    });

    let recv_init = i_rx
        .recv_timeout(INIT_TIMEOUT)
        .context("Failed to receive initial")?;

    Ok((recv_init, t_tx, r_rx))
}

fn run_ipc_reader<R: DeserializeOwned>(
    read: &mut impl std::io::BufRead,
    tx: std::sync::mpsc::Sender<R>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();

    loop {
        if read.read_until(0, &mut buf)? == 0 {
            break;
        }

        let Some(val) = postcard::from_bytes_cobs(&mut buf)
            .context("Failed to deserialize")
            .ok_or_log()
        else {
            continue;
        };
        if tx.send(val).is_err() {
            break;
        }
        buf.clear();
    }
    Ok(())
}

fn run_ipc_writer<T: Serialize>(
    write: &mut impl std::io::Write,
    rx: std::sync::mpsc::Receiver<T>,
) -> anyhow::Result<()> {
    while let Ok(ready) = rx.recv() {
        let vals = std::iter::chain(
            std::iter::once(ready),
            std::iter::from_fn(|| rx.try_recv().ok()),
        );
        for val in vals {
            if let Some(buf) = postcard::to_stdvec_cobs(&val).ok_or_log() {
                write.write_all(&buf)?;
            }
        }
        write.flush()?;
    }
    Ok(())
}
