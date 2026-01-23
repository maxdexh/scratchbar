use crate::data::{BasicDesktopState, BasicWorkspace};
use crate::utils::ResultExt;
use crate::utils::{ReloadRx, SharedEmit, WatchRx, watch_chan};
use anyhow::Context;
use hyprland::data::*;
use hyprland::shared::{HyprData, HyprDataVec};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_util::task::AbortOnDropHandle;

// TODO: Detailed state (for menu), including clients

pub struct HyprClient {
    pub basic_rx: WatchRx<BasicDesktopState>,
    _background: AbortOnDropHandle<()>,
}

async fn run_bg(basic_tx: impl SharedEmit<BasicDesktopState>, mut reload_rx: ReloadRx) {
    let ev_rx = hyprland::event_listener::EventStream::new()
        .filter_map(|res| res.context("Hyprland error").ok_or_log());
    tokio::pin!(ev_rx);

    let mut workspaces = Default::default();
    let mut monitors = HashMap::new();

    loop {
        type HyprEvent = hyprland::event_listener::Event;
        let (upd_wss, upd_mons) = tokio::select! {
            Some(()) = reload_rx.wait() => (true, true),
            Some(ev) = ev_rx.next() => match ev {
                HyprEvent::MonitorAdded(_) => (false, true),
                HyprEvent::ActiveMonitorChanged(_) => (false, true),
                HyprEvent::MonitorRemoved(_) => (false, true),
                HyprEvent::WorkspaceChanged(_) => (false, true),

                HyprEvent::WorkspaceMoved(_) => (true, false),
                HyprEvent::WorkspaceAdded(_) => (true, false),
                HyprEvent::WorkspaceDeleted(_) => (true, false),
                HyprEvent::WorkspaceRenamed(_) => (true, false),

                _ => continue,
            },
        };

        futures::future::join(
            async {
                if upd_wss {
                    let Ok(wss) = Workspaces::get_async()
                        .await
                        .map_err(|err| log::error!("Failed to fetch workspaces: {err}"))
                    else {
                        return;
                    };
                    workspaces = wss.to_vec();
                }
            },
            async {
                if upd_mons {
                    let Ok(mrs) = Monitors::get_async()
                        .await
                        .map_err(|err| log::error!("Failed to fetch monitors: {err}"))
                    else {
                        return;
                    };
                    for Monitor {
                        id,
                        name,
                        active_workspace,
                        ..
                    } in mrs
                    {
                        monitors
                            .entry(id)
                            .and_modify(|(mname, mactive)| {
                                *mactive = active_workspace.id;
                                if mname as &str != name.as_str() {
                                    *mname = Arc::<str>::from(name.as_str());
                                }
                            })
                            .or_insert_with(|| (name.into(), active_workspace.id));
                    }
                }
            },
        )
        .await;

        let mut workspaces: Vec<_> = workspaces
            .iter()
            .map(
                |Workspace {
                     id,
                     name,
                     monitor_id,
                     ..
                 }| {
                    let mon = monitor_id.and_then(|id| monitors.get(&id));
                    BasicWorkspace {
                        id: id.to_string().into(),
                        name: name.as_str().into(),
                        monitor: mon.as_ref().map(|(name, _)| name.clone()),
                        is_active: mon.is_some_and(|(_, active_id)| active_id == id),
                    }
                },
            )
            .collect();

        workspaces.sort_unstable_by(|w1, w2| w1.name.cmp(&w2.name));
        basic_tx.emit(BasicDesktopState { workspaces });
    }
}

pub fn connect(reload_rx: ReloadRx) -> HyprClient {
    let (basic_tx, basic_rx) = watch_chan(BasicDesktopState::default());
    HyprClient {
        _background: AbortOnDropHandle::new(tokio::spawn(run_bg(basic_tx, reload_rx))),
        basic_rx,
    }
}
