pub mod fixed;
pub mod hypr;
pub mod ppd;
pub mod pulse;
pub mod time;
pub mod tray;
pub mod upower;

pub mod prelude {
    use crate::{
        tui::{self, InteractGeneric},
        utils::CancelDropGuard,
    };
    use std::sync::Arc;

    #[non_exhaustive]
    pub enum ModuleAct {
        RenderByMonitor(std::collections::HashMap<Arc<str>, tui::StackItem>),
        RenderAll(tui::StackItem),
        HideModule,
        // FIXME: Add means to update the content of a menu.
        OpenMenu(OpenMenu),
    }
    pub struct OpenMenu {
        pub monitor: Arc<str>,
        pub tui: tui::Elem,
        pub pos: tui::Vec2<u32>,
        pub menu_kind: MenuKind,
    }
    #[derive(Debug, Clone, Copy)]
    pub enum MenuKind {
        Tooltip,
        Context,
    }
    #[derive(Debug, Clone)] // FIXME: Remove Clone
    pub enum ModuleUpd {
        Interact(ModuleInteract),
    }

    pub type ModuleInteract = InteractGeneric<ModuleInteractPayload>;
    #[derive(Debug, Clone)]
    pub struct ModuleInteractPayload {
        pub tag: tui::InteractTag,
        pub monitor: Arc<str>,
    }
    pub struct ModuleArgs {
        pub act_tx: ModuleActTx,
        pub upd_rx: ModuleUpdRx,
        pub reload_rx: crate::utils::ReloadRx,
        pub inst_id: ModInstId,
    }
    pub type ModuleActTx = crate::panels::ModuleActTxImpl;
    pub type ModuleUpdRx = crate::panels::ModuleUpdRxImpl;
    pub type ModInstId = crate::panels::ModInstIdImpl;

    pub trait Module: 'static + Send + Sync {
        type Config: 'static + Send; //+ serde::de::DeserializeOwned + Default;

        fn connect() -> Self;

        fn run_module_instance(
            self: Arc<Self>,
            cfg: Self::Config,
            _: ModuleArgs,
            _cancel: CancelDropGuard,
        ) -> impl Future<Output = ()> + Send;
    }
}
