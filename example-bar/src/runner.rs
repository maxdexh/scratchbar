use std::{collections::HashMap, sync::Arc};

use crate::utils::{ReloadRx, ReloadTx, ResultExt as _, WatchRx, WatchTx, watch_chan};
use anyhow::Context as _;
use ctrl::{api, tui};
use tokio::task::JoinSet;
use tokio_util::task::AbortOnDropHandle;

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
    tui_tx: WatchTx<BarTuiElem>,
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
    ) -> WatchRx<BarTuiElem> {
        let (tui_tx, tui_rx) = watch_chan(BarTuiElem::Hide);
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
    ) -> WatchRx<BarTuiElem> {
        self.spawn(|args| task(ctx, args))
    }
    fn fixed(&mut self, elem: BarTuiElem) -> WatchRx<BarTuiElem> {
        let (_, rx) = watch_chan(elem);
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

pub async fn main(
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

    let pulse = Arc::new(clients::pulse::connect(reload_tx.subscribe()));
    let mut modules = [
        fac.fixed(BarTuiElem::Spacing(1)),
        fac.spawn(hypr_module),
        fac.fixed(BarTuiElem::FillSpace(1)),
        fac.spawn(tray_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            PulseModuleCtx {
                pulse: pulse.clone(),
                device_kind: clients::pulse::PulseDeviceKind::Source,
                muted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
                unmuted_sym: center_symbol("", 2),
            },
            pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            PulseModuleCtx {
                pulse,
                device_kind: clients::pulse::PulseDeviceKind::Sink,
                muted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
                unmuted_sym: tui::Elem::text(" ", tui::TextOptions::default()),
            },
            pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(ppd_module),
        fac.spawn(energy_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(time_module),
        fac.fixed(BarTuiElem::Spacing(1)),
    ];

    let mut module_tasks = JoinSet::new();

    {
        let bar_tui_tx_inner = WatchTx::new(Vec::from_iter(
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

    if let Some(res) = required_tasks.join_next().await {
        match res.ok_or_log() {
            Some(_) => std::process::ExitCode::SUCCESS,
            None => std::process::ExitCode::FAILURE,
        }
    } else {
        unreachable!()
    }
}

async fn hypr_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        tag_callback_tx,
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
                    let mk2 = |style| tui::Elem::text(ws.name.clone(), style);
                    mk2(tui::Style {
                        fg: Clone::clone(&fg),
                        ..Default::default()
                    })
                    .interactive_hover(
                        tag.clone(),
                        mk2(tui::Style {
                            fg,
                            modifiers: Some(tui::Modifiers {
                                underline: true,
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
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
                tag_callback_tx
                    .send((tag.clone(), Some(on_interact)))
                    .ok_or_debug();

                (mk(None), mk(Some(tui::Color::Green)))
            });

            let wss = by_monitor
                .entry(monitor)
                .or_insert_with(|| tui::StackBuilder::new(tui::Axis::X));

            wss.fit(if ws.is_active {
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

async fn time_module(
    ModuleArgs {
        tui_tx,
        mut reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    use chrono::{Datelike, Timelike};
    use std::time::Duration;
    const WEEK_DAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    fn make_calendar(month: chrono::NaiveDate, today: chrono::NaiveDate) -> Option<tui::Elem> {
        let title = month.format("%B %Y").to_string();

        let first_weekday_offset = month
            .with_day(1)
            .with_context(|| format!("Failed to set day 1 of month {month}"))
            .ok_or_log()?
            .weekday() as u16;

        let mut tui_ystack = tui::StackBuilder::new(tui::Axis::Y);
        tui_ystack.fit(tui::Elem::text(
            title,
            tui::Modifiers {
                bold: true,
                ..Default::default()
            },
        ));
        tui_ystack.fit({
            let mut xstack = tui::StackBuilder::new(tui::Axis::X);
            for day in WEEK_DAYS {
                xstack.fit(tui::Elem::text(day, tui::TextOptions::default()));
                xstack.spacing(1);
            }
            xstack.delete_last();
            xstack.build()
        });

        let mut week_xstack = tui::StackBuilder::new(tui::Axis::X);
        week_xstack.spacing(3 * first_weekday_offset);
        for d0 in 0u16..month.num_days_in_month().into() {
            let d1 = d0.checked_add(1).context("Overflow").ok_or_log()?;
            let day = month
                .with_day(d1.into())
                .with_context(|| format!("Failed to set day {d1} in month {month}"))
                .ok_or_log()?;

            let text = format!("{d1:>2}");
            week_xstack.fit(tui::Elem::text(
                text,
                tui::Style {
                    fg: (day == today).then_some(tui::Color::Green),
                    ..Default::default()
                },
            ));

            if day.weekday() == chrono::Weekday::Sun {
                tui_ystack.fit(week_xstack.build());
                week_xstack = tui::StackBuilder::new(tui::Axis::X);
            } else {
                week_xstack.spacing(1);
            }
        }
        if !week_xstack.is_empty() {
            tui_ystack.fit(week_xstack.build());
        }

        Some(tui_ystack.build())
    }

    let interact_tag = mk_fresh_interact_tag();

    let interact_tag_clone = interact_tag.clone();
    let _menu_task = AbortOnDropHandle::new({
        tokio::spawn(async move {
            let now = chrono::Local::now().date_naive();
            if let Some(tui) = make_calendar(now, now) {
                ctrl_tx.set_menu(api::RegisterMenu {
                    on_tag: interact_tag_clone.clone(),
                    on_kind: tui::InteractKind::Hover,
                    tui,
                    menu_kind: api::MenuKind::Tooltip,
                    options: Default::default(),
                });
            }
            // FIXME: Schedule regular update
        })
    });

    let mut prev_minutes = 61;
    loop {
        let now = chrono::Local::now();
        let minute = now.minute();
        if prev_minutes != minute {
            let tui = tui::Elem::text(now.format("%H:%M %d/%m"), tui::TextOptions::default())
                .interactive(interact_tag.clone());

            tui_tx.send_replace(BarTuiElem::Shared(tui));

            prev_minutes = minute;
        } else {
            let seconds_until_minute = 60 - u64::from(now.second());
            let timeout_ms = std::cmp::max(750 * seconds_until_minute, 100);

            tokio::select! {
                Some(()) = reload_rx.wait() => {}
                () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {}
            }
        }
    }
}

struct PulseModuleCtx {
    pulse: Arc<clients::pulse::PulseClient>,
    device_kind: clients::pulse::PulseDeviceKind,
    muted_sym: tui::Elem,
    unmuted_sym: tui::Elem,
}
async fn pulse_module(
    PulseModuleCtx {
        pulse,
        device_kind,
        muted_sym,
        unmuted_sym,
    }: PulseModuleCtx,
    ModuleArgs {
        tui_tx,
        tag_callback_tx,
        ..
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
    tag_callback_tx
        .send((interact_tag.clone(), Some(on_interact)))
        .ok_or_log();

    while let Some(()) = state_rx.changed().await.ok_or_debug() {
        let state = state_rx.borrow_and_update();
        let &PulseDeviceState { volume, muted, .. } = match device_kind {
            PulseDeviceKind::Sink => &state.sink,
            PulseDeviceKind::Source => &state.source,
        };
        drop(state);

        tui_tx.send_replace(BarTuiElem::Shared({
            let mut stack = tui::StackBuilder::new(tui::Axis::X);
            stack.fit(if muted {
                muted_sym.clone()
            } else {
                unmuted_sym.clone()
            });
            stack.fit(tui::Elem::text(
                format!("{:>3}%", (volume * 100.0).round() as u32),
                tui::TextOptions::default(),
            ));
            stack.build().interactive(interact_tag.clone())
        }));
    }
}
async fn energy_module(
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
                let tui = tui::Elem::text(energy.as_str(), tui::TextOptions::default())
                    .interactive(interact_tag.clone());
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
                ctrl_tx.set_menu(api::RegisterMenu {
                    on_tag: interact_tag.clone(),
                    on_kind: tui::InteractKind::Hover,
                    tui: tui::Elem::text(text.as_str(), tui::TextOptions::default()),
                    menu_kind: api::MenuKind::Tooltip,
                    options: Default::default(),
                });
                last_tooltip = text;
            }
        }
    }
}
async fn ppd_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        tag_callback_tx,
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
    tag_callback_tx
        .send((interact_tag.clone(), Some(on_interact)))
        .ok_or_log();

    let mut profile_rx = ppd.profile_rx.clone();
    while let Some(()) = profile_rx.changed().await.ok_or_debug() {
        let profile = profile_rx.borrow_and_update().clone();
        ctrl_tx.set_menu(api::RegisterMenu {
            on_tag: interact_tag.clone(),
            on_kind: tui::InteractKind::Hover,
            tui: tui::Elem::text(
                profile.as_deref().unwrap_or("No profile"),
                tui::TextOptions::default(),
            ),
            menu_kind: api::MenuKind::Tooltip,
            options: Default::default(),
        });

        let icon: tui::Elem = match profile.as_deref() {
            Some("balanced") => tui::Elem::text(" ", tui::TextOptions::default()),
            Some("performance") => center_symbol(" ", 2),
            Some("power-saver") => tui::Elem::text(" ", tui::TextOptions::default()),
            _ => {
                tui_tx.send_if_modified(|tui| {
                    let old = std::mem::replace(tui, BarTuiElem::Hide);
                    !matches!(old, BarTuiElem::Hide)
                });
                continue;
            }
        };

        tui_tx.send_replace(BarTuiElem::Shared(icon.interactive(interact_tag.clone())));
    }
}

async fn tray_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        tag_callback_tx,
        ..
    }: ModuleArgs,
) {
    use crate::clients::tray::*;
    let tray = Arc::new(clients::tray::connect(reload_rx));

    let mut entry_reg = InteractTagRegistry::new();

    let mut state_rx = tray.state_rx.clone();
    while state_rx.changed().await.is_ok() {
        let state = state_rx.borrow_and_update().clone();

        let mut tui_stack = tui::StackBuilder::new(tui::Axis::X);
        for TrayEntry { addr, item, menu } in state.entries.iter() {
            let (tag, ()) = entry_reg.get_or_init(addr, |_| ());

            // FIXME: Handle icon attrs
            if let Some(system_tray::item::Tooltip {
                icon_name: _,
                icon_data: _,
                title,
                description,
            }) = item.tool_tip.as_ref()
            {
                let menu_tui = {
                    let mut menu_tui_stack = tui::StackBuilder::new(tui::Axis::Y);
                    menu_tui_stack.fit({
                        let mut hstack = tui::StackBuilder::new(tui::Axis::X);
                        hstack.fill(1, tui::Elem::empty());
                        hstack.fit(tui::Elem::text(
                            title,
                            tui::Modifiers {
                                bold: true,
                                ..Default::default()
                            },
                        ));
                        hstack.fill(1, tui::Elem::empty());
                        hstack.build()
                    });
                    menu_tui_stack.fit(tui::Elem::text(description, tui::TextOptions::default()));
                    menu_tui_stack.build()
                };
                ctrl_tx.set_menu(api::RegisterMenu {
                    on_tag: tag.clone(),
                    on_kind: tui::InteractKind::Hover,
                    menu_kind: api::MenuKind::Tooltip,
                    tui: menu_tui,
                    options: Default::default(),
                });
            }

            if let Some(TrayMenuExt {
                menu_path,
                submenus,
                ..
            }) = menu.as_ref()
            {
                let menu_tui = tray_menu_to_tui(0, submenus, &|id| {
                    let tag = mk_fresh_interact_tag();
                    let Some(menu_path) = menu_path.clone() else {
                        return tag;
                    };

                    let tray = tray.clone();
                    let addr = addr.clone();
                    let icb = Arc::new(move |interact: InteractArgs| {
                        if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                            return;
                        }
                        let addr = addr.clone();
                        let menu_path = menu_path.clone();
                        tray.sched_with_client(async move |client| {
                            client
                                .activate(system_tray::client::ActivateRequest::MenuItem {
                                    address: str::to_owned(&addr),
                                    menu_path: str::to_owned(&menu_path),
                                    submenu_id: id,
                                })
                                .await
                                .context("Failed to send ActivateRequest")
                                .ok_or_log();
                        });
                    });
                    tag_callback_tx.send((tag.clone(), Some(icb))).ok_or_debug();
                    tag
                });

                ctrl_tx.set_menu(api::RegisterMenu {
                    on_tag: tag.clone(),
                    on_kind: tui::InteractKind::Click(tui::MouseButton::Right),
                    menu_kind: api::MenuKind::Context,
                    tui: tui::Elem::build_block(|block| {
                        block.set_borders_at(tui::Borders::all());
                        block.set_style(tui::Style {
                            fg: Some(tui::Color::DarkGrey),
                            ..Default::default()
                        });
                        block.set_lines(tui::LineSet::thick());
                        block.set_inner(menu_tui);
                    }),
                    options: Default::default(),
                });
            }

            // FIXME: Handle the other options
            // FIXME: Why are we showing all icons?
            for system_tray::item::IconPixmap {
                width,
                height,
                pixels,
            } in item.icon_pixmap.as_deref().unwrap_or(&[])
            {
                let mut img = match ctrl::image::RgbaImage::from_vec(
                    width.cast_unsigned(),
                    height.cast_unsigned(),
                    pixels.clone(),
                ) {
                    Some(img) => img,
                    None => {
                        log::error!("Failed to load image from bytes");
                        continue;
                    }
                };

                // https://users.rust-lang.org/t/argb32-color-model/92061/4
                for ctrl::image::Rgba(pixel) in img.pixels_mut() {
                    *pixel = u32::from_be_bytes(*pixel).rotate_left(8).to_be_bytes();
                }

                tui_stack.fit(
                    tui::Elem::image(
                        img, //
                        tui::ImageSizeMode::FillAxis(tui::Axis::Y, 1),
                    )
                    .interactive(tag.clone()),
                );
                tui_stack.spacing(1);
            }
        }
        tui_tx.send_replace(BarTuiElem::Shared(tui_stack.build()));
    }
    fn tray_menu_item_to_tui(
        depth: u16,
        item: &system_tray::menu::MenuItem,
        mk_interact: &impl Fn(i32) -> tui::InteractTag,
    ) -> Option<tui::Elem> {
        use system_tray::menu::*;
        let main_elem = match item {
            MenuItem { visible: false, .. } => return None,
            MenuItem {
                visible: true,
                menu_type: MenuType::Separator,
                ..
            } => tui::Elem::build_block(|block| {
                block.set_borders_at(tui::Borders {
                    top: true,
                    ..Default::default()
                });
                block.set_style(tui::Style {
                    fg: Some(tui::Color::DarkGrey),
                    ..Default::default()
                });
            }),
            MenuItem {
                id,
                menu_type: MenuType::Standard,
                label: Some(label),
                enabled: _,
                visible: true,
                icon_name: _,
                icon_data,
                shortcut: _,
                toggle_type: _, // TODO: implement toggle
                toggle_state: _,
                children_display: _,
                disposition: _, // TODO: what to do with this?
                submenu: _,
            } => {
                let mut stack = tui::StackBuilder::new(tui::Axis::X);
                stack.spacing(depth + 1);
                if let Some(icon) = icon_data
                    && let Some(img) = ctrl::image::load_from_memory_with_format(
                        icon,
                        ctrl::image::ImageFormat::Png,
                    )
                    .context("Systray icon has invalid png data")
                    .ok_or_log()
                {
                    stack.fit(tui::Elem::image(
                        img.into_rgba8(),
                        tui::ImageSizeMode::FillAxis(tui::Axis::Y, 1),
                    ));
                    stack.spacing(1);
                }
                stack.fit(tui::Elem::text(label, tui::TextOptions::default()));
                // FIXME: Add hover
                stack.build().interactive(mk_interact(*id))
            }

            _ => {
                log::error!("Unhandled menu item: {item:#?}");
                return None;
            }
        };

        Some(if item.submenu.is_empty() {
            main_elem
        } else {
            let mut stack = tui::StackBuilder::new(tui::Axis::Y);
            stack.fit(main_elem);
            stack.fit(tray_menu_to_tui(depth + 1, &item.submenu, mk_interact));
            stack.build()
        })
    }

    fn tray_menu_to_tui(
        depth: u16,
        items: &[system_tray::menu::MenuItem],
        mk_interact: &impl Fn(i32) -> tui::InteractTag,
    ) -> tui::Elem {
        let mut stack = tui::StackBuilder::new(tui::Axis::Y);
        for item in items {
            if let Some(item) = tray_menu_item_to_tui(depth, item, mk_interact) {
                stack.fit(item)
            }
        }
        stack.build()
    }
}

fn center_symbol(sym: impl std::fmt::Display, width: u16) -> tui::Elem {
    tui::Elem::raw_print(
        format_args!("\x1b]66;w={width}:h=2:n=1:d=1;{sym}\x07"),
        tui::Vec2 { x: width, y: 1 },
    )
}
