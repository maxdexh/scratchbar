use std::{collections::HashMap, sync::Arc};

use anyhow::Context as _;
use ctrl::BarTuiState;
use ctrl::{
    tui,
    utils::{
        Callback, ReloadRx, ReloadTx, ResultExt as _, UnbRx, UnbTx, WatchRx, WatchTx, lock_mutex,
        unb_chan, watch_chan,
    },
};
use futures::StreamExt;
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
type InteractCallback = Callback<InteractArgs, ()>;
type RegTagCallback = (tui::InteractTag, Option<InteractCallback>);

#[derive(Debug, Clone)]
struct ModuleControllerTx {
    tx: UnbTx<ctrl::ControllerUpdate>,
}
impl ModuleControllerTx {
    fn set_menu(&self, tag: tui::InteractTag, kind: tui::InteractKind, menu: tui::OpenMenu) {
        self.tx
            .send(ctrl::ControllerUpdate::BarMenu(ctrl::BarMenuUpdate {
                tag,
                kind,
                menu: Some(menu),
            }))
            .ok_or_debug();
    }
}

struct ModuleArgs {
    tui_tx: WatchTx<BarTuiElem>,
    reload_rx: ReloadRx,
    ctrl_tx: ModuleControllerTx,
    tag_callback_tx: UnbTx<RegTagCallback>,
    _unused: (),
}

struct BarModuleFactory {
    reload_tx: ReloadTx,
    ctrl_tx: ModuleControllerTx,
    tag_callback_tx: UnbTx<RegTagCallback>,
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

fn gather_bar_tui(bar_tui: &[BarTuiElem]) -> BarTuiState {
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

    BarTuiState {
        by_monitor: by_monitor
            .into_iter()
            .map(|(k, stack)| (k, stack.build()))
            .collect(),
        fallback: fallback.build(),
    }
}

pub async fn main(
    ctrl_upd_tx: UnbTx<ctrl::ControllerUpdate>,
    mut ctrl_ev_rx: UnbRx<ctrl::ControllerEvent>,
) -> std::process::ExitCode {
    let mut required_tasks = JoinSet::new();
    let mut reload_tx = ReloadTx::new();

    let (tag_callback_tx, mut tag_callback_rx) = unb_chan();
    let mut fac = BarModuleFactory {
        reload_tx: reload_tx.clone(),
        ctrl_tx: ModuleControllerTx {
            tx: ctrl_upd_tx.clone(),
        },
        tag_callback_tx,
        tasks: JoinSet::new(),
    };

    let callbacks = Arc::new(std::sync::Mutex::new(HashMap::new()));
    {
        let callbacks = callbacks.clone();
        tokio::spawn(async move {
            while let Some((tag, cb)) = tag_callback_rx.next().await {
                if let Some(cb) = cb {
                    lock_mutex(&callbacks).insert(tag, cb);
                } else {
                    lock_mutex(&callbacks).remove(&tag);
                }
            }
        });
    }

    {
        let mut reload_tx = reload_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = ctrl_ev_rx.next().await {
                match ev {
                    ctrl::ControllerEvent::Interact(ctrl::TuiInteract { kind, tag, .. }) => {
                        let callback: Option<InteractCallback> =
                            lock_mutex(&callbacks).get(&tag).cloned();
                        callback.inspect(|cb| cb.call(InteractArgs { kind }));
                    }
                    ctrl::ControllerEvent::ReloadRequest => {
                        reload_tx.reload();
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
                muted_sym: tui::RawPrint::plain(" ").into(),
                unmuted_sym: tui::RawPrint::center_symbol("", 2).into(),
            },
            pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn_with(
            PulseModuleCtx {
                pulse,
                device_kind: clients::pulse::PulseDeviceKind::Sink,
                muted_sym: tui::RawPrint::plain(" ").into(),
                unmuted_sym: tui::RawPrint::plain(" ").into(),
            },
            pulse_module,
        ),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(ppd_module),
        fac.spawn(energy_module),
        fac.fixed(BarTuiElem::Spacing(3)),
        fac.spawn(time_module),
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
                let tui = gather_bar_tui(&bar_tui_rx_inner.borrow_and_update());
                ctrl_upd_tx
                    .send(ctrl::ControllerUpdate::BarTui(tui))
                    .ok_or_debug();
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
                let tui = tui::RawPrint::plain(ws.name.clone());
                let tui_active = tui
                    .clone()
                    .styled(tui::Style {
                        fg: Some(tui::Color::Green),
                        ..Default::default()
                    })
                    .map_display(|it| it.to_string().into());

                let with_hover = |base: tui::RawPrint<Arc<str>>| {
                    let hovered = base.clone().styled(tui::Style {
                        modifier: tui::Modifier {
                            underline: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                    tui::Elem::from(base).interactive_hover(tag.clone(), hovered.into())
                };

                let on_interact = InteractCallback::from_fn_ctx(
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

                (with_hover(tui), with_hover(tui_active))
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

    let mk_menu = || {
        let now = chrono::Local::now().date_naive();
        let title = now.format("%B %Y").to_string();

        let first_day_offset = now
            .with_day(1)
            .map(|first| first.weekday() as u16)
            .context("Failed to set day")
            .ok_or_log()?;

        let num_weeks = usize::div_ceil(
            usize::from(now.num_days_in_month()) + usize::from(first_day_offset),
            7,
        );

        // FIXME: Simplify
        let mut lines: Vec<_> = std::iter::repeat_n(
            std::array::repeat::<_, 7>(tui::Elem::empty().with_min_size(tui::Vec2 {
                x: 2,
                ..Default::default()
            })),
            num_weeks,
        )
        .collect();

        for n0 in 0u16..now.num_days_in_month().into() {
            let n1 = n0 + 1;
            let day = now
                .with_day(n1.into())
                .with_context(|| format!("Failed to set day {n1}"))
                .ok_or_log()?;

            let week_in_month = (first_day_offset + n0) / 7;
            let item = &mut lines[usize::from(week_in_month)][day.weekday() as usize];
            *item = {
                let it = tui::RawPrint::plain(format!("{n1:>2}"));
                if now == day {
                    it.styled(tui::Style {
                        fg: Some(tui::Color::Green),
                        ..Default::default()
                    })
                    .map_display(|styled| styled.to_string())
                    .into()
                } else {
                    it.into()
                }
            };
        }
        let weekday_line =
            ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map(|d| tui::RawPrint::plain(d).into());

        let elem = tui::Elem::build_stack(tui::Axis::Y, |vstack| {
            vstack.fit(
                tui::RawPrint::plain(title)
                    .styled(tui::Style {
                        modifier: tui::Modifier {
                            bold: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into(),
            );
            for line in std::iter::once(weekday_line).chain(lines) {
                vstack.fit(tui::Elem::build_stack(tui::Axis::X, |hstack| {
                    let mut first = true;
                    for day in line {
                        if !first {
                            hstack.spacing(1);
                        }
                        first = false;

                        hstack.fit(day);
                    }
                }));
            }
        });
        Some(tui::OpenMenu::tooltip(elem))
    };
    let interact_tag = mk_fresh_interact_tag();
    let _menu_task = AbortOnDropHandle::new({
        let interact_tag = interact_tag.clone();
        tokio::spawn(async move {
            if let Some(menu) = mk_menu() {
                ctrl_tx.set_menu(interact_tag.clone(), tui::InteractKind::Hover, menu);
            }
            // FIXME: Schedule daily
        })
    });

    let mut prev_minutes = 61;
    loop {
        let now = chrono::Local::now();
        let minute = now.minute();
        if prev_minutes != minute {
            let tui = tui::RawPrint::plain(now.format("%H:%M %d/%m").to_string());
            let tui = tui::Elem::from(tui).interactive(interact_tag.clone());

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

    let on_interact = InteractCallback::from_fn_ctx(pulse.clone(), move |pulse, interact| {
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

        tui_tx.send_replace(BarTuiElem::Shared(
            tui::Elem::build_stack(tui::Axis::X, |stack| {
                stack.fit(if muted {
                    muted_sym.clone()
                } else {
                    unmuted_sym.clone()
                });
                stack.fit(
                    tui::RawPrint::plain(format!("{:>3}%", (volume * 100.0).round() as u32)).into(),
                );
            })
            .interactive(interact_tag.clone()),
        ));
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
                let tui = tui::RawPrint::plain(energy.as_str());
                let tui = tui::Elem::from(tui).interactive(interact_tag.clone());
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
                let menu = tui::OpenMenu::tooltip(tui::RawPrint::plain(text.as_str()).into());
                ctrl_tx.set_menu(interact_tag.clone(), tui::InteractKind::Hover, menu);
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

    let on_interact = InteractCallback::from_fn_ctx(ppd.clone(), |ppd, interact| {
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
        ctrl_tx.set_menu(
            interact_tag.clone(),
            tui::InteractKind::Click(tui::MouseButton::Left),
            tui::OpenMenu::tooltip(
                tui::PlainLines::new(profile.as_deref().unwrap_or("No profile")).into(),
            ),
        );

        let icon: tui::Elem = match profile.as_deref() {
            Some("balanced") => tui::RawPrint::plain(" ").into(),
            Some("performance") => tui::RawPrint::center_symbol(" ", 2).into(),
            Some("power-saver") => tui::RawPrint::plain(" ").into(),
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

        let tui = tui::Elem::build_stack(tui::Axis::X, |stack| {
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
                    let header = tui::PlainLines::new(title).styled(tui::Style {
                        modifier: tui::Modifier {
                            bold: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                    let header = tui::Elem::build_stack(tui::Axis::X, |hstack| {
                        hstack.fill(1, tui::Elem::empty());
                        hstack.fit(header.into());
                        hstack.fill(1, tui::Elem::empty());
                    });
                    let tui = tui::Elem::build_stack(tui::Axis::Y, |vstack| {
                        vstack.fit(header);
                        vstack.fit(tui::PlainLines::new(description).into());
                    });
                    let menu = tui::OpenMenu::tooltip(tui);
                    ctrl_tx.set_menu(tag.clone(), tui::InteractKind::Hover, menu);
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
                        let icb = InteractCallback::from_fn(move |interact| {
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

                    let menu = tui::OpenMenu::context(tui::Elem::build_block(|block| {
                        block.set_borders_at(tui::Borders::all());
                        block.set_style(tui::Style {
                            fg: Some(tui::Color::DarkGrey),
                            ..Default::default()
                        });
                        block.set_lines(tui::LineSet::thick());
                        block.set_inner(menu_tui);
                    }));

                    ctrl_tx.set_menu(
                        tag.clone(),
                        tui::InteractKind::Click(tui::MouseButton::Right),
                        menu,
                    );
                }

                // FIXME: Handle the other options
                // FIXME: Why are we showing all icons?
                for system_tray::item::IconPixmap {
                    width,
                    height,
                    pixels,
                } in item.icon_pixmap.as_deref().unwrap_or(&[])
                {
                    let mut img = match image::RgbaImage::from_vec(
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
                    for image::Rgba(pixel) in img.pixels_mut() {
                        *pixel = u32::from_be_bytes(*pixel).rotate_left(8).to_be_bytes();
                    }

                    stack.fit(
                        tui::Elem::image(
                            img, //
                            tui::ImageSizeMode::FillAxis(tui::Axis::Y, 1),
                        )
                        .interactive(tag.clone()),
                    );
                    stack.spacing(1);
                }
            }
        });
        tui_tx.send_replace(BarTuiElem::Shared(tui));
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
                let elem = tui::Elem::build_stack(tui::Axis::X, |stack| {
                    stack.spacing(depth + 1);
                    if let Some(icon) = icon_data
                        && let Some(img) =
                            image::load_from_memory_with_format(icon, image::ImageFormat::Png)
                                .context("Systray icon has invalid png data")
                                .ok_or_log()
                    {
                        stack.fit(tui::Elem::image(
                            img.into_rgba8(),
                            tui::ImageSizeMode::FillAxis(tui::Axis::Y, 1),
                        ));
                        stack.spacing(1);
                    }
                    stack.fit(tui::PlainLines::new(label).into())
                });

                // FIXME: Add hover
                elem.interactive(mk_interact(*id))
            }

            _ => {
                log::error!("Unhandled menu item: {item:#?}");
                return None;
            }
        };

        Some(if item.submenu.is_empty() {
            main_elem
        } else {
            tui::Elem::build_stack(tui::Axis::Y, |stack| {
                stack.fit(main_elem);
                stack.fit(tray_menu_to_tui(depth + 1, &item.submenu, mk_interact));
            })
        })
    }

    fn tray_menu_to_tui(
        depth: u16,
        items: &[system_tray::menu::MenuItem],
        mk_interact: &impl Fn(i32) -> tui::InteractTag,
    ) -> tui::Elem {
        tui::Elem::build_stack(tui::Axis::Y, |stack| {
            for item in items {
                if let Some(item) = tray_menu_item_to_tui(depth, item, mk_interact) {
                    stack.fit(item)
                }
            }
        })
    }
}
