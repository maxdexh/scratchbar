mod energy;
mod hypr;
mod pulse;
mod time;
mod tray;

use std::{collections::HashMap, sync::Arc};

use crate::utils::{ReloadRx, ReloadTx, ResultExt as _};
use ctrl::{api, tui};
use tokio::{sync::watch, task::JoinSet};

use crate::clients;

#[derive(Clone, Debug)]
enum BarTuiElem {
    ByMonitor(HashMap<Arc<str>, tui::Elem>),
    Shared(tui::Elem),
    Hide,
    FillSpace(u16),
    Spacing(u16),
}
impl From<tui::Elem> for BarTuiElem {
    fn from(value: tui::Elem) -> Self {
        Self::Shared(value)
    }
}

struct InteractTagRegistry<K, V> {
    key_to_tag: HashMap<K, (tui::InteractTag, V)>,
    tag_to_key: HashMap<tui::InteractTag, K>,
}

fn mk_fresh_interact_tag() -> tui::InteractTag {
    use std::sync::atomic::*;

    static TAG_COUNTER: AtomicU64 = AtomicU64::new(0);
    tui::InteractTag::from_bytes(&TAG_COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes())
}

impl<K: std::hash::Hash + std::cmp::Eq + Clone, V> InteractTagRegistry<K, V> {
    fn new() -> Self {
        Self {
            key_to_tag: Default::default(),
            tag_to_key: Default::default(),
        }
    }
    fn get_or_init(
        &mut self,
        key: &K,
        init: impl FnOnce(&tui::InteractTag) -> V,
    ) -> (&tui::InteractTag, &mut V) {
        let (tag, val) = self.key_to_tag.entry(key.clone()).or_insert_with(|| {
            let tag = mk_fresh_interact_tag();
            self.tag_to_key.insert(tag.clone(), key.clone());
            let val = init(&tag);
            (tag, val)
        });
        (tag, val)
    }
}

struct InteractArgs {
    kind: tui::InteractKind,
}
type InteractCallback = Arc<dyn Fn(InteractArgs) + Send + Sync + 'static>;
fn interact_callback_with<C: Send + Sync + 'static>(
    ctx: C,
    f: impl Fn(&C, InteractArgs) + Send + Sync + 'static,
) -> InteractCallback {
    Arc::new(move |args| f(&ctx, args))
}
type RegTagCallback = (tui::InteractTag, Option<InteractCallback>);

#[derive(Debug, Clone)]
struct ModuleControllerTx {
    tx: tokio::sync::mpsc::UnboundedSender<api::ControllerUpdate>,
}
impl ModuleControllerTx {
    fn set_menu(&self, menu: api::RegisterMenu) {
        self.tx
            .send(api::ControllerUpdate::RegisterMenu(menu))
            .ok_or_debug();
    }
}

struct ModuleArgs {
    tui_tx: watch::Sender<BarTuiElem>,
    reload_rx: ReloadRx,
    ctrl_tx: ModuleControllerTx,
    tag_callback_tx: tokio::sync::mpsc::UnboundedSender<RegTagCallback>,
    _unused: (),
}

struct BarModuleFactory {
    reload_tx: ReloadTx,
    ctrl_tx: ModuleControllerTx,
    tag_callback_tx: tokio::sync::mpsc::UnboundedSender<RegTagCallback>,
    tasks: JoinSet<()>,
}
impl BarModuleFactory {
    fn spawn<F: Future<Output = ()> + 'static + Send>(
        &mut self,
        task: impl FnOnce(ModuleArgs) -> F,
    ) -> watch::Receiver<BarTuiElem> {
        let (tui_tx, tui_rx) = watch::channel(BarTuiElem::Hide);
        self.tasks.spawn(task(ModuleArgs {
            reload_rx: self.reload_tx.subscribe(),
            ctrl_tx: self.ctrl_tx.clone(),
            tag_callback_tx: self.tag_callback_tx.clone(),
            tui_tx,
            _unused: (),
        }));
        tui_rx
    }
    fn spawn_with<F: Future<Output = ()> + 'static + Send, C>(
        &mut self,
        ctx: C,
        task: impl FnOnce(C, ModuleArgs) -> F,
    ) -> watch::Receiver<BarTuiElem> {
        self.spawn(|args| task(ctx, args))
    }
    fn fixed(&mut self, elem: BarTuiElem) -> watch::Receiver<BarTuiElem> {
        let (_, rx) = watch::channel(elem);
        rx
    }
}

fn send_bar_tui(
    bar_tui: &[BarTuiElem],
    ctrl_tx: &tokio::sync::mpsc::UnboundedSender<api::ControllerUpdate>,
) {
    let mut by_monitor = HashMap::new();
    let mut fallback = tui::StackBuilder::new(tui::Axis::X);
    for elem in bar_tui {
        match elem {
            BarTuiElem::Shared(elem) => {
                for stack in by_monitor.values_mut().chain(Some(&mut fallback)) {
                    stack.fit(elem.clone());
                }
            }
            BarTuiElem::ByMonitor(elems) => {
                for (mtr, elem) in elems {
                    by_monitor
                        .entry(mtr.clone())
                        .or_insert_with(|| fallback.clone())
                        .fit(elem.clone());
                }
            }
            BarTuiElem::Hide => {}
            BarTuiElem::FillSpace(weight) => {
                for stack in by_monitor.values_mut().chain(Some(&mut fallback)) {
                    stack.fill(*weight, tui::Elem::empty());
                }
            }
            BarTuiElem::Spacing(len) => {
                for stack in by_monitor.values_mut().chain(Some(&mut fallback)) {
                    stack.spacing(*len);
                }
            }
        };
    }

    ctrl_tx
        .send(api::ControllerUpdate::SetDefaultTui(api::SetBarTui {
            tui: fallback.build(),
            options: Default::default(),
        }))
        .ok_or_debug();

    for (monitor, tui) in by_monitor {
        ctrl_tx
            .send(api::ControllerUpdate::UpdateBars(
                api::BarSelection::OnMonitor {
                    monitor_name: monitor,
                },
                api::SetBarTui {
                    tui: tui.build(),
                    options: Default::default(),
                }
                .into(),
            ))
            .ok_or_debug();
    }
}

pub async fn driver_main(
    ctrl_upd_tx: tokio::sync::mpsc::UnboundedSender<api::ControllerUpdate>,
    mut ctrl_ev_rx: tokio::sync::mpsc::UnboundedReceiver<api::ControllerEvent>,
) -> std::process::ExitCode {
    let mut required_tasks = JoinSet::new();
    let mut reload_tx = ReloadTx::new();

    let (tag_callback_tx, mut tag_callback_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut fac = BarModuleFactory {
        reload_tx: reload_tx.clone(),
        ctrl_tx: ModuleControllerTx {
            tx: ctrl_upd_tx.clone(),
        },
        tag_callback_tx,
        tasks: JoinSet::new(),
    };

    let callbacks = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    {
        let callbacks = callbacks.clone();
        tokio::spawn(async move {
            while let Some((tag, cb)) = tag_callback_rx.recv().await {
                if let Some(cb) = cb {
                    callbacks.lock().await.insert(tag, cb);
                } else {
                    callbacks.lock().await.remove(&tag);
                }
            }
        });
    }

    {
        // TODO: Reload on certain events
        let mut _reload_tx = reload_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = ctrl_ev_rx.recv().await {
                match ev {
                    api::ControllerEvent::Interact(api::InteractEvent { kind, tag, .. }) => {
                        let callback: Option<InteractCallback> =
                            callbacks.lock().await.get(&tag).cloned();
                        callback.inspect(|cb| cb(InteractArgs { kind }));
                    }
                    ev => log::warn!("Unimplemented event handler: {ev:?}"),
                }
            }
        });
    }

    let pulse = Arc::new(clients::pulse::PulseClient::connect(reload_tx.subscribe()));
    let mut modules = [
        fac.fixed(BarTuiElem::Spacing(1)),
        fac.spawn(hypr::hypr_module),
        fac.fixed(BarTuiElem::FillSpace(1)),
        fac.spawn(tray::tray_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            pulse::PulseModuleCtx {
                pulse: pulse.clone(),
                device_kind: clients::pulse::PulseDeviceKind::Source,
                muted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
                unmuted_sym: crate::xtui::tui_center_symbol("", 2),
            },
            pulse::pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            pulse::PulseModuleCtx {
                pulse,
                device_kind: clients::pulse::PulseDeviceKind::Sink,
                muted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
                unmuted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
            },
            pulse::pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(energy::ppd_module),
        fac.spawn(energy::energy_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(time::time_module),
        fac.fixed(BarTuiElem::Spacing(1)),
    ];

    let mut module_tasks = JoinSet::new();

    {
        let bar_tui_tx_inner = watch::Sender::new(Vec::from_iter(
            modules.iter_mut().map(|it| it.borrow_and_update().clone()),
        ));
        for (i, mut module) in modules.into_iter().enumerate() {
            let bar_tui_tx_inner = bar_tui_tx_inner.clone();
            module_tasks.spawn(async move {
                while let Ok(()) = module.changed().await {
                    let tui = module.borrow_and_update().clone();
                    bar_tui_tx_inner.send_modify(|modules| modules[i] = tui);
                }
            });
        }
        let mut bar_tui_rx_inner = bar_tui_tx_inner.subscribe();
        required_tasks.spawn(async move {
            while let Ok(()) = bar_tui_rx_inner.changed().await {
                send_bar_tui(&bar_tui_rx_inner.borrow_and_update(), &ctrl_upd_tx);
            }
        });
    }

    reload_tx.reload();

    match required_tasks
        .join_next()
        .await
        .unwrap_or_else(|| unreachable!())
        .ok_or_log()
    {
        Some(_) => std::process::ExitCode::SUCCESS,
        None => std::process::ExitCode::FAILURE,
    }
}
