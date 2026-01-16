use std::sync::Arc;

use anyhow::Context;
use tokio::task::JoinSet;
use tokio_util::time::FutureExt;

use crate::{
    modules::{
        self,
        prelude::{Module, ModuleArgs},
    },
    panels::{BarMgrModuleArgs, BarMgrModuleStartArgs, BarMgrUpd},
    tui,
    utils::{Emit, ResultExt, unb_chan},
};

fn mkstart<M: Module>(module: Arc<M>, cfg: M::Config) -> BarMgrModuleStartArgs {
    let start = move |args| {
        let BarMgrModuleArgs {
            act_tx,
            upd_rx,
            reload_rx,
            cancel,
        } = args;

        tokio::spawn(async move {
            module
                .run_module_instance(
                    cfg,
                    ModuleArgs {
                        act_tx,
                        upd_rx,
                        reload_rx,
                    },
                    cancel.clone().into(),
                )
                .with_cancellation_token_owned(cancel)
                .await
        });

        Ok(())
    };
    BarMgrModuleStartArgs {
        start: Box::new(start),
    }
}

struct WeakModule<M>(std::sync::Weak<M>);
impl<M: Module> WeakModule<M> {
    pub fn get(&mut self) -> Arc<M> {
        if let Some(it) = self.0.upgrade() {
            it
        } else {
            let it = Arc::new(M::connect());
            self.0 = Arc::downgrade(&it);
            it
        }
    }
}
impl<M> Default for WeakModule<M> {
    fn default() -> Self {
        Self(Default::default())
    }
}

#[derive(Default)]
struct Modules {
    hypr: WeakModule<modules::hypr::HyprModule>,
    time: WeakModule<modules::time::TimeModule>,
    pulse: WeakModule<modules::pulse::PulseModule>,
    ppd: WeakModule<modules::ppd::PpdModule>,
    energy: WeakModule<modules::upower::EnergyModule>,
    tray: WeakModule<modules::tray::TrayModule>,
    fixed: WeakModule<modules::fixed::FixedTuiModule>,
}

#[allow(clippy::unit_arg)]
pub async fn main() {
    let mut tasks = JoinSet::new();
    let mut bar_upd_tx;
    {
        let bar_upd_rx;
        (bar_upd_tx, bar_upd_rx) = unb_chan();
        tasks.spawn(crate::panels::run_manager(bar_upd_rx));
    }

    let mut modules = Modules::default();

    bar_upd_tx.emit(BarMgrUpd::LoadModules(crate::panels::LoadModules {
        modules: [
            mkstart(modules.fixed.get(), tui::StackItem::spacing(1)),
            mkstart(modules.hypr.get(), Default::default()),
            mkstart(
                modules.fixed.get(),
                tui::StackItem::new(tui::Constr::Fill(1), tui::Elem::Empty),
            ),
            mkstart(modules.tray.get(), Default::default()),
            mkstart(modules.fixed.get(), tui::StackItem::spacing(3)),
            mkstart(modules.pulse.get(), Default::default()),
            mkstart(modules.fixed.get(), tui::StackItem::spacing(3)),
            mkstart(modules.ppd.get(), Default::default()),
            mkstart(modules.energy.get(), Default::default()),
            mkstart(modules.fixed.get(), tui::StackItem::spacing(3)),
            mkstart(modules.time.get(), Default::default()),
            mkstart(modules.fixed.get(), tui::StackItem::spacing(1)),
        ]
        .into(),
    }));

    if let Some(res) = tasks.join_next().await {
        res.context("Task failed").ok_or_log();
    }
}
