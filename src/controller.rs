use tempfile::TempDir;

use std::{collections::HashMap, ffi::OsString, sync::Arc, time::Duration};

use anyhow::Context;
use futures::{Stream, StreamExt};
use tokio::{sync::watch, task::JoinSet};
use tokio_util::{sync::CancellationToken, time::FutureExt as _};

use crate::{
    api,
    inst::{TermEvent, TermUpdate},
    monitors::MonitorInfo,
    tui,
    utils::{CancelDropGuard, ResultExt, with_mutex_lock},
};

#[derive(Debug)]
struct BarMenu {
    kind: api::MenuKind,
    tui: tui::Elem,
}
type BarMenus = HashMap<tui::InteractTag, HashMap<tui::InteractKind, watch::Sender<BarMenu>>>;

#[derive(Debug, Clone)]
struct BarTuiState {
    tui: tui::Elem,
    hidden: bool,
}
#[derive(Debug, Clone)]
struct BarTuiStateTx {
    tui: watch::Sender<tui::Elem>,
    hidden: watch::Sender<bool>,
}
#[derive(Debug)]
struct BarTuiStates {
    by_monitor: HashMap<Arc<str>, watch::Sender<BarTuiStateTx>>,
    defaults: BarTuiStateTx,
}
impl BarTuiStates {
    fn get_or_mk_monitor(&mut self, name: Arc<str>) -> &mut watch::Sender<BarTuiStateTx> {
        self.by_monitor
            .entry(name)
            .or_insert_with(|| watch::Sender::new(self.defaults.clone()))
    }
}

async fn run_controller(
    update_rx: impl Stream<Item = api::ControllerUpdate> + Send + 'static,
    event_tx: tokio::sync::mpsc::UnboundedSender<api::ControllerEvent>,
) {
    let mut required_tasks = tokio::task::JoinSet::new();

    let bar_tui_states = Arc::new(std::sync::Mutex::new(BarTuiStates {
        by_monitor: Default::default(),
        defaults: BarTuiStateTx {
            tui: watch::Sender::new(tui::Elem::empty()),
            hidden: watch::Sender::new(false),
        },
    }));
    let bar_menus_tx = watch::Sender::new(BarMenus::default());
    required_tasks.spawn(run_controller_inner(
        bar_tui_states.clone(),
        bar_menus_tx.subscribe(),
        event_tx.clone(),
    ));

    required_tasks.spawn(async move {
        tokio::pin!(update_rx);
        while let Some(update) = update_rx.next().await {
            match update {
                api::ControllerUpdate::RegisterMenu(api::RegisterMenu {
                    on_tag,
                    on_kind,
                    tui,
                    menu_kind,
                    options:
                        api::RegisterMenuOpts {
                            #[expect(deprecated)]
                            __non_exhaustive_struct_update,
                        },
                }) => {
                    use std::collections::hash_map::Entry;
                    let menu = BarMenu {
                        kind: menu_kind,
                        tui,
                    };

                    bar_menus_tx.send_if_modified(|menus| match menus.entry(on_tag) {
                        Entry::Occupied(mut entry) => match entry.get_mut().entry(on_kind) {
                            Entry::Occupied(cur) => {
                                cur.get().send_replace(menu);
                                false
                            }
                            cur => {
                                cur.insert_entry(watch::Sender::new(menu));
                                true
                            }
                        },
                        Entry::Vacant(entry) => {
                            entry.insert_entry(HashMap::from_iter([(
                                on_kind,
                                watch::Sender::new(menu),
                            )]));
                            true
                        }
                    });
                }
                api::ControllerUpdate::UpdateBars(api::BarSelection::All, update) => {
                    fn doit<T>(
                        bar_tui_states: &mut BarTuiStates,
                        val: T,
                        get_tx: impl Fn(&mut BarTuiStateTx) -> &mut watch::Sender<T>,
                    ) {
                        let default_tx = get_tx(&mut bar_tui_states.defaults);
                        default_tx.send_replace(val);
                        for state in bar_tui_states.by_monitor.values_mut() {
                            state.send_modify(|it| *get_tx(it) = default_tx.clone());
                        }
                    }
                    with_mutex_lock(&bar_tui_states, |bar_tui_states| {
                        // TODO: Keep unknown monitors around only for a few minutes
                        match update {
                            api::BarUpdate::SetTui(api::SetBarTui {
                                tui,
                                options:
                                    api::SetBarTuiOpts {
                                        #[expect(deprecated)]
                                        __non_exhaustive_struct_update,
                                    },
                            }) => {
                                doit(bar_tui_states, tui, |state| &mut state.tui);
                            }
                            api::BarUpdate::Hide | api::BarUpdate::Show => {
                                doit(
                                    bar_tui_states,
                                    matches!(update, api::BarUpdate::Hide),
                                    |state| &mut state.hidden,
                                );
                            }
                        }
                    });
                }
                api::ControllerUpdate::UpdateBars(
                    api::BarSelection::OnMonitor { monitor_name },
                    update,
                ) => {
                    fn doit<T>(
                        bar_tui_states: &mut BarTuiStates,
                        monitor: Arc<str>,
                        val: T,
                        get_tx: impl Fn(&mut BarTuiStateTx) -> &mut watch::Sender<T>,
                    ) {
                        let default_tx = get_tx(&mut bar_tui_states.defaults).clone();
                        bar_tui_states
                            .get_or_mk_monitor(monitor.clone())
                            .send_if_modified(|state| {
                                let tx = get_tx(state);
                                if tx.same_channel(&default_tx) {
                                    *tx = watch::Sender::new(val);
                                    true
                                } else {
                                    tx.send_replace(val);
                                    false
                                }
                            });
                    }
                    with_mutex_lock(&bar_tui_states, |bar_tui_states| {
                        // TODO: Keep unknown monitors around only for a few minutes
                        match update {
                            api::BarUpdate::SetTui(api::SetBarTui {
                                tui,
                                options:
                                    api::SetBarTuiOpts {
                                        #[expect(deprecated)]
                                        __non_exhaustive_struct_update,
                                    },
                            }) => {
                                doit(bar_tui_states, monitor_name, tui, |state| &mut state.tui);
                            }
                            api::BarUpdate::Hide | api::BarUpdate::Show => {
                                doit(
                                    bar_tui_states,
                                    monitor_name,
                                    matches!(update, api::BarUpdate::Hide),
                                    |state| &mut state.hidden,
                                );
                            }
                        }
                    });
                }
                api::ControllerUpdate::SetDefaultTui(api::SetBarTui {
                    tui,
                    options:
                        api::SetBarTuiOpts {
                            #[expect(deprecated)]
                            __non_exhaustive_struct_update,
                        },
                }) => with_mutex_lock(&bar_tui_states, |bar_tui_states| {
                    bar_tui_states.defaults.tui.send_replace(tui);
                }),
            }
        }
    });

    if let Some(res) = required_tasks.join_next().await {
        res.ok_or_log();
    }
}

async fn run_controller_inner(
    bar_tui_states: Arc<std::sync::Mutex<BarTuiStates>>,
    bar_menus_rx: watch::Receiver<BarMenus>,
    event_tx: tokio::sync::mpsc::UnboundedSender<api::ControllerEvent>,
) {
    // TODO: Consider moving this to BarTuiStates
    let mut monitors_auto_cancel = HashMap::new();

    let mut monitor_rx = crate::monitors::connect();

    while let Some(ev) = monitor_rx.next().await {
        with_mutex_lock(&bar_tui_states, |bar_tui_states| {
            for monitor in ev.removed() {
                drop(monitors_auto_cancel.remove(monitor));
                bar_tui_states.by_monitor.remove(monitor);
            }
            for monitor in ev.added_or_changed() {
                let bar_state_tx = bar_tui_states.get_or_mk_monitor(monitor.name.clone());

                let cancel = CancellationToken::new();
                tokio::spawn(run_monitor(RunMonitorArgs {
                    monitor: monitor.clone(),
                    cancel_monitor: cancel.clone(),
                    bar_state_tx: bar_state_tx.clone(),
                    bar_menus_rx: bar_menus_rx.clone(),
                    event_tx: event_tx.clone(),
                }));
                monitors_auto_cancel.insert(monitor.name.clone(), CancelDropGuard::from(cancel));
            }
        });
    }
}

#[derive(Clone)]
struct RunMonitorArgs {
    monitor: MonitorInfo,
    cancel_monitor: CancellationToken,
    bar_state_tx: watch::Sender<BarTuiStateTx>,
    bar_menus_rx: watch::Receiver<BarMenus>,
    event_tx: tokio::sync::mpsc::UnboundedSender<api::ControllerEvent>,
}
async fn run_monitor(mut args: RunMonitorArgs) {
    let monitor = args.monitor.name.clone();
    let _auto_cancel = CancelDropGuard::from(args.cancel_monitor.clone());

    loop {
        const TIMEOUT: Duration = Duration::from_secs(20);
        if let Some(()) = try_run_monitor(&mut args)
            .await
            .with_context(|| format!("Failed to run task. Retrying in {}s", TIMEOUT.as_secs()))
            .ok_or_log()
        {
            break;
        }
        tokio::time::sleep(TIMEOUT).await;
    }
    log::debug!("Exiting panel manager for monitor {monitor:?}");
}

struct Term {
    term_ev_rx: tokio::sync::mpsc::UnboundedReceiver<TermEvent>,
    term_upd_tx: tokio::sync::mpsc::UnboundedSender<TermUpdate>,
    sizes: tui::Sizes,
    layout: Option<tui::RenderedLayout>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermKind {
    Menu,
    Bar,
}
enum Upd {
    Noop,
    Term(TermKind, TermEvent),
}

struct StartedMonitorEnv {
    bar: Term,
    menu: Term,
    intern_upd_rx: tokio::sync::mpsc::UnboundedReceiver<Upd>,
    bar_tui_rx: watch::Receiver<tui::Elem>,
    bar_hide_rx: watch::Receiver<bool>,
    event_tx: tokio::sync::mpsc::UnboundedSender<api::ControllerEvent>,
}

async fn try_run_monitor(args: &mut RunMonitorArgs) -> anyhow::Result<()> {
    log::debug!("Starting panel manager for monitor {:?}", args.monitor);

    let mut required_tasks = JoinSet::<anyhow::Result<std::convert::Infallible>>::new();
    let cancel = args.cancel_monitor.child_token();
    let _auto_cancel = CancelDropGuard::from(cancel.clone());
    let env = try_init_monitor(args, &mut required_tasks, &cancel).await?;
    required_tasks.spawn(run_monitor_main(
        args.monitor.clone(),
        env,
        args.bar_menus_rx.clone(),
    ));

    if let Some(Some(res)) = required_tasks
        .join_next()
        .with_cancellation_token(&cancel)
        .await
    {
        match res {
            Err(join_err) => {
                if join_err.is_cancelled() {
                    Ok(())
                } else {
                    Err(join_err).context("Failure joining instance task")
                }
            }
            Ok(Err(task_err)) => Err(task_err).context("Failure running instance task"),
        }
    } else {
        Ok(())
    }
}

// FIXME: Add to update enum
const EDGE: &str = "top";

/// Adds an extra line and centers the content of the menu with padding of half a cell.
const VERTICAL_PADDING: bool = true;
const HORIZONTAL_PADDING: u16 = 4;

#[derive(Debug)]
struct ShowMenu {
    kind: api::MenuKind,
    pix_location: tui::Vec2<u32>,
    cached_size: tui::Vec2<u16>,
    sizing: tui::SizingArgs,
    tui: tui::Elem,
    receiver: watch::Receiver<BarMenu>,
}
impl ShowMenu {
    fn mk_recv(
        mut receiver: watch::Receiver<BarMenu>,
        pix_location: tui::Vec2<u32>,
        env: &StartedMonitorEnv,
    ) -> Self {
        let sizing = tui::SizingArgs {
            font_size: env.menu.sizes.font_size(),
        };
        let (tui, kind) = {
            let BarMenu { tui, kind } = &*receiver.borrow_and_update();
            (tui.clone(), kind.internal_clone())
        };
        Self {
            cached_size: tui::calc_min_size(&tui, &sizing),
            sizing,
            tui,
            kind,
            pix_location,
            receiver,
        }
    }
}
// FIXME: This function is way too large
async fn run_monitor_main(
    monitor: MonitorInfo,
    mut env: StartedMonitorEnv,
    bar_menus_rx: watch::Receiver<BarMenus>,
) -> anyhow::Result<std::convert::Infallible> {
    let mut show_menu = None::<ShowMenu>;
    let mut bar_tui_state = BarTuiState {
        tui: tui::Elem::empty(),
        hidden: false,
    };
    loop {
        let mut rerender_menu = false;
        let mut bar_tui_changed = false;
        let mut bar_vis_changed = false;

        let upd = tokio::select! {
            Some(ev) = env.bar.term_ev_rx.recv() => Upd::Term(TermKind::Bar, ev),
            Some(ev) = env.menu.term_ev_rx.recv() => Upd::Term(TermKind::Menu, ev),
            Some(upd) = env.intern_upd_rx.recv() => upd,
            Some(Ok(())) = async { Some(show_menu.as_mut()?.receiver.changed().await) } => {
                let ShowMenu { pix_location, receiver, .. } = show_menu.unwrap();
                show_menu = Some(ShowMenu::mk_recv(receiver, pix_location, &env));
                rerender_menu = true;
                Upd::Noop
            },
            Ok(()) = env.bar_hide_rx.changed() => {
                let hidden = *env.bar_hide_rx.borrow_and_update();
                bar_vis_changed = hidden != std::mem::replace(&mut bar_tui_state.hidden, hidden);
                Upd::Noop
            }
            Ok(()) = env.bar_tui_rx.changed() => {
                bar_tui_state.tui = env.bar_tui_rx.borrow_and_update().clone();
                bar_tui_changed = true;
                Upd::Noop
            },
        };
        match upd {
            Upd::Noop => {}
            Upd::Term(term_kind, TermEvent::Crossterm(ev)) => match ev {
                crossterm::event::Event::Mouse(ev) => {
                    let Some(layout) = (match term_kind {
                        TermKind::Menu => env.menu.layout.as_mut(),
                        TermKind::Bar => env.bar.layout.as_mut(),
                    }) else {
                        continue;
                    };

                    let tui::MouseEventResult {
                        kind,
                        tag,
                        empty,
                        changed,
                        rerender,
                        has_hover,
                        pix_location,
                    } = layout.interpret_mouse_event(ev, env.bar.sizes.font_size());
                    let is_hover = kind == tui::InteractKind::Hover;

                    if term_kind == TermKind::Menu
                        && let Some(menu) = &show_menu
                        && matches!(&menu.kind, api::MenuKind::Tooltip)
                    {
                        show_menu = None;
                        rerender_menu = true;
                    }

                    if rerender {
                        match term_kind {
                            TermKind::Menu => rerender_menu = true,
                            TermKind::Bar => bar_tui_changed = true,
                        }
                    }

                    if changed || !is_hover {
                        if term_kind == TermKind::Bar {
                            if empty
                                && let Some(menu) = &show_menu
                                && (!is_hover || matches!(&menu.kind, api::MenuKind::Tooltip))
                            {
                                show_menu = None;
                                rerender_menu = true;
                            }

                            let mut menu = tag.as_ref().and_then(|tag| {
                                bar_menus_rx
                                    .borrow()
                                    .get(tag)
                                    .and_then(|tag_menus| tag_menus.get(&kind))
                                    .cloned()
                            });
                            // If the element has a hovered version, then it needs to have a menu,
                            // since we have no way to tell when the bar loses focus.
                            // TODO: This is not necessary if the bar has focus-policy=on-demand,
                            // but that is currently not supported.
                            if has_hover && menu.is_none() {
                                menu = Some(watch::Sender::new(BarMenu {
                                    kind: api::MenuKind::Tooltip,
                                    tui: tui::Elem::empty().with_min_size(tui::Size {
                                        width: 1,
                                        height: 1,
                                    }),
                                }));
                            }

                            if let Some(menu) = menu {
                                show_menu =
                                    Some(ShowMenu::mk_recv(menu.subscribe(), pix_location, &env));
                                rerender_menu = true;
                            }
                        }

                        if let Some(tag) = tag {
                            env.event_tx
                                .send(api::ControllerEvent::Interact(api::InteractEvent {
                                    kind,
                                    tag,
                                }))
                                .ok_or_debug();
                        }
                    }
                }
                _ => {
                    //
                }
            },
            Upd::Term(TermKind::Menu, TermEvent::Sizes(sizes)) => {
                if sizes.font_size() != env.menu.sizes.font_size() {
                    rerender_menu = true;
                }
                env.menu.sizes = sizes;
            }
            Upd::Term(TermKind::Bar, TermEvent::Sizes(sizes)) => {
                env.bar.sizes = sizes;
                bar_tui_changed = true;
            }
            Upd::Term(term_kind, TermEvent::FocusChange { is_focused }) => {
                // FIXME: This only works because the menu doesnt lose focus while we are
                // on the bar, which forbids focus.
                if !is_focused && term_kind == TermKind::Menu {
                    show_menu = None;
                    if let Some(layout) = &mut env.bar.layout
                        && layout.ext_focus_loss()
                    {
                        bar_tui_changed = true;
                    }
                }
            }
        }

        if rerender_menu {
            if show_menu.is_none()
                && let Some(layout) = &mut env.bar.layout
                && layout.ext_focus_loss()
            {
                bar_tui_changed = true;
            }

            if let Some(&ShowMenu {
                pix_location: location,
                cached_size: cached_tui_size,
                ref tui,
                ref sizing,
                kind: _,
                receiver: _,
            }) = show_menu.as_ref()
            {
                // HACK: This minimizes the rounding error for some reason (as far as I can tell).
                let scale = (monitor.scale * 1000.0).ceil() / 1000.0;

                // NOTE: There is no absolute positioning system, nor a way to directly specify the
                // geometry (since this is controlled by the compositor). So we have to get creative by
                // using the right and left margin to control both position and size of the panel.

                let lines = cached_tui_size.y.saturating_add(VERTICAL_PADDING.into());

                // Find the distance between window edge and center
                let half_pix_w = {
                    let cell_pix_w = u32::from(env.menu.sizes.font_size().x);
                    let cell_w = cached_tui_size.x + HORIZONTAL_PADDING;
                    let pix_w = u32::from(cell_w) * cell_pix_w;
                    pix_w.div_ceil(2)
                };

                // Clamp position such that we fit. Note that this does not guarantee
                // that there is enough space for the entire width.
                let x = location.x.clamp(
                    half_pix_w, //
                    monitor.width.saturating_sub(half_pix_w),
                );

                // The left margin should be such that half the space is between
                // left margin and x. Use saturating_sub so that the left
                // margin becomes zero if the width would reach outside the screen.
                let mleft = x.saturating_sub(half_pix_w);

                // The right margin is calculated the same way, but starting from the right edge.
                let mright = (monitor.width - x).saturating_sub(half_pix_w);

                // The font size (on which cell->pixel conversion is based) and the monitor's
                // size are in physical pixels. This makes sense because different monitors can
                // have different scales, and the application should not be affected by that
                // (this is not x11 after all).
                // However, panels are bound to a monitor and the margins are in scaled pixels,
                // so we have to make this correction.
                let margin_left = (f64::from(mleft) / scale) as u32;
                let margin_right = (f64::from(mright) / scale) as u32;

                env.menu
                    .term_upd_tx
                    .send(TermUpdate::RemoteControl(vec![
                        "resize-os-window".into(),
                        "--incremental".into(),
                        "--action=os-panel".into(),
                        format!("margin-left={margin_left}").into(),
                        format!("margin-right={margin_right}").into(),
                        format!("lines={lines}").into(),
                    ]))
                    .ok_or_log();

                let mut buf = Vec::new();

                // NOTE: The terminal might not be done resizing at this point,
                // which would cause issues if passing the terminal's size here.
                // Passing the tui's desired size sidesteps this because kitty
                // will rerender it correctly once the resize is done.
                if let Some(layout) = tui::render(
                    tui,
                    tui::Area {
                        size: cached_tui_size,
                        pos: tui::Vec2 {
                            x: HORIZONTAL_PADDING / 2,
                            y: 0,
                        },
                    },
                    &mut buf,
                    sizing,
                    env.menu.layout.as_ref(),
                )
                .context("Failed to draw menu")
                .ok_or_log()
                {
                    env.menu.layout = Some(layout);
                    env.menu
                        .term_upd_tx
                        .send(TermUpdate::Print(buf))
                        .ok_or_log();
                    env.menu.term_upd_tx.send(TermUpdate::Flush).ok_or_log();
                }
            }

            // FIXME: Only send when necessary
            env.menu
                .term_upd_tx
                .send(set_vis_update(show_menu.is_some()))
                .ok_or_debug();
        }

        if !bar_tui_state.hidden && (bar_vis_changed || bar_tui_changed) {
            let mut buf = Vec::new();

            let Some(layout) = tui::render(
                &bar_tui_state.tui,
                tui::Area {
                    size: env.bar.sizes.cell_size,
                    pos: Default::default(),
                },
                &mut buf,
                &tui::SizingArgs {
                    font_size: env.bar.sizes.font_size(),
                },
                env.bar.layout.as_ref(),
            )
            .context("Failed to render bar")
            .ok_or_log() else {
                continue;
            };
            env.bar.layout = Some(layout);

            env.bar
                .term_upd_tx
                .send(TermUpdate::Print(buf))
                .ok_or_debug();
            env.bar.term_upd_tx.send(TermUpdate::Flush).ok_or_debug();
        }
        if bar_vis_changed {
            env.bar
                .term_upd_tx
                .send(set_vis_update(!bar_tui_state.hidden))
                .ok_or_debug();
        }
    }
}

fn set_vis_update(vis: bool) -> TermUpdate {
    let action = if vis { "show" } else { "hide" };
    TermUpdate::RemoteControl(vec![
        "resize-os-window".into(),
        format!("--action={}", action).into(),
    ])
}

async fn init_term(
    sock_path: std::path::PathBuf,
    log_name: String,
    extra_args: impl IntoIterator<Item = OsString>,
    extra_envs: impl IntoIterator<Item = (OsString, OsString)>,
    cancel: &CancellationToken,
) -> anyhow::Result<Term> {
    let (term_upd_tx, mut term_upd_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };
    let (term_ev_tx, mut term_ev_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };

    crate::inst::start_generic_panel(
        &sock_path,
        &log_name,
        futures::stream::poll_fn(move |cx| term_upd_rx.poll_recv(cx)),
        extra_args,
        extra_envs,
        term_ev_tx,
        cancel.clone(),
    )
    .await?;

    let sizes = loop {
        match term_ev_rx.recv().await {
            Some(TermEvent::Sizes(sizes)) => break sizes,
            Some(ev) => {
                log::error!("Ignoring term event {ev:?}. The first event should be _::Sizes");
            }
            None => {
                anyhow::bail!("Failure receiving initial size event from terminal (channel closed)")
            }
        }
    };

    anyhow::Ok(Term {
        sizes,
        layout: Default::default(),
        term_ev_rx,
        term_upd_tx,
    })
}

async fn try_init_monitor(
    args: &RunMonitorArgs,
    required_tasks: &mut JoinSet<anyhow::Result<std::convert::Infallible>>,
    cancel: &CancellationToken,
) -> anyhow::Result<StartedMonitorEnv> {
    let monitor = args.monitor.clone();

    let (intern_upd_tx, intern_upd_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };

    let tmpdir = tokio::task::spawn_blocking(TempDir::new).await??;

    let bar_fut = init_term(
        tmpdir.path().join("bar-term-socket.sock"),
        format!("BAR@{}", monitor.name),
        [
            format!("--output-name={}", monitor.name).into(),
            // Allow remote control
            "-o=allow_remote_control=socket-only".into(),
            "--listen-on=unix:/tmp/kitty-bar-panel.sock".into(),
            // Allow logging to $KITTY_STDIO_FORWARDED
            "-o=forward_stdio=yes".into(),
            // Do not use the system's kitty.conf
            "--config=NONE".into(),
            // Basic look of the bar
            "-o=foreground=white".into(),
            "-o=background=black".into(),
            // location of the bar
            format!("--edge={}", EDGE).into(),
            // disable hiding the mouse
            "-o=mouse_hide_wait=0".into(),
        ],
        [],
        cancel,
    );

    let menu_fut = async {
        let watcher_py = tmpdir.path().join("menu_watcher.py");

        tokio::fs::write(&watcher_py, include_bytes!("menu_watcher.py")).await?;

        let watcher_sock_path = tmpdir.path().join("menu_watcher.sock");
        let watcher_sock = tokio::net::UnixListener::bind(&watcher_sock_path)?;

        let menu = init_term(
            tmpdir.path().join("menu-term-socket.sock"),
            format!("MENU@{}", monitor.name),
            [
                {
                    let mut arg = OsString::from("-o=watcher=");
                    arg.push(watcher_py);
                    arg
                },
                format!("--output-name={}", monitor.name).into(),
                // Configure remote control via socket
                "-o=allow_remote_control=socket-only".into(),
                "--listen-on=unix:/tmp/kitty-bar-menu-panel.sock".into(),
                // Allow logging to $KITTY_STDIO_FORWARDED
                "-o=forward_stdio=yes".into(),
                // Do not use the system's kitty.conf
                "--config=NONE".into(),
                // Basic look of the menu
                "-o=background_opacity=0.85".into(),
                "-o=background=black".into(),
                "-o=foreground=white".into(),
                // Center within leftover pixels if cell size does not divide window size.
                "-o=placement_strategy=center".into(),
                // location of the menu
                "--edge=top".into(),
                // disable hiding the mouse
                "-o=mouse_hide_wait=0".into(),
                // Window behavior of the menu panel. Makes panel
                // act as an overlay on top of other windows.
                // We do not want tilers to dedicate space to it.
                // Taken from the args that quick-access-terminal uses.
                "--exclusive-zone=0".into(),
                "--override-exclusive-zone".into(),
                "--layer=overlay".into(),
                // Focus behavior of the panel. Since we cannot tell from
                // mouse events alone when the cursor leaves the panel
                // (since terminal mouse capture only gives us mouse
                // events inside the panel), we need external support for
                // hiding it automatically. We use a watcher to be able
                // to reset the menu state when this happens.
                "--focus-policy=on-demand".into(),
                "--hide-on-focus-loss".into(),
                // Since we control resizes from the program and not from
                // a somewhat continuous drag-resize, debouncing between
                // resize and reloads is completely inappropriate and
                // just results in a larger delay between resize and
                // the old menu content being replaced with the new one.
                "-o=resize_debounce_time=0 0".into(),
                // TODO: Mess with repaint_delay, input_delay
            ],
            [("BAR_MENU_WATCHER_SOCK".into(), watcher_sock_path.into())],
            cancel,
        )
        .await?;
        // NOTE: Never pass start-as-hidden!
        menu.term_upd_tx
            .send(TermUpdate::RemoteControl(vec![
                "resize-os-window".into(),
                "--action=hide".into(),
            ]))
            .ok_or_log();
        if VERTICAL_PADDING {
            // HACK: For some reason, using half font height padding at top and bottom
            // shrinks the height by 2 cells. This way of doing it only works assuming
            // that we do not have more than 1 pixel to spare for the padding and it
            // can only be used for vertical padding of 1 cell in total.
            menu.term_upd_tx
                .send(TermUpdate::RemoteControl(vec![
                    "set-spacing".into(),
                    "padding-top=1".into(),
                    "padding-bottom=1".into(),
                ]))
                .ok_or_log();
        }

        let (s, _) = watcher_sock.accept().await?;
        anyhow::Ok((menu, s))
    };

    let res = async { tokio::try_join!(bar_fut, menu_fut) }
        .timeout(Duration::from_secs(10))
        .await;

    // We have connected to the sockets, there is no need to keep the files around.
    tokio::task::spawn_blocking(move || drop(tmpdir));

    let (bar, (menu, mut watcher_stream)) = res??;

    required_tasks.spawn({
        let upd_tx = intern_upd_tx.clone();
        async move {
            use tokio::io::AsyncReadExt as _;
            loop {
                let byte = watcher_stream
                    .read_u8()
                    .await
                    .context("Failed to read from watcher stream")?;

                let parsed = match byte {
                    0 => Upd::Term(TermKind::Menu, TermEvent::FocusChange { is_focused: false }),
                    1 => Upd::Term(TermKind::Menu, TermEvent::FocusChange { is_focused: true }),
                    _ => {
                        log::error!("Unknown watcher event {byte}");
                        continue;
                    }
                };

                upd_tx.send(parsed).ok_or_log();
            }
        }
    });

    let (bar_tui_tx, bar_tui_rx) = watch::channel(tui::Elem::empty());
    let (bar_hide_tx, bar_hide_rx) = watch::channel(false);
    {
        let mut bar_state_tx_rx = args.bar_state_tx.subscribe();
        required_tasks.spawn(async move {
            'outer: loop {
                let mut tui_rx;
                let mut hide_rx;
                {
                    let BarTuiStateTx { tui, hidden } = &*bar_state_tx_rx.borrow_and_update();

                    tui_rx = tui.subscribe();
                    tui_rx.mark_changed();

                    hide_rx = hidden.subscribe();
                    hide_rx.mark_changed();
                }

                loop {
                    tokio::select! {
                        Ok(()) = tui_rx.changed() => {
                            let tui = tui_rx.borrow_and_update().clone();
                            bar_tui_tx.send_replace(tui);
                        }
                        Ok(()) = hide_rx.changed() => {
                            let hidden = *hide_rx.borrow_and_update();
                            bar_hide_tx.send_replace(hidden);
                        }
                        Ok(()) = bar_state_tx_rx.changed() => {
                            continue 'outer;
                        }
                    }
                }
            }
        });
    }

    Ok(StartedMonitorEnv {
        bar,
        menu,
        intern_upd_rx,
        bar_tui_rx,
        bar_hide_rx,
        event_tx: args.event_tx.clone(),
    })
}

pub(crate) fn ctrl_main() -> Option<std::process::ExitCode> {
    use std::process::ExitCode;

    use anyhow::Context as _;

    crate::logging::init_logger("CONTROLLER".into());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;

    let _guard = runtime.enter();

    // FIXME: Proper arg parsing
    let driver = std::env::args_os().nth(1)?;

    let (mut driver_child, driver_socket) = {
        let socket_dir = tempfile::TempDir::new().ok_or_log()?;
        let sock_path = socket_dir.path().join("driver.sock");
        let socket = std::os::unix::net::UnixListener::bind(&sock_path).ok_or_log()?;

        let child = tokio::process::Command::new(driver)
            .kill_on_drop(true)
            .args(std::env::args_os().skip(2))
            .env(crate::driver_ipc::CONTROLLER_SOCK_PATH_VAR, sock_path)
            .spawn()
            .ok_or_log()?;

        let (conn, _) = socket.accept().ok_or_log()?;

        (child, conn)
    };

    let signals_task = runtime.spawn(async move {
        type SK = tokio::signal::unix::SignalKind;

        let mut tasks = tokio::task::JoinSet::new();

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        for kind in [
            SK::interrupt(),
            SK::quit(),
            SK::alarm(),
            SK::hangup(),
            SK::pipe(),
            SK::terminate(),
            SK::user_defined1(),
            SK::user_defined2(),
        ] {
            let Some(mut signal) = tokio::signal::unix::signal(kind).ok_or_log() else {
                continue;
            };
            let tx = tx.clone();
            tasks.spawn(async move {
                while let Some(()) = signal.recv().await
                    && tx.send(kind).await.is_ok()
                {}
            });
        }
        drop(tx);

        rx.recv()
            .await
            .context("Failed to receive any signals")
            .map(|kind| {
                log::debug!("Received exit signal {kind:?}");
                let code = 128 + kind.as_raw_value();
                ExitCode::from(code as u8)
            })
            .ok_or_log()
    });
    let signals_task = async move {
        signals_task
            .await
            .context("Signal handler failed")
            .ok_or_log()
            .flatten()
    };

    let (update_tx, mut update_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };
    let (event_tx, mut event_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };
    runtime.spawn(crate::api::run_ipc_connection(
        driver_socket,
        move |upd| update_tx.send(upd).ok(),
        async move || event_rx.recv().await,
    ));

    let main_task = tokio_util::task::AbortOnDropHandle::new(runtime.spawn(async move {
        run_controller(
            futures::stream::poll_fn(move |cx| update_rx.poll_recv(cx)),
            event_tx,
        )
        .await;
        // FIXME: Return exit code
        ExitCode::SUCCESS
    }));

    let exit_task = runtime.spawn(async move {
        let wait_res = tokio::select! {
            it = driver_child.wait() => Ok(it),
            join = main_task => {
                let code = join
                    .context("Main task failed")
                    .ok_or_log()
                    .unwrap_or(std::process::ExitCode::FAILURE);
                Err(code)
            },
            Some(code) = signals_task => Err(code),
        };
        let (wait_res, code) = match wait_res {
            Ok(res) => (Some(res), std::process::ExitCode::SUCCESS),
            Err(code) => (
                driver_child
                    .wait()
                    .timeout(std::time::Duration::from_secs(5))
                    .await
                    .context("Driver process failed to exit on its own")
                    .ok_or_log(),
                code,
            ),
        };
        let child_code = match wait_res {
            Some(res) => res
                .ok_or_log()
                .map_or(std::process::ExitCode::FAILURE, |exit| {
                    std::process::ExitCode::from(exit.code().unwrap_or(0) as u8)
                }),
            None => std::process::ExitCode::FAILURE,
        };
        if code == std::process::ExitCode::SUCCESS {
            child_code
        } else {
            code
        }
    });

    runtime.block_on(async move { exit_task.await.ok_or_log() })
}
