use std::sync::Mutex;
use std::{sync::Arc, time::Duration};

use anyhow::Context;
use system_tray::data::BaseMap;
use system_tray::item::StatusNotifierItem;
use system_tray::menu::TrayMenu;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;
use tokio_util::task::AbortOnDropHandle;

use crate::utils::{ReloadRx, ReloadTx, ResultExt, run_or_retry};

#[derive(Debug)]
pub struct TrayEntry {
    pub addr: Arc<str>,
    pub item: StatusNotifierItem,
    pub menu: Option<TrayMenuExt>,
}

#[derive(Debug, Default, Clone)]
pub struct TrayState {
    pub entries: Arc<[TrayEntry]>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct TrayMenuExt {
    pub id: u32,
    pub menu_path: Option<Arc<str>>,
    pub submenus: Vec<system_tray::menu::MenuItem>,
}

type ClientCallback = Box<dyn FnOnce(Arc<system_tray::client::Client>) + Send + 'static>;
#[derive(Debug)]
pub struct TrayClient {
    pub state_rx: watch::Receiver<TrayState>,
    client_sched_tx: tokio::sync::mpsc::UnboundedSender<ClientCallback>,
    _background: AbortOnDropHandle<()>,
}
impl TrayClient {
    #[track_caller]
    pub fn sched_with_client<Fut: Future<Output = ()> + Send + 'static>(
        &self,
        f: impl FnOnce(Arc<system_tray::client::Client>) -> Fut + Send + 'static,
    ) {
        self.client_sched_tx
            .send(Box::new(|client| {
                tokio::spawn(f(client));
            }))
            .map_err(|e| anyhow::anyhow!("{e}"))
            .ok_or_debug();
    }
}
pub fn connect(reload_rx: ReloadRx) -> TrayClient {
    let (state_tx, state_rx) = watch::channel(Default::default());
    let (client_sched_tx, client_sched_rx) = tokio::sync::mpsc::unbounded_channel();
    TrayClient {
        _background: AbortOnDropHandle::new(tokio::spawn(run_bg(
            state_tx,
            client_sched_rx,
            reload_rx,
        ))),
        state_rx,
        client_sched_tx,
    }
}
async fn run_bg(
    state_tx: watch::Sender<TrayState>,
    client_sched_rx: tokio::sync::mpsc::UnboundedReceiver<ClientCallback>,
    mut reload_rx: ReloadRx,
) {
    let client = run_or_retry(
        |_: &mut ()| system_tray::client::Client::new(),
        (),
        |res| res.context("Failed to initialize tray client"),
        Duration::from_secs(90),
        Some(&mut reload_rx),
    )
    .await;

    let mut tasks = JoinSet::<()>::new();

    let event_reload_tx = ReloadTx::new();
    let event_reload_rx = event_reload_tx.subscribe();

    tasks.spawn(events_to_reloads(event_reload_tx, client.subscribe()));

    tasks.spawn(run_state_fetcher(
        client.items(),
        event_reload_rx,
        state_tx,
        reload_rx,
    ));

    tasks.spawn(run_client_sched(client, client_sched_rx));

    if let Some(res @ Err(_)) = tasks.join_next().await {
        res.context("Systray module failed").ok_or_log();
    }
}

async fn events_to_reloads(mut tx: ReloadTx, mut rx: broadcast::Receiver<impl Clone>) {
    loop {
        if let Err(broadcast::error::RecvError::Closed) = rx.recv().await {
            return;
        }

        let debounce = tokio::time::sleep(Duration::from_millis(50));
        tokio::pin!(debounce);
        loop {
            tokio::select! {
                res = rx.recv() => {
                    if let Err(broadcast::error::RecvError::Closed) = res {
                        return;
                    }
                }
                () = &mut debounce => break,
            }
        }

        tx.reload();
    }
}

async fn run_state_fetcher(
    state_mutex: Arc<Mutex<BaseMap>>,
    mut event_reload_rx: ReloadRx,
    state_tx: watch::Sender<TrayState>,
    mut reload_rx: ReloadRx,
) {
    let fetch_blocking = move || {
        let lock = state_mutex.lock().unwrap_or_else(|it| it.into_inner());

        lock.iter()
            .map(|(addr, (item, menu))| {
                let menu = menu
                    .as_ref()
                    .map(|&TrayMenu { id, ref submenus }| TrayMenuExt {
                        id,
                        menu_path: item.menu.as_deref().map(Into::into),
                        submenus: submenus.clone(),
                    });
                TrayEntry {
                    addr: addr.as_str().into(),
                    item: item.clone(),
                    menu,
                }
            })
            .collect()
    };

    loop {
        tokio::select! {
            Some(()) = event_reload_rx.wait() => {}
            Some(()) = reload_rx.wait() => {},
        }

        let task = tokio::task::spawn_blocking(fetch_blocking.clone());
        let Some(entries) = task.await.ok_or_log() else {
            continue;
        };
        state_tx.send_replace(TrayState { entries });
    }
}
async fn run_client_sched(
    client: system_tray::client::Client,
    mut cb_rx: tokio::sync::mpsc::UnboundedReceiver<ClientCallback>,
) {
    let client = Arc::new(client);
    while let Some(cb) = cb_rx.recv().await {
        cb(client.clone());
    }
    log::warn!("Tray interact stream was closed");
}
