use std::{
    io::{self, BufRead as _, Write as _},
    sync::Arc,
};

use tokio_util::sync::CancellationToken;

use crate::utils::ResultExt as _;

pub fn read_cobs_blocking<R: serde::de::DeserializeOwned>(
    read: impl std::io::Read,
    tx: impl Fn(R) -> Option<()>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut read = io::BufReader::new(read);

    loop {
        if read.read_until(0, &mut buf)? == 0 {
            break;
        }
        let Some(val) = postcard::from_bytes_cobs(&mut buf).ok_or_log() else {
            continue;
        };
        if tx(val).is_none() {
            break;
        }
        buf.clear();
    }
    Ok(())
}
pub fn write_cobs_blocking<T: serde::Serialize>(
    write: impl std::io::Write,
    mut rx: impl FnMut() -> Option<T>,
) -> std::io::Result<()> {
    let mut write = std::io::BufWriter::new(write);

    while let Some(val) = rx() {
        let Some(buf) = postcard::to_stdvec_cobs(&val).ok_or_log() else {
            continue;
        };
        write.write_all(&buf).and_then(|()| write.flush())?;
    }
    Ok(())
}
pub async fn run_cobs_socket<T: serde::Serialize, R: serde::de::DeserializeOwned>(
    socket: std::os::unix::net::UnixStream,
    tx: impl Fn(R) -> Option<()> + Send + 'static,
    rx: impl FnMut() -> Option<T> + Send + 'static,
    on_disconnect: CancellationToken,
) {
    #[derive(Clone)]
    struct SharedSocket {
        inner: Arc<std::os::unix::net::UnixStream>,
        on_disconnect: CancellationToken,
    }
    impl Drop for SharedSocket {
        fn drop(&mut self) {
            self.inner.shutdown(std::net::Shutdown::Both).ok_or_log();
            self.on_disconnect.cancel();
        }
    }
    let socket = SharedSocket {
        inner: Arc::new(socket),
        on_disconnect,
    };

    fn handle_io_res(res: std::io::Result<()>, cancelled: &CancellationToken) {
        if cancelled.is_cancelled()
            && let Err(err) = &res
            && err.kind() == std::io::ErrorKind::BrokenPipe
        {
            return;
        }

        res.ok_or_log();
    }
    {
        let read = socket.clone();
        std::thread::spawn(move || {
            let res = read_cobs_blocking(&*read.inner, tx);
            handle_io_res(res, &read.on_disconnect);
            drop(read);
        });
    }
    {
        let write = socket.clone();
        std::thread::spawn(move || {
            let res = write_cobs_blocking(&*write.inner, rx);
            handle_io_res(res, &write.on_disconnect);
            drop(write);
        });
    }

    socket.on_disconnect.cancelled().await;
    drop(socket);
}
