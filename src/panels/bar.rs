use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Context;
use futures::Stream;
use tokio_stream::StreamExt;
use tokio_util::{sync::CancellationToken, time::FutureExt as _};

use crate::{
    modules::prelude::{ModuleAct, ModuleId, ModuleInteract, ModuleInteractPayload, ModuleUpd},
    monitors::{MonitorEvent, MonitorInfo},
    terminals::{SpawnTerm, TermEvent, TermId, TermMgrUpdate, TermUpdate},
    tui,
    utils::{
        CancelDropGuard, Emit, ReloadRx, ReloadTx, ResultExt, SharedEmit, UnbTx, WatchRx, WatchTx,
        unb_chan,
    },
};

#[derive(Clone, Debug)]
pub struct ModuleActTxImpl {
    id: ModuleId,
    tx: UnbTx<Upd>,
}
impl Emit<ModuleAct> for ModuleActTxImpl {
    fn emit(&mut self, val: ModuleAct) -> std::ops::ControlFlow<()> {
        let id = self.id.clone();
        self.tx.emit(Upd::Act(id, val))
    }
}
pub type ModuleUpdRxImpl = tokio_stream::wrappers::UnboundedReceiverStream<ModuleUpd>;

pub enum BarMgrUpd {
    LoadModules(LoadModules),
}
pub struct BarMgrModuleArgs {
    pub act_tx: ModuleActTxImpl,
    pub upd_rx: ModuleUpdRxImpl,
    pub reload_rx: crate::utils::ReloadRx,
    pub cancel: tokio_util::sync::CancellationToken,
    // TODO: Config
}
pub struct BarMgrModuleParams {
    pub start: Box<dyn FnOnce(BarMgrModuleArgs) -> anyhow::Result<()> + Send>,
}

pub struct LoadModules {
    pub start: HashMap<ModuleId, BarMgrModuleParams>,
    pub left: Vec<ModuleId>,
    pub right: Vec<ModuleId>,
}

struct ModuleInst {
    _cancel: CancelDropGuard,
    tui: BarTuiElem,
    upd_tx: UnbTx<ModuleUpd>,
}
fn mod_inst(
    old: Option<ModuleInst>,
    reload_rx: ReloadRx,
    act_tx: ModuleActTxImpl,
) -> (ModuleInst, BarMgrModuleArgs) {
    let (upd_tx, upd_rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();

    let tui = old.map_or(BarTuiElem::Shared(tui::Elem::Empty), |it| it.tui);
    (
        ModuleInst {
            _cancel: cancel.clone().into(),
            tui,
            upd_tx,
        },
        BarMgrModuleArgs {
            cancel,
            act_tx,
            upd_rx: upd_rx.into(),
            reload_rx,
        },
    )
}

enum Upd {
    Bar(BarMgrUpd),
    Monitor(MonitorEvent),
    Act(ModuleId, ModuleAct),
    Mod(ModuleUpd), // FIXME: (ModuleId, ModuleUpd)
}

#[derive(Debug, Clone)]
enum BarTuiElem {
    Shared(tui::Elem),
    ByMonitor(HashMap<Arc<str>, tui::Elem>),
    Hide,
}
#[derive(Default)]
struct BarTui {
    left: Vec<BarTuiElem>,
    right: Vec<BarTuiElem>,
}
impl BarTui {
    fn to_tui(&self, monitor: &MonitorInfo) -> tui::Tui {
        let mut parts = Vec::new();
        let ap = |parts: &mut Vec<_>, elem: &_| {
            // TODO: Error on missing
            let elem = match elem {
                BarTuiElem::Shared(elem) => Some(elem.clone()),
                BarTuiElem::ByMonitor(elems) => Some(elems.get(&monitor.name).cloned()?),
                BarTuiElem::Hide => None,
            };
            parts.extend(elem.map(tui::StackItem::auto));
            Some(())
        };
        parts.push(tui::StackItem::spacing(1));
        for elem in &self.left {
            ap(&mut parts, elem);
        }
        parts.push(tui::StackItem::new(tui::Constr::Fill(1), tui::Elem::Empty));
        for elem in &self.right {
            ap(&mut parts, elem);
        }
        parts.push(tui::StackItem::spacing(1));
        tui::Tui {
            root: Box::new(tui::Stack::horizontal(parts).into()),
        }
    }
}

pub async fn run_manager(bar_upd_rx: impl Stream<Item = BarMgrUpd> + Send + 'static) {
    tokio::pin!(bar_upd_rx);

    let mut reload_tx = crate::utils::ReloadTx::new();

    #[derive(Default)]
    struct State {
        modules: HashMap<ModuleId, ModuleInst>,
        left: Vec<ModuleId>,
        right: Vec<ModuleId>,
        monitors: HashMap<Arc<str>, CancelDropGuard>,
    }
    let mut state = State::default();
    let bar_tui_tx = WatchTx::new(BarTui::default());

    let (mgr_upd_tx, mut mgr_upd_rx) = unb_chan();
    let (term_upd_tx, term_upd_rx) = unb_chan();
    tokio::spawn(crate::terminals::run_term_manager(term_upd_rx));

    crate::monitors::connect({
        let mut mgr_upd_tx = mgr_upd_tx.clone();
        move |ev| mgr_upd_tx.emit(Upd::Monitor(ev))
    });

    loop {
        // TODO: listen for monitor cancellations
        let upd = tokio::select! {
            Some(upd) = bar_upd_rx.next() => Upd::Bar(upd),
            Some(upd) = mgr_upd_rx.next() => upd,
        };
        match upd {
            Upd::Mod(ev) => {
                // FIXME: Targeted
                state
                    .modules
                    .retain(|_, module| module.upd_tx.emit(ev.clone()).is_continue());
            }
            Upd::Bar(BarMgrUpd::LoadModules(LoadModules { start, left, right })) => {
                state.left = left;
                state.right = right;
                let mut old = std::mem::take(&mut state.modules);
                for (id, m) in start {
                    let (inst, args) = mod_inst(
                        old.remove(&id),
                        reload_tx.subscribe(),
                        ModuleActTxImpl {
                            id: id.clone(),
                            tx: mgr_upd_tx.clone(),
                        },
                    );
                    if (m.start)(args).ok_or_log().is_none() {
                        continue;
                    }
                    state.modules.insert(id, inst);
                }
                reload_tx.reload();
            }
            Upd::Monitor(ev) => {
                for monitor in ev.added_or_changed() {
                    let cancel = CancellationToken::new();
                    tokio::spawn(run_monitor(
                        monitor.clone(),
                        cancel.clone(),
                        bar_tui_tx.subscribe(),
                        reload_tx.clone(),
                        term_upd_tx.clone(),
                        {
                            let mut mgr_upd_tx = mgr_upd_tx.clone();
                            move |ev| mgr_upd_tx.emit(Upd::Mod(ev))
                        },
                    ));
                    state.monitors.insert(monitor.name.clone(), cancel.into());
                }
                reload_tx.reload();
            }
            Upd::Act(id, act) => {
                if let Some(module) = state
                    .modules
                    .get_mut(&id)
                    .with_context(|| format!("Unknown module id {id:?}"))
                    .ok_or_log()
                {
                    let mut rerender = false;
                    match act {
                        ModuleAct::RenderByMonitor(elems) => {
                            module.tui = BarTuiElem::ByMonitor(elems);
                            rerender = true
                        }
                        ModuleAct::RenderAll(elem) => {
                            module.tui = BarTuiElem::Shared(elem);
                            rerender = true
                        }
                        ModuleAct::OpenMenu(open) => {
                            // TODO: Send to menu directly
                        }
                        ModuleAct::HideModule => {
                            module.tui = BarTuiElem::Hide;
                        }
                    }

                    if rerender {
                        let mut tui = BarTui::default();
                        for (src, dst) in
                            [(&state.left, &mut tui.left), (&state.right, &mut tui.right)]
                        {
                            for mid in src {
                                let Some(module) = state.modules.get(mid) else {
                                    log::error!("Unknown module id {mid:?}");
                                    continue;
                                };
                                dst.push(module.tui.clone());
                            }
                        }
                        _ = bar_tui_tx.send(tui);
                    }
                }
            }
        }
    }
}

async fn run_monitor(
    monitor: MonitorInfo,
    cancel: CancellationToken,
    mut bar_tui_rx: WatchRx<BarTui>,
    mut reload_tx: ReloadTx,
    mut term_upd_tx: impl SharedEmit<TermMgrUpdate>,
    mut mod_upd_tx: impl SharedEmit<ModuleUpd>, // FIXME: (ModuleId, ModuleUpd)
) {
    let _auto_cancel = CancelDropGuard::from(cancel.clone());

    let bar_term_id = TermId::from_str(&format!("BAR-{}", monitor.name));

    let (term_ev_tx, mut term_ev_rx) = unb_chan();

    // FIXME: Spawn in task, with timeout and await first size update
    if term_upd_tx
        .emit(TermMgrUpdate::SpawnPanel(SpawnTerm {
            term_id: bar_term_id.clone(),
            extra_args: vec![
                format!("--output-name={}", monitor.name).into(),
                // Allow logging to $KITTY_STDIO_FORWARDED
                "-o=forward_stdio=yes".into(),
                // Do not use the system's kitty.conf
                "--config=NONE".into(),
                // Basic look of the bar
                "-o=foreground=white".into(),
                "-o=background=black".into(),
                // location of the bar
                format!("--edge={}", crate::panels::EDGE).into(),
                // disable hiding the mouse
                "-o=mouse_hide_wait=0".into(),
            ],
            extra_envs: Default::default(),
            term_ev_tx,
            cancel: cancel.clone(),
        }))
        .is_break()
    {
        return;
    }

    let Some(mut bar_sizes) = async {
        loop {
            if let Some((_, TermEvent::Sizes(sizes))) = term_ev_rx.next().await {
                break sizes;
            }
        }
    }
    .timeout(Duration::from_secs(10))
    .await
    .context("Failed to receive terminal sizes")
    .ok_or_log() else {
        return;
    };

    enum Upd {
        BarTui,
        BarTerm(TermEvent),
    }
    let mut bar_layout = tui::RenderedLayout::default();
    loop {
        let upd = tokio::select! {
            Some((_, ev)) = term_ev_rx.next() => Upd::BarTerm(ev),
            Ok(()) = bar_tui_rx.changed() => Upd::BarTui,
        };
        match upd {
            Upd::BarTui => {
                let tui = bar_tui_rx.borrow_and_update().to_tui(&monitor);
                let mut buf = Vec::new();
                let Some(layout) = tui::draw_to(&mut buf, |ctx| {
                    let size = bar_sizes.cell_size;
                    tui.render(
                        ctx,
                        tui::SizingContext {
                            font_size: bar_sizes.font_size(),
                            div_w: Some(size.x),
                            div_h: Some(size.y),
                        },
                        tui::Area {
                            size,
                            pos: Default::default(),
                        },
                    )
                })
                .context("Failed to render bar")
                .ok_or_log() else {
                    continue;
                };
                bar_layout = layout;
                let mut emit =
                    |tupd| term_upd_tx.emit(TermMgrUpdate::TermUpdate(bar_term_id.clone(), tupd));
                if emit(TermUpdate::Print(buf)).is_break() || emit(TermUpdate::Flush).is_break() {
                    return;
                }
            }
            Upd::BarTerm(ev) => match ev {
                TermEvent::Crossterm(ev) => match ev {
                    crossterm::event::Event::Mouse(ev) => {
                        match bar_layout.interpret_mouse_event(ev, bar_sizes.font_size()) {
                            Some(tui::TuiInteract {
                                location,
                                payload: Some(target),
                                kind,
                            }) => {
                                if mod_upd_tx
                                    .emit(ModuleUpd::Interact(ModuleInteract {
                                        location,
                                        payload: ModuleInteractPayload {
                                            tag: target.clone(),
                                            monitor: monitor.name.clone(),
                                        },
                                        kind: kind.clone(),
                                    }))
                                    .is_break()
                                {
                                    break;
                                }
                            }
                            Some(tui::TuiInteract {
                                location: _,
                                payload: None,
                                kind,
                            }) => {
                                // TODO: Close menu of this monitor
                            }
                            None => {}
                        }
                    }
                    crossterm::event::Event::FocusLost => {
                        // TODO: Close menu of this monitor
                    }
                    _ => {}
                },
                TermEvent::Sizes(sizes) => {
                    bar_sizes = sizes;
                    reload_tx.reload();
                }
            },
        }
    }
}
