use crate::data::{ActiveMonitorInfo, BasicDesktopState, BasicMonitor, BasicWorkspace};
use crate::utils::{ReloadRx, fused_lossy_stream};
use futures::Stream;
use hyprland::data::*;
use hyprland::shared::HyprData;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

// TODO: Detailed state (for menu), including clients
// TODO: channel to receive full refetch request
pub fn connect(
    reload_rx: ReloadRx,
) -> (
    impl Stream<Item = BasicDesktopState>,
    impl Stream<Item = Option<ActiveMonitorInfo>>,
) {
    type HyprEvent = hyprland::event_listener::Event;
    let ev_tx = broadcast::Sender::new(100);

    let events = fused_lossy_stream(ev_tx.subscribe());
    let (ws_tx, ws_rx) = broadcast::channel(10);
    let (am_tx, am_rx) = broadcast::channel(10);
    tokio::spawn(async move {
        tokio::pin!(events);

        let mut workspaces = Default::default();
        let mut monitors = Default::default();
        let mut active_monitor = Default::default();

        let update_wss = async |workspaces: &mut _| {
            let Ok(wss) = Workspaces::get_async()
                .await
                .map_err(|err| log::error!("Failed to fetch workspaces: {err}"))
            else {
                return;
            };

            *workspaces = wss
                .into_iter()
                .map(
                    |Workspace {
                         id,
                         name,
                         monitor_id,
                         ..
                     }| BasicWorkspace {
                        id: id.to_string().into(),
                        name: name.into(),
                        monitor: monitor_id.map(|id| id.to_string().into()),
                    },
                )
                .collect();

            // This should always succeed
            if let Some(slice) = Arc::get_mut(workspaces) {
                slice.sort_by(|w1, w2| w1.name.cmp(&w2.name));
            }
        };

        let update_mons = async |monitors: &mut _, active_monitor: &mut _| {
            let Ok(mrs) = Monitors::get_async()
                .await
                .map_err(|err| log::error!("Failed to fetch monitors: {err}"))
            else {
                return;
            };

            let active = mrs.iter().find(|mon| mon.focused);
            *active_monitor = active.map(|mon| mon.id.to_string().into());
            if let Err(err) = am_tx.send(active.map(
                |&Monitor {
                     width,
                     height,
                     scale,
                     ref name,
                     ..
                 }| ActiveMonitorInfo {
                    width: width.into(),
                    height: height.into(),
                    scale: scale.into(),
                    name: name.as_str().into(),
                },
            )) {
                log::error!("Failed to send active monitor information: {err}")
            }
            *monitors = mrs
                .into_iter()
                .map(|mr| BasicMonitor {
                    active_workspace: mr.active_workspace.id.to_string().into(),
                    name: mr.name.into(),
                })
                .collect();
        };

        let send_update = |workspaces: &_, monitors: &_, active_monitor: &_| {
            if let Err(err) = ws_tx.send(BasicDesktopState {
                workspaces: Arc::clone(workspaces),
                monitors: Arc::clone(monitors),
                active_monitor: Option::clone(active_monitor),
            }) {
                log::warn!("Hyprland state channel closed: {err}");
            }
        };

        // TODO: Test if this is correct. If there is no event that needs to update both,
        // divide them into two seperate tasks.
        let changes = events.map(|ev| match ev {
            HyprEvent::MonitorAdded(_) => (false, true),
            HyprEvent::ActiveMonitorChanged(_) => (false, true),
            HyprEvent::MonitorRemoved(_) => (false, true),
            HyprEvent::WorkspaceChanged(_) => (false, true),

            HyprEvent::WorkspaceMoved(_) => (true, false),
            HyprEvent::WorkspaceAdded(_) => (true, false),
            HyprEvent::WorkspaceDeleted(_) => (true, false),
            HyprEvent::WorkspaceRenamed(_) => (true, false),

            _ => (false, false),
        });

        let changes = reload_rx.into_stream().map(|_| (true, true)).merge(changes);
        tokio::pin!(changes);
        while let Some((upd_wss, upd_mons)) = changes.next().await {
            if !upd_wss && !upd_mons {
                continue;
            }

            futures::future::join(
                futures::future::OptionFuture::from(upd_wss.then(|| update_wss(&mut workspaces))),
                futures::future::OptionFuture::from(
                    upd_mons.then(|| update_mons(&mut monitors, &mut active_monitor)),
                ),
            )
            .await;
            send_update(&workspaces, &monitors, &active_monitor);
        }
    });

    tokio::spawn(async move {
        let mut hypr_events = hyprland::event_listener::EventStream::new();
        while let Some(event) = hypr_events.next().await {
            match event {
                Ok(ev) => {
                    if ev_tx.send(ev).is_err() {
                        log::warn!("Hyprland event channel closed");
                        return;
                    }
                }
                Err(err) => log::error!("Error with hypr event: {err}"),
            }
        }
        log::warn!("Hyprland event stream closed");
    });

    (fused_lossy_stream(ws_rx), fused_lossy_stream(am_rx))
}
