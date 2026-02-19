use std::sync::Arc;

use crate::{
    clients,
    control::{BarTuiElem, ModuleArgs, interact_callback_with, mk_fresh_interact_tag},
    utils::ResultExt as _,
    xtui,
};
use scratchbar::tui;

pub struct PulseModuleArgs {
    pub pulse: Arc<clients::pulse::PulseClient>,
    pub device_kind: clients::pulse::PulseDeviceKind,
    pub muted_sym: tui::Elem,
    pub unmuted_sym: tui::Elem,
}
pub async fn pulse_module(
    PulseModuleArgs {
        pulse,
        device_kind,
        muted_sym,
        unmuted_sym,
    }: PulseModuleArgs,
    ModuleArgs {
        tui_tx, ctrl_tx, ..
    }: ModuleArgs,
) {
    use crate::clients::pulse::*;

    let mut state_rx = pulse.state_rx.clone();

    let on_interact = interact_callback_with(pulse.clone(), move |pulse, interact| {
        pulse
            .update_tx
            .send(PulseUpdate {
                target: device_kind,
                kind: match interact.kind {
                    tui::InteractKind::Click(tui::MouseButton::Left) => PulseUpdateKind::ToggleMute,
                    tui::InteractKind::Click(tui::MouseButton::Right) => {
                        PulseUpdateKind::ResetVolume
                    }
                    tui::InteractKind::Scroll(direction) => PulseUpdateKind::VolumeDelta(
                        2 * match direction {
                            tui::Direction::Up => 1,
                            tui::Direction::Down => -1,
                            tui::Direction::Left => -1,
                            tui::Direction::Right => 1,
                        },
                    ),
                    _ => return,
                },
            })
            .ok_or_log();
    });

    let interact_tag = mk_fresh_interact_tag();
    ctrl_tx.register_callback(interact_tag.clone(), on_interact);

    while let Some(()) = state_rx.changed().await.ok_or_debug() {
        let state = state_rx.borrow_and_update();
        let &PulseDeviceState { volume, muted, .. } = match device_kind {
            PulseDeviceKind::Sink => &state.sink,
            PulseDeviceKind::Source => &state.source,
        };
        drop(state);

        tui_tx.send_replace(BarTuiElem::Shared({
            let mut stack = xtui::StackBuilder::new(tui::Axis::X);
            stack.push(if muted {
                muted_sym.clone()
            } else {
                unmuted_sym.clone()
            });
            stack.push(tui::Elem::text(
                format!("{:>3}%", (volume * 100.0).round() as u32),
                tui::TextOpts::default(),
            ));
            stack.build().interactive(interact_tag.clone())
        }));
    }
}
