mod entry_point;
mod monitor_inst;
mod monitor_listen;

use std::{collections::HashMap, sync::Arc};

use futures::{Stream, StreamExt};
use tokio::sync::{mpsc::UnboundedSender, watch};

use crate::{host, tui, utils::ResultExt};

pub(crate) fn host_main() -> std::process::ExitCode {
    entry_point::host_main_inner().unwrap_or(std::process::ExitCode::FAILURE)
}

// TODO: Consider stopping/starting the monitor instance instead of having this hide system
#[derive(Debug, Clone)]
struct BarTuiState {
    tui: tui::Elem,
    hidden: bool,
}
#[derive(Debug, Clone)]
struct BarTuiStateSender {
    tui: watch::Sender<tui::Elem>,
    hidden: watch::Sender<bool>,
}
#[derive(Debug)]
struct BarTuiStates {
    by_monitor: HashMap<Arc<str>, watch::Sender<BarTuiStateSender>>,
    defaults: BarTuiStateSender,
}
impl BarTuiStates {
    fn get_or_mk_monitor(&mut self, name: Arc<str>) -> &mut watch::Sender<BarTuiStateSender> {
        self.by_monitor
            .entry(name)
            .or_insert_with(|| watch::Sender::new(self.defaults.clone()))
    }
}
#[derive(PartialEq, Clone, Debug)]
pub(crate) struct MonitorInfo {
    pub name: Arc<str>,
    pub scale: f64,
    pub width: u32,
    pub height: u32,
}

async fn run_host(
    update_rx: impl Stream<Item = host::HostUpdate> + Send + 'static,
    event_tx: UnboundedSender<host::HostEvent>,
) -> std::process::ExitCode {
    let mut required_tasks = tokio::task::JoinSet::<std::process::ExitCode>::new();

    let bar_tui_states_tx = watch::Sender::new(BarTuiStates {
        by_monitor: Default::default(),
        defaults: BarTuiStateSender {
            tui: watch::Sender::new(tui::Elem::empty()),
            hidden: watch::Sender::new(false),
        },
    });

    let open_menu_tx = watch::Sender::new(None);
    required_tasks.spawn(monitor_listen::run_monitor_listener(
        bar_tui_states_tx.clone(),
        open_menu_tx.subscribe(),
        event_tx.clone(),
    ));
    required_tasks.spawn(run_update_handler(
        update_rx,
        open_menu_tx,
        bar_tui_states_tx,
    ));

    if let Some(res) = required_tasks.join_next().await {
        res.ok_or_log().unwrap_or(std::process::ExitCode::FAILURE)
    } else {
        unreachable!()
    }
}

async fn run_update_handler(
    update_rx: impl Stream<Item = host::HostUpdate> + Send + 'static,
    open_menu_tx: watch::Sender<Option<host::OpenMenu>>,
    bar_tui_states_tx: watch::Sender<BarTuiStates>,
) -> std::process::ExitCode {
    tokio::pin!(update_rx);
    while let Some(update) = update_rx.next().await {
        match update {
            host::HostUpdate::UpdateBars(host::BarSelect::All, update) => {
                fn doit<T>(
                    bar_tui_states: &mut BarTuiStates,
                    val: T,
                    get_tx: impl Fn(&mut BarTuiStateSender) -> &mut watch::Sender<T>,
                ) {
                    let default_tx = get_tx(&mut bar_tui_states.defaults);
                    default_tx.send_replace(val);
                    for state in bar_tui_states.by_monitor.values_mut() {
                        state.send_modify(|it| *get_tx(it) = default_tx.clone());
                    }
                }
                bar_tui_states_tx.send_modify(|bar_tui_states| {
                    // TODO: Keep unknown monitors around only for a few minutes
                    match update {
                        host::BarUpdate::SetTui(host::SetBarTui {
                            tui,
                            options:
                                host::SetBarTuiOpts {
                                    #[expect(deprecated)]
                                        __non_exhaustive_struct_update: (),
                                },
                        }) => {
                            doit(bar_tui_states, tui, |state| &mut state.tui);
                        }
                        host::BarUpdate::Hide | host::BarUpdate::Show => {
                            doit(
                                bar_tui_states,
                                matches!(update, host::BarUpdate::Hide),
                                |state| &mut state.hidden,
                            );
                        }
                    }
                });
            }
            host::HostUpdate::UpdateBars(host::BarSelect::OnMonitor { monitor_name }, update) => {
                fn doit<T>(
                    bar_tui_states: &mut BarTuiStates,
                    monitor: Arc<str>,
                    val: T,
                    get_tx: impl Fn(&mut BarTuiStateSender) -> &mut watch::Sender<T>,
                ) {
                    let default_tx = get_tx(&mut bar_tui_states.defaults).clone();
                    bar_tui_states
                        .get_or_mk_monitor(monitor.clone())
                        .send_if_modified(|state| {
                            let tx = get_tx(state);
                            if tx.same_channel(&default_tx) {
                                *tx = watch::Sender::new(val);
                                true
                            } else {
                                tx.send_replace(val);
                                false
                            }
                        });
                }
                bar_tui_states_tx.send_modify(|bar_tui_states| {
                    // TODO: Keep unknown monitors around only for a few minutes
                    match update {
                        host::BarUpdate::SetTui(host::SetBarTui {
                            tui,
                            options:
                                host::SetBarTuiOpts {
                                    #[expect(deprecated)]
                                        __non_exhaustive_struct_update: (),
                                },
                        }) => {
                            doit(bar_tui_states, monitor_name, tui, |state| &mut state.tui);
                        }
                        host::BarUpdate::Hide | host::BarUpdate::Show => {
                            doit(
                                bar_tui_states,
                                monitor_name,
                                matches!(update, host::BarUpdate::Hide),
                                |state| &mut state.hidden,
                            );
                        }
                    }
                });
            }
            host::HostUpdate::SetDefaultTui(host::SetBarTui {
                tui,
                options:
                    host::SetBarTuiOpts {
                        #[expect(deprecated)]
                            __non_exhaustive_struct_update: (),
                    },
            }) => {
                bar_tui_states_tx.borrow().defaults.tui.send_replace(tui);
            }
            host::HostUpdate::OpenMenu(open) => {
                open_menu_tx.send_replace(Some(open));
            }
            host::HostUpdate::CloseMenu => {
                open_menu_tx.send_replace(None);
            }
        }
    }

    std::process::ExitCode::SUCCESS
}
