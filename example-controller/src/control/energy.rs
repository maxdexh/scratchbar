use std::sync::Arc;

use crate::{
    clients,
    control::{
        BarTuiElem, MenuKind, ModuleArgs, RegisterMenu, interact_callback_with,
        mk_fresh_interact_tag,
    },
    utils::ResultExt as _,
    xtui::text,
};
use scratchbar::tui;
use tokio::sync::watch;

pub async fn ppd_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    let ppd = Arc::new(clients::ppd::connect(reload_rx));

    let interact_tag = mk_fresh_interact_tag();

    let on_interact = interact_callback_with(ppd.clone(), |ppd, interact| {
        if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
            return;
        }
        ppd.cycle_profile();
    });
    ctrl_tx.register_callback(interact_tag.clone(), on_interact);

    let mut profile_rx = ppd.profile_rx.clone();
    while let Some(()) = profile_rx.changed().await.ok_or_debug() {
        let profile = profile_rx.borrow_and_update().clone();
        let tui = text::TextOpts::default().render_line("No profile");
        ctrl_tx.register_menu(RegisterMenu {
            on_tag: interact_tag.clone(),
            on_kind: tui::InteractKind::Hover,
            tui_rx: watch::channel(tui).1,
            menu_kind: MenuKind::Tooltip,
            opts: Default::default(),
        });

        let icon = text::TextOpts::from(text::HorizontalAlign::Center).render_cell(
            match profile.as_deref() {
                Some("balanced") => "",
                Some("performance") => "",
                Some("power-saver") => "",
                _ => {
                    tui_tx.send_if_modified(|tui| {
                        let old = std::mem::replace(tui, BarTuiElem::Hide);
                        !matches!(old, BarTuiElem::Hide)
                    });
                    continue;
                }
            },
            2.try_into().unwrap(),
        );

        tui_tx.send_replace(BarTuiElem::Shared(icon.interactive(interact_tag.clone())));
    }
}

pub async fn energy_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    use crate::clients::upower::*;
    let energy = Arc::new(clients::upower::connect(reload_rx));

    let interact_tag = mk_fresh_interact_tag();

    let mut state_rx = energy.state_rx.clone();
    state_rx.mark_changed();

    let mut last_energy_text = String::default();
    let mut last_tooltip = String::default();
    while let Some(()) = state_rx.changed().await.ok_or_debug() {
        let state = state_rx.borrow_and_update().clone();
        if !state.is_present {
            tui_tx.send_replace(BarTuiElem::Hide);
            continue;
        }

        {
            let percentage = state.percentage.round() as i64;
            let sign = match state.battery_state {
                BatteryState::Discharging | BatteryState::PendingDischarge => '-',
                BatteryState::Charging | BatteryState::PendingCharge => '+',
                BatteryState::FullyCharged | BatteryState::Unknown | BatteryState::Empty => '±',
            };
            let rate = format!("{sign}{:.1}W", state.energy_rate);
            let energy = format!("{percentage:>3}% {rate:<6}");

            if energy != last_energy_text {
                let tui = text::TextOpts::default().render_line(energy.as_str());
                tui_tx.send_replace(BarTuiElem::Shared(tui));
                last_energy_text = energy;
            }
        }

        {
            let text = {
                let display_time = |time: std::time::Duration| {
                    let hours = time.as_secs() / 3600;
                    let mins = (time.as_secs() / 60) % 60;
                    format!("{hours}h {mins}min")
                };
                match state.battery_state {
                    BatteryState::Discharging | BatteryState::PendingDischarge => {
                        format!("Battery empty in {}", display_time(state.time_to_empty))
                    }
                    BatteryState::FullyCharged => "Battery full".to_owned(),
                    BatteryState::Empty => "Battery empty".to_owned(),
                    BatteryState::Unknown => "Battery state unknown".to_owned(),
                    BatteryState::Charging | BatteryState::PendingCharge => {
                        format!("Battery full in {}", display_time(state.time_to_full))
                    }
                }
            };
            if text != last_tooltip {
                let tui = text::TextOpts::default().render_line(text.as_str());
                ctrl_tx.register_menu(RegisterMenu {
                    on_tag: interact_tag.clone(),
                    on_kind: tui::InteractKind::Hover,
                    tui_rx: watch::channel(tui).1,
                    menu_kind: MenuKind::Tooltip,
                    opts: Default::default(),
                });
                last_tooltip = text;
            }
        }
    }
}
