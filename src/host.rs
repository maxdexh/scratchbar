use serde::{Deserialize, Serialize};

use std::sync::Arc;

use crate::{ctrl_ipc, tui, utils::ResultExt};

pub struct HostError(anyhow::Error);
impl std::fmt::Debug for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl std::error::Error for HostError {}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HostConnectOpts {
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}

#[derive(Debug)]
#[non_exhaustive]
pub struct HostConnection {
    pub update_tx: HostUpdateSender,
}
#[derive(Clone, Debug)]
pub struct HostUpdateSender {
    tx: std::sync::mpsc::Sender<HostUpdate>,
}
impl HostUpdateSender {
    pub fn send(&self, update: HostUpdate) -> Result<(), std::sync::mpsc::SendError<HostUpdate>> {
        self.tx.send(update)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HostUpdate {
    UpdateBars(BarSelect, BarUpdate),
    SetDefaultTui(SetBarTui),
    OpenMenu(OpenMenu),
    CloseMenu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenMenu {
    pub tui: tui::Elem,
    pub monitor: Arc<str>,
    pub bar_anchor: tui::CustomId,
    pub opts: OpenMenuOpts,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenMenuOpts {
    // TODO: Option to keep location, layout
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CloseMenuOpts {
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}

#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub enum BarUpdate {
    SetTui(SetBarTui),
    Hide,
    Show,
}
impl From<SetBarTui> for BarUpdate {
    fn from(value: SetBarTui) -> Self {
        Self::SetTui(value)
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SetBarTui {
    pub tui: tui::Elem,
    pub options: SetBarTuiOpts,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SetBarTuiOpts {
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
// FIXME: Use a struct similar to TermInfo instead
#[non_exhaustive]
#[derive(Debug, Serialize, Deserialize)]
pub enum BarSelect {
    All,
    OnMonitor { monitor_name: Arc<str> },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RegisterMenuOpts {
    // TODO: Option on whether to apply update to already open tui
    // TODO: Option to set font size of menu / other options temporarily / run commands when menu is shown / hidden?
    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HostEvent {
    Term(TermInfo, TermEvent),
    // TODO: Add monitor change event
    // TODO: Menu closed
}
#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TermEvent {
    Interact(InteractEvent),
    MouseLeave,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InteractEvent {
    pub kind: tui::InteractKind,
    pub tag: Option<tui::CustomId>,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FocusEvent {
    pub term: TermInfo,
    pub is_focused: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TermInfo {
    pub monitor: Arc<str>,
    pub kind: TermKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TermKind {
    Menu,
    Bar,
}

pub fn connect(
    opts: HostConnectOpts,
    mut event_tx: impl FnMut(HostEvent) -> Result<(), HostEvent> + Send + 'static,
    on_stop: impl FnOnce(Result<(), HostError>) + Send + 'static,
) -> Result<HostConnection, HostError> {
    match ctrl_ipc::connect_from_ctrl(
        ctrl_ipc::HostCtrlInit {
            version: ctrl_ipc::VERSION.into(),
            opts,
        },
        move |ev| {
            event_tx(ev)
                .map_err(std::sync::mpsc::SendError)
                .ok_or_debug()
        },
        |res| on_stop(res.map_err(HostError)),
    ) {
        Ok((ctrl_ipc::HostInitResponse {}, tx)) => Ok(HostConnection {
            update_tx: HostUpdateSender { tx },
        }),
        Err(err) => Err(HostError(err)),
    }
}

pub fn init_controller_logger() {
    crate::logging::init_logger("CONTROLLER".into());
}
