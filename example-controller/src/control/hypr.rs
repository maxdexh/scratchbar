use std::{collections::HashMap, sync::Arc};

use crate::{
    clients,
    control::{BarTuiElem, InteractTagRegistry, ModuleArgs, interact_callback_with},
    utils::ResultExt as _,
    xtui,
};
use scratchbar::tui;

pub async fn hypr_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    let hypr = Arc::new(clients::hypr::connect(reload_rx));

    let mut basic_rx = hypr.basic_rx.clone();
    basic_rx.mark_changed();

    let mut ws_reg = InteractTagRegistry::new();

    while let Some(()) = basic_rx.changed().await.ok_or_debug() {
        let mut by_monitor = HashMap::new();
        for ws in basic_rx.borrow_and_update().workspaces.iter() {
            let Some(monitor) = ws.monitor.clone() else {
                continue;
            };

            let (_, (tui, tui_active)) = ws_reg.get_or_init(&ws.id, |tag| {
                let mk = |fg| {
                    xtui::underline_hovered(
                        &ws.name,
                        tui::TextStyle {
                            fg: Clone::clone(&fg),
                            ..Default::default()
                        },
                        tag.clone(),
                    )
                };

                let on_interact = interact_callback_with(
                    (hypr.clone(), ws.id.clone()),
                    move |(hypr, ws_id), interact| {
                        if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                            return;
                        }
                        hypr.switch_workspace(ws_id.clone());
                    },
                );

                ctrl_tx.register_callback(tag.clone(), Some(on_interact));

                (mk(None), mk(Some(tui::TermColor::Green)))
            });

            let wss = by_monitor
                .entry(monitor)
                .or_insert_with(|| xtui::StackBuilder::new(tui::Axis::X));

            wss.push(if ws.is_active {
                tui_active.clone()
            } else {
                tui.clone()
            });
            wss.spacing(1);
        }
        let by_monitor = by_monitor
            .into_iter()
            .map(|(k, v)| (k, v.build()))
            .collect();

        tui_tx.send_replace(BarTuiElem::ByMonitor(by_monitor));
    }
}
