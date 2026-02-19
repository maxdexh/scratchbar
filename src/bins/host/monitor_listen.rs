use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Context as _;
use tokio::sync::{mpsc::UnboundedSender, watch};
use tokio_util::sync::CancellationToken;

use crate::{bins::host::MonitorInfo, utils::ResultExt as _};

const NO_CHANGE_SLEEP: Duration = Duration::from_millis(1000);
const CHANGE_SLEEP: Duration = Duration::from_millis(500);

pub(super) async fn run_monitor_listener(
    bar_tui_states_tx: watch::Sender<super::BarTuiStates>,
    open_menu_rx: watch::Receiver<Option<crate::host::OpenMenu>>,
    event_tx: UnboundedSender<crate::host::HostEvent>,
) -> std::process::ExitCode {
    // TODO: Consider moving this to BarTuiStates to ensure consistent data
    let mut monitors_auto_cancel = HashMap::<Arc<str>, tokio_util::sync::DropGuard>::new();

    let mut state = MonitorState::default();
    loop {
        let old_state = {
            let Some(new_state) = MonitorState::fetch().await.take_if(|it| *it != state) else {
                tokio::time::sleep(NO_CHANGE_SLEEP).await;
                continue;
            };
            std::mem::replace(&mut state, new_state)
        };

        bar_tui_states_tx.send_modify(|bar_tui_states| {
            for monitor in old_state
                .mtrs
                .keys()
                .filter(|&it| !state.mtrs.contains_key(it))
            {
                drop(monitors_auto_cancel.remove(monitor));
                bar_tui_states.by_monitor.remove(monitor);
            }
            for monitor in state
                .mtrs
                .values()
                .filter(|&new| old_state.mtrs.get(&new.name).is_none_or(|old| old != new))
            {
                let bar_state_tx = bar_tui_states.get_or_mk_monitor(monitor.name.clone());

                let cancel = CancellationToken::new();
                tokio::spawn(super::monitor_inst::run_monitor(
                    super::monitor_inst::RunMonitorArgs {
                        monitor: monitor.clone(),
                        cancel_monitor: cancel.clone(),
                        bar_state_tx: bar_state_tx.clone(),
                        open_menu_rx: open_menu_rx.clone(),
                        event_tx: event_tx.clone(),
                    },
                ));
                monitors_auto_cancel.insert(monitor.name.clone(), cancel.drop_guard());
            }
        });

        tokio::time::sleep(CHANGE_SLEEP).await;
    }
}

#[derive(PartialEq, Clone, Debug, Default)]
struct MonitorState {
    mtrs: HashMap<Arc<str>, MonitorInfo>,
}
impl MonitorState {
    async fn fetch() -> Option<Self> {
        #[derive(serde::Deserialize)]
        struct MonitorData {
            name: Arc<str>,
            scale: f64,
            modes: Vec<MonitorMode>,
            enabled: bool,
        }
        #[derive(serde::Deserialize)]
        struct MonitorMode {
            width: u32,
            height: u32,
            current: bool,
        }

        let std::process::Output {
            status,
            stdout,
            stderr,
        } = tokio::process::Command::new("wlr-randr")
            .arg("--json")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .context("Failed to run wlr-randr --json")
            .ok_or_log()?;

        if !status.success() {
            log::error!(
                "wlr-randr --json exited with exit code {status}. Stderr: {}",
                String::from_utf8_lossy(&stderr),
            );
            return None;
        }

        let data = serde_json::from_slice::<Vec<MonitorData>>(&stdout)
            .context("Failed to deserialize output of wlr-randr --json")
            .ok_or_log()?;

        let monitors: HashMap<_, _> = data
            .into_iter()
            .filter(|md| md.enabled)
            .filter_map(|md| {
                let MonitorData {
                    name, scale, modes, ..
                } = md;
                let MonitorMode { width, height, .. } = modes.into_iter().find(|it| it.current)?;
                Some((
                    name.clone(),
                    MonitorInfo {
                        name,
                        scale,
                        width,
                        height,
                    },
                ))
            })
            .collect();

        Some(MonitorState { mtrs: monitors })
    }
}
