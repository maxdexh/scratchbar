use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Context;
use futures::Stream;
use futures::StreamExt as _;
use system_tray::item::StatusNotifierItem;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tokio_util::task::AbortOnDropHandle;

use crate::utils::Callback;
use crate::utils::{ReloadRx, ResultExt, UnbTx, WatchRx, unb_chan, watch_chan};
use crate::utils::{ReloadTx, WatchTx};

#[derive(Debug, Default)]
pub struct TrayState {
    pub items: HashMap<Arc<str>, StatusNotifierItem>,
    pub menus: HashMap<Arc<str>, TrayMenuExt>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct TrayMenuExt {
    pub id: u32,
    pub menu_path: Option<Arc<str>>,
    pub submenus: Vec<system_tray::menu::MenuItem>,
}

pub type ClientCallback = Callback<Arc<system_tray::client::Client>, ()>;
pub struct TrayClient {
    pub state_rx: WatchRx<TrayState>,
    pub client_sched_tx: UnbTx<ClientCallback>,
    _background: AbortOnDropHandle<()>,
}
pub fn connect(reload_rx: ReloadRx) -> TrayClient {
    let (state_tx, state_rx) = watch_chan(Default::default());
    let (client_sched_tx, client_sched_rx) = unb_chan();
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
    state_tx: WatchTx<TrayState>,
    client_sched_rx: impl Stream<Item = ClientCallback> + Send + 'static,
    mut reload_rx: ReloadRx,
) {
    let client = loop {
        match system_tray::client::Client::new().await {
            Ok(it) => break it,
            res @ Err(_) => {
                res.context("Failed to connect to system tray").ok_or_log();

                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(90)) => (),
                    Some(()) = reload_rx.wait() => (),
                }
            }
        }
    };

    let mut tasks = JoinSet::<()>::new();

    {
        let mut event_reload_tx = ReloadTx::new();
        let event_reload_rx = event_reload_tx.subscribe();

        // Minimize lagged events
        let mut event_rx = client.subscribe();
        tasks.spawn(async move {
            loop {
                if let Err(broadcast::error::RecvError::Closed) = event_rx.recv().await {
                    return;
                }

                let debounce = tokio::time::sleep(Duration::from_millis(50));
                tokio::pin!(debounce);
                loop {
                    tokio::select! {
                        res = event_rx.recv() => {
                            if let Err(broadcast::error::RecvError::Closed) = res {
                                return;
                            }
                        }
                        () = &mut debounce => break,
                    }
                }

                event_reload_tx.reload();
            }
        });

        tasks.spawn(run_state_fetcher(
            client.items(),
            event_reload_rx,
            state_tx,
            reload_rx,
        ));
    }
    tasks.spawn(run_client_sched(client, client_sched_rx));

    if let Some(res @ Err(_)) = tasks.join_next().await {
        res.context("Systray module failed").ok_or_log();
    }
}
async fn run_state_fetcher(
    state_mutex: Arc<std::sync::Mutex<system_tray::data::BaseMap>>,
    mut event_reload_rx: ReloadRx,
    state_tx: WatchTx<TrayState>,
    mut reload_rx: ReloadRx,
) {
    let fetch_blocking = move || {
        let lock = state_mutex.lock().unwrap_or_else(|it| it.into_inner());

        let mut items = HashMap::with_capacity(lock.len());
        let mut menus = HashMap::with_capacity(lock.len());

        for (addr, (item, menu)) in &*lock {
            let addr = Arc::<str>::from(addr.as_str());
            if let Some(menu) = &menu {
                menus.insert(
                    addr.clone(),
                    TrayMenuExt {
                        id: menu.id,
                        menu_path: item.menu.as_deref().map(Into::into),
                        submenus: menu.submenus.clone(),
                    },
                );
            }
            items.insert(addr, item.clone());
        }
        (items, menus)
    };
    let fetch = || async {
        tokio::task::spawn_blocking(fetch_blocking.clone())
            .await
            .ok_or_log()
    };

    //let mut menu_paths = HashMap::new();

    loop {
        tokio::select! {
            Some(()) = event_reload_rx.wait() => {}
            Some(()) = reload_rx.wait() => {},
        }

        let Some((items, menus)) = fetch().await else {
            continue;
        };
        state_tx.send_replace(TrayState { items, menus });
    }
}
async fn run_client_sched(
    client: system_tray::client::Client,
    cb_rx: impl Stream<Item = ClientCallback>,
) {
    let client = Arc::new(client);
    tokio::pin!(cb_rx);
    while let Some(cb) = cb_rx.next().await {
        let client = client.clone();
        tokio::task::spawn_blocking(move || cb.call(client));
    }
    log::warn!("Tray interact stream was closed");
}
