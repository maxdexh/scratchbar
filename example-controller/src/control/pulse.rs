use std::sync::Arc;

use crate::{
    clients,
    control::{BarTuiElem, ModuleArgs, interact_callback_with, mk_fresh_interact_tag},
    utils::ResultExt as _,
    xtui::{self, text},
};
use anyhow::Context as _;
use scratchbar::tui;

pub struct PulseModuleArgs {
    pub pulse: Arc<clients::pulse::PulseClient>,
    pub device_kind: clients::pulse::PulseDeviceKind,
}

pub async fn pulse_module(
    PulseModuleArgs { pulse, device_kind }: PulseModuleArgs,
    ModuleArgs {
        tui_tx, ctrl_tx, ..
    }: ModuleArgs,
) {
    use crate::clients::pulse::*;

    let [mut_icon, lo_icon, med_icon, hi_icon] = match device_kind {
        PulseDeviceKind::Sink => [
            xtui::icon!("audio-volume-muted"),
            xtui::icon!("audio-volume-low"),
            xtui::icon!("audio-volume-medium"),
            xtui::icon!("audio-volume-high"),
        ],
        PulseDeviceKind::Source => [
            xtui::icon!("microphone-sensitivity-muted"),
            xtui::icon!("microphone-sensitivity-low"),
            xtui::icon!("microphone-sensitivity-medium"),
            xtui::icon!("microphone-sensitivity-high"),
        ],
    }
    .map(|buf| {
        xtui::png_fill_axis(buf, tui::Axis::Y, 1)
            .context("icon must be valid png data")
            .ok_or_log()
            .unwrap_or_else(tui::Elem::empty)
    });

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
        let device_state @ &PulseDeviceState { volume, .. } = match device_kind {
            PulseDeviceKind::Sink => &state.sink,
            PulseDeviceKind::Source => &state.source,
        };
        let icon = match device_state {
            PulseDeviceState { muted: true, .. } => &mut_icon,
            PulseDeviceState { volume: ..0.33, .. } => &lo_icon,
            PulseDeviceState { volume: ..0.66, .. } => &med_icon,
            PulseDeviceState { .. } => &hi_icon,
        };
        drop(state);

        tui_tx.send_replace(BarTuiElem::Shared({
            let mut stack = xtui::StackBuilder::new(tui::Axis::X);

            stack.push(icon.clone());
            stack.push(
                text::TextOpts::default()
                    .render_line(&format!("{:>3}%", (volume * 100.0).round() as u32)),
            );
            stack.build().interactive(interact_tag.clone())
        }));
    }
}
