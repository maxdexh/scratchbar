mod energy;
mod hypr;
mod pulse;
mod time;
mod tray;

use std::{collections::HashMap, sync::Arc};

use crate::{
    utils::{ReloadRx, ReloadTx, ResultExt as _},
    xtui::{self, text},
};
use scratchbar::{host, tui};
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
    key_to_tag: HashMap<K, (tui::CustomId, V)>,
    tag_to_key: HashMap<tui::CustomId, K>,
}

fn mk_fresh_interact_tag() -> tui::CustomId {
    use std::sync::atomic::*;

    static TAG_COUNTER: AtomicU64 = AtomicU64::new(0);
    tui::CustomId::from_bytes(&TAG_COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes())
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
        init: impl FnOnce(&tui::CustomId) -> V,
    ) -> (&tui::CustomId, &mut V) {
        let (tag, val) = self.key_to_tag.entry(key.clone()).or_insert_with(|| {
            let tag = mk_fresh_interact_tag();
            self.tag_to_key.insert(tag.clone(), key.clone());
            let val = init(&tag);
            (tag, val)
        });
        (tag, val)
    }
}

#[derive(Debug)]
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

#[derive(Debug, Clone)]
struct BarMenu {
    tui_rx: watch::Receiver<tui::Elem>,
    kind: MenuKind,
}
type BarMenus = HashMap<tui::CustomId, HashMap<tui::InteractKind, BarMenu>>;

#[derive(Default)]
struct Callbacks {
    cbs: HashMap<tui::CustomId, InteractCallback>,
}
impl std::fmt::Debug for Callbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.cbs.keys(), f)
    }
}
#[derive(Debug, Clone)]
struct ModuleControlTx {
    tag_cb_tx: watch::Sender<Callbacks>,
    bar_menus_tx: watch::Sender<BarMenus>,
}
struct RegisterMenu {
    pub on_tag: tui::CustomId,
    pub on_kind: tui::InteractKind,
    pub tui_rx: watch::Receiver<tui::Elem>,
    pub menu_kind: MenuKind,
    pub opts: RegisterMenuOpts,
}
#[derive(Default)]
struct RegisterMenuOpts {}
#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuKind {
    Tooltip,
    Context,
}
impl ModuleControlTx {
    fn register_menu(&self, menu: RegisterMenu) {
        let RegisterMenu {
            on_tag,
            on_kind,
            tui_rx,
            opts: RegisterMenuOpts {},
            menu_kind,
        } = menu;

        let menu = BarMenu {
            tui_rx,
            kind: menu_kind,
        };
        self.bar_menus_tx.send_modify(|menus| {
            menus.entry(on_tag).or_default().insert(on_kind, menu);
        });
    }
    fn register_callback(&self, tag: tui::CustomId, cb: InteractCallback) {
        self.tag_cb_tx.send_modify(|cbs| {
            cbs.cbs.insert(tag, cb);
        })
    }
}

struct ModuleArgs {
    tui_tx: watch::Sender<BarTuiElem>,
    reload_rx: ReloadRx,
    ctrl_tx: ModuleControlTx,
    _unused: (),
}

struct BarModuleFactory {
    reload_tx: ReloadTx,
    ctrl_tx: ModuleControlTx,
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

fn send_bar_tui(bar_tui: &[BarTuiElem], ctrl_tx: &host::HostUpdateSender) {
    let mut by_monitor = HashMap::new();
    let mut fallback = xtui::StackBuilder::new(tui::Axis::X);
    for elem in bar_tui {
        match elem {
            BarTuiElem::Shared(elem) => {
                for stack in by_monitor.values_mut().chain(Some(&mut fallback)) {
                    stack.push(elem.clone());
                }
            }
            BarTuiElem::ByMonitor(elems) => {
                for (mtr, elem) in elems {
                    by_monitor
                        .entry(mtr.clone())
                        .or_insert_with(|| fallback.clone())
                        .push(elem.clone());
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
        .send(host::HostUpdate::SetDefaultTui(host::SetBarTui {
            tui: fallback.build(),
            options: Default::default(),
        }))
        .ok_or_debug();

    for (monitor, tui) in by_monitor {
        ctrl_tx
            .send(host::HostUpdate::UpdateBars(
                host::BarSelect::OnMonitor {
                    monitor_name: monitor,
                },
                host::SetBarTui {
                    tui: tui.build(),
                    options: Default::default(),
                }
                .into(),
            ))
            .ok_or_debug();
    }
}

#[derive(Clone, Debug)]
struct CurMenu {
    bar_anchor: tui::CustomId,
    menu_kind: MenuKind,
    monitor: Arc<str>,
    tui_rx: watch::Receiver<tui::Elem>,
}

async fn run_menu_mgr(
    ctrl_upd_tx: host::HostUpdateSender,
    mut cur_menu_rx: watch::Receiver<Option<CurMenu>>,
) {
    cur_menu_rx.mark_changed();
    let mut cur = None::<CurMenu>;
    loop {
        let in_place = tokio::select! {
            Some(Ok(())) = async { Some(cur.as_mut()?.tui_rx.changed().await) } => true,
            Ok(()) = cur_menu_rx.changed() => false,
            else => break,
        };
        if !in_place {
            cur = cur_menu_rx.borrow_and_update().clone();
        }
        ctrl_upd_tx
            .send(match cur.as_mut() {
                Some(cur) => {
                    let mut tui = cur.tui_rx.borrow_and_update().clone();
                    if cur.menu_kind == MenuKind::Tooltip {
                        // Make the entire tui interactive because we want to close
                        // tooltips on any interaction
                        tui = tui.interactive(mk_fresh_interact_tag());
                    }
                    host::HostUpdate::OpenMenu(host::OpenMenu {
                        tui,
                        monitor: cur.monitor.clone(),
                        bar_anchor: cur.bar_anchor.clone(),
                        opts: Default::default(),
                    })
                }
                None => host::HostUpdate::CloseMenu,
            })
            .ok_or_debug();
    }
}

async fn run_event_handler(
    ctrl_upd_tx: host::HostUpdateSender,
    mut ctrl_ev_rx: tokio::sync::mpsc::UnboundedReceiver<host::HostEvent>,
    mut bar_menus_rx: watch::Receiver<BarMenus>,
    tag_cb_rx: watch::Receiver<Callbacks>,
    // TODO: Reload on certain events (monitor changes)
    _reload_tx: ReloadTx,
) {
    let cur_menu_tx = watch::Sender::new(None);
    tokio::spawn(run_menu_mgr(ctrl_upd_tx.clone(), cur_menu_tx.subscribe()));

    while let Some(ev) = ctrl_ev_rx.recv().await {
        match ev {
            host::HostEvent::Term(
                term,
                host::TermEvent::Interact(host::InteractEvent {
                    kind: ikind, tag, ..
                }),
            ) => {
                if let Some(tag) = &tag {
                    let callback: Option<InteractCallback> =
                        tag_cb_rx.borrow().cbs.get(tag).cloned();
                    callback.inspect(|cb| {
                        cb(InteractArgs {
                            kind: ikind.clone(),
                        })
                    });
                }

                let is_hover = matches!(ikind, tui::InteractKind::Hover);

                match term.kind {
                    host::TermKind::Bar => {
                        if let Some(tag) = tag
                            && let Some(BarMenu {
                                tui_rx,
                                kind: mkind,
                            }) = bar_menus_rx
                                .borrow_and_update()
                                .get(&tag)
                                .and_then(|tag_menus| tag_menus.get(&ikind))
                                .cloned()
                        {
                            cur_menu_tx.send_if_modified(|cur| {
                                // Do not replace non-tooltips with tooltips
                                if mkind == MenuKind::Tooltip
                                    && cur
                                        .as_ref()
                                        .is_some_and(|it| it.menu_kind != MenuKind::Tooltip)
                                {
                                    return false;
                                }
                                *cur = Some(CurMenu {
                                    bar_anchor: tag,
                                    menu_kind: mkind,
                                    monitor: term.monitor,
                                    tui_rx: tui_rx.clone(),
                                });
                                true
                            });
                        } else {
                            cur_menu_tx.send_if_modified(|cur_opt| {
                                cur_opt
                                    .take_if(|cur| cur.menu_kind == MenuKind::Tooltip || !is_hover)
                                    .is_some()
                            });
                        }
                    }
                    host::TermKind::Menu => {
                        cur_menu_tx.send_if_modified(|cur_opt| {
                            cur_opt
                                .take_if(|cur| cur.menu_kind == MenuKind::Tooltip)
                                .is_some()
                        });
                    }
                    _ => {}
                }
            }
            host::HostEvent::Term(
                host::TermInfo {
                    kind: host::TermKind::Menu,
                    ..
                },
                host::TermEvent::MouseLeave,
            ) => {
                cur_menu_tx.send_replace(None);
            }
            ev => {
                log::trace!("Ignoring event {ev:?}");
            }
        }
    }
}

pub async fn control_main(
    connect: host::HostConnection,
    ctrl_ev_rx: tokio::sync::mpsc::UnboundedReceiver<host::HostEvent>,
) -> std::process::ExitCode {
    let mut required_tasks = JoinSet::new();

    let mut reload_tx = ReloadTx::new();

    let tag_cb_tx = watch::Sender::new(Callbacks::default());
    let bar_menus_tx = watch::Sender::new(BarMenus::default());
    tokio::spawn(run_event_handler(
        connect.update_tx.clone(),
        ctrl_ev_rx,
        bar_menus_tx.subscribe(),
        tag_cb_tx.subscribe(),
        reload_tx.clone(),
    ));
    let mut fac = BarModuleFactory {
        reload_tx: reload_tx.clone(),
        ctrl_tx: ModuleControlTx {
            tag_cb_tx,
            bar_menus_tx,
        },
        tasks: JoinSet::new(),
    };

    let pulse = Arc::new(clients::pulse::PulseClient::connect(reload_tx.subscribe()));
    let pulse_symbol_opts = text::TextOpts::from(text::HorizontalAlign::Center);
    let pulse_symbol_width = 2.try_into().unwrap();

    let mut modules = [
        fac.fixed(BarTuiElem::Spacing(1)),
        fac.spawn(hypr::hypr_module),
        fac.fixed(BarTuiElem::FillSpace(1)),
        fac.spawn(tray::tray_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            pulse::PulseModuleArgs {
                pulse: pulse.clone(),
                device_kind: clients::pulse::PulseDeviceKind::Source,
                muted_sym: pulse_symbol_opts.render_cell("", pulse_symbol_width),
                unmuted_sym: pulse_symbol_opts.render_cell("", pulse_symbol_width),
            },
            pulse::pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            pulse::PulseModuleArgs {
                pulse,
                device_kind: clients::pulse::PulseDeviceKind::Sink,
                muted_sym: pulse_symbol_opts.render_cell("", pulse_symbol_width),
                unmuted_sym: pulse_symbol_opts.render_cell("", pulse_symbol_width),
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
                send_bar_tui(&bar_tui_rx_inner.borrow_and_update(), &connect.update_tx);
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
