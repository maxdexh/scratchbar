pub extern crate image;
pub extern crate log;

mod inst;
mod logging;
mod monitors;
pub mod tui;
pub mod utils;

pub fn init_driver_logger() {
    logging::init_logger(logging::ProcKind::Controller, "DRIVER".into());
}

use inst::{TermEvent, TermUpdate};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use std::{collections::HashMap, ffi::OsString, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use tokio::task::JoinSet;
use tokio_util::{sync::CancellationToken, time::FutureExt as _};

use crate::{
    monitors::MonitorInfo,
    tui::MenuKind,
    utils::{
        CancelDropGuard, ReloadTx, ResultExt, UnbRx, UnbTx, WatchRx, WatchTx, run_or_retry,
        unb_chan, watch_chan,
    },
};
// FIXME: Move everything out of this file

// FIXME: Add to update enum
const EDGE: &str = "top";

/// Adds an extra line and centers the content of the menu with padding of half a cell.
const VERTICAL_PADDING: bool = true;
const HORIZONTAL_PADDING: u16 = 4;

#[derive(Debug, Serialize, Deserialize)]
pub struct BarTuiState {
    // FIXME: Use Option<Elem> to hide, start hidden
    pub by_monitor: HashMap<Arc<str>, tui::Elem>,
    pub fallback: tui::Elem,
}

#[derive(Debug, Clone)]
struct BarMenu {
    kind: tui::MenuKind,
    tui_tx: WatchTx<tui::Elem>,
    tui_rx: WatchRx<tui::Elem>,
}
// TODO: Auto clean unused
type BarMenus = HashMap<tui::InteractTag, HashMap<tui::InteractKind, BarMenu>>;

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerUpdate {
    BarMenu(BarMenuUpdate),
    BarTui(BarTuiState),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BarMenuUpdate {
    pub tag: tui::InteractTag,
    pub kind: tui::InteractKind,
    pub menu: Option<tui::OpenMenu>,
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ControllerEvent {
    Interact(TuiInteract),
    ReloadRequest,
}
#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TuiInteract {
    pub kind: tui::InteractKind,
    pub tag: tui::InteractTag,
}

pub async fn run_controller(
    update_rx: impl Stream<Item = ControllerUpdate> + Send + 'static,
    event_tx: UnbTx<ControllerEvent>,
) {
    let mut tasks = tokio::task::JoinSet::new();

    let (tui_tx, tui_rx) = watch_chan(BarTuiState {
        by_monitor: Default::default(),
        fallback: tui::Elem::empty(),
    });
    let (bar_menus_tx, bar_menus_rx) = watch_chan(BarMenus::default());

    tasks.spawn(async move {
        tokio::pin!(update_rx);
        while let Some(update) = update_rx.next().await {
            match update {
                ControllerUpdate::BarMenu(BarMenuUpdate { tag, kind, menu }) => {
                    bar_menus_tx.send_if_modified(|menus| {
                        if let Some(tui::OpenMenu { tui, menu_kind }) = menu {
                            use std::collections::hash_map::Entry;
                            match menus.entry(tag) {
                                Entry::Occupied(mut entry) => match entry.get_mut().entry(kind) {
                                    Entry::Occupied(mut cur) if cur.get().kind == menu_kind => {
                                        cur.get_mut().tui_tx.send_replace(tui);
                                        false
                                    }
                                    cur => {
                                        let (tui_tx, tui_rx) = watch_chan(tui);
                                        cur.insert_entry(BarMenu {
                                            kind: menu_kind,
                                            tui_tx,
                                            tui_rx,
                                        });
                                        true
                                    }
                                },
                                Entry::Vacant(entry) => {
                                    let (tui_tx, tui_rx) = watch_chan(tui);
                                    entry.insert_entry(HashMap::from_iter([(
                                        kind,
                                        BarMenu {
                                            kind: menu_kind,
                                            tui_tx,
                                            tui_rx,
                                        },
                                    )]));
                                    true
                                }
                            }
                        } else if let Some(tag_menus) = menus.get_mut(&tag)
                            && tag_menus.remove(&kind).is_some()
                        {
                            true
                        } else {
                            false
                        }
                    });
                }
                ControllerUpdate::BarTui(tui) => {
                    tui_tx.send_replace(tui);
                }
            }
        }
    });

    let reload_tx = ReloadTx::new();
    let mut reload_rx = reload_tx.subscribe();

    tasks.spawn(run_controller_inner(
        tui_rx,
        bar_menus_rx,
        reload_tx,
        event_tx.clone(),
    ));
    tokio::spawn(async move {
        while let Some(()) = reload_rx.wait().await
            && let Some(()) = event_tx.send(ControllerEvent::ReloadRequest).ok_or_debug()
        {}
    });

    if let Some(res) = tasks.join_next().await {
        res.ok_or_log();
    }
}

async fn run_controller_inner(
    tui_rx: WatchRx<BarTuiState>,
    bar_menus_rx: WatchRx<BarMenus>,
    mut reload_tx: ReloadTx,
    event_tx: UnbTx<ControllerEvent>,
) {
    let mut monitors_auto_cancel = HashMap::new();

    let mut monitor_rx = crate::monitors::connect();

    while let Some(ev) = monitor_rx.next().await {
        for monitor in ev.removed() {
            drop(monitors_auto_cancel.remove(monitor));
        }
        for monitor in ev.added_or_changed() {
            let cancel = CancellationToken::new();
            tokio::spawn(run_monitor(RunMonitorArgs {
                monitor: monitor.clone(),
                cancel_monitor: cancel.clone(),
                bar_rx: tui_rx.clone(),
                bar_menus_rx: bar_menus_rx.clone(),
                event_tx: event_tx.clone(),
            }));
            monitors_auto_cancel.insert(monitor.name.clone(), CancelDropGuard::from(cancel));
        }
        reload_tx.reload();
    }
}

#[derive(Clone)]
struct RunMonitorArgs {
    monitor: MonitorInfo,
    cancel_monitor: CancellationToken,
    bar_rx: WatchRx<BarTuiState>,
    bar_menus_rx: WatchRx<BarMenus>,
    event_tx: UnbTx<ControllerEvent>,
}

async fn run_monitor(args: RunMonitorArgs) {
    let monitor = args.monitor.name.clone();
    let _auto_cancel = CancelDropGuard::from(args.cancel_monitor.clone());

    run_or_retry(
        try_run_monitor,
        args,
        |it| it.with_context(|| format!("Failed to run panels for monitor {monitor}")),
        Duration::from_secs(10),
        None,
    )
    .await;
    log::debug!("Exiting panel manager for monitor {monitor:?}");
}

struct Term {
    term_ev_rx: UnbRx<TermEvent>,
    term_upd_tx: UnbTx<TermUpdate>,
    sizes: tui::Sizes,
    layout: Option<tui::RenderedLayout>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermKind {
    Menu,
    Bar,
}
enum Upd {
    BarTui,
    Term(TermKind, TermEvent),
}

struct StartedMonitorEnv {
    bar: Term,
    menu: Term,
    intern_upd_rx: UnbRx<Upd>,
    bar_tui_rx: WatchRx<tui::Elem>,
    event_tx: UnbTx<ControllerEvent>,
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

async fn run_monitor_main(
    monitor: MonitorInfo,
    mut env: StartedMonitorEnv,
    bar_menus_rx: WatchRx<BarMenus>,
) -> anyhow::Result<std::convert::Infallible> {
    #[derive(Debug)]
    struct ShowMenu {
        kind: MenuKind,
        pix_location: tui::Vec2<u32>,
        cached_size: tui::Vec2<u16>,
        sizing: tui::SizingArgs,
        tui: tui::Elem,
    }
    let mut show_menu = None::<ShowMenu>;
    let mut show_bar = Some(tui::Elem::empty());
    loop {
        let mut rerender_menu = false;
        let mut rerender_bar = false;

        let upd = tokio::select! {
            Some(ev) = env.bar.term_ev_rx.next() => Upd::Term(TermKind::Bar, ev),
            Some(ev) = env.menu.term_ev_rx.next() => Upd::Term(TermKind::Menu, ev),
            Some(upd) = env.intern_upd_rx.next() => upd,
            Ok(()) = env.bar_tui_rx.changed() => Upd::BarTui,
        };
        match upd {
            Upd::BarTui => {
                if let Some(bar) = &mut show_bar {
                    *bar = env.bar_tui_rx.borrow_and_update().clone();
                    rerender_bar = true;
                }
            }
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
                        pix_location,
                    } = layout.interpret_mouse_event(ev, env.bar.sizes.font_size());
                    let is_hover = kind == tui::InteractKind::Hover;

                    if term_kind == TermKind::Menu
                        && let Some(menu) = &show_menu
                        && menu.kind == MenuKind::Tooltip
                    {
                        show_menu = None;
                        rerender_menu = true;
                    }

                    if rerender {
                        match term_kind {
                            TermKind::Menu => rerender_menu = true,
                            TermKind::Bar => rerender_bar = true,
                        }
                    }

                    if changed || !is_hover {
                        if empty
                            && term_kind == TermKind::Bar
                            && let Some(menu) = &show_menu
                            && (!is_hover || menu.kind == MenuKind::Tooltip)
                        {
                            show_menu = None;
                            rerender_menu = true;
                        }

                        if let Some(tag) = tag.as_ref()
                            && let Some(BarMenu { kind, tui_rx, .. }) = bar_menus_rx
                                .borrow()
                                .get(tag)
                                .and_then(|tag_menus| tag_menus.get(&kind))
                                .cloned()
                        {
                            let sizing = tui::SizingArgs {
                                font_size: env.menu.sizes.font_size(),
                            };
                            // TODO: remember receiver (Also update on change of bar_menus_rx)
                            // Use a seperate task for the menu to do this
                            let tui = tui_rx.borrow().clone();
                            show_menu = Some(ShowMenu {
                                cached_size: tui::calc_min_size(&tui, &sizing),
                                sizing,
                                tui,
                                kind,
                                pix_location,
                            });
                            rerender_menu = true;
                        }

                        if let Some(tag) = tag {
                            env.event_tx
                                .send(ControllerEvent::Interact(TuiInteract { kind, tag }))
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
                rerender_bar = true;
            }
            Upd::Term(term_kind, TermEvent::FocusChange { is_focused }) => {
                // FIXME: This only works because the menu doesnt lose focus while we are
                // on the bar, which forbids focus.
                if !is_focused && term_kind == TermKind::Menu {
                    show_menu = None;
                    if let Some(layout) = &mut env.bar.layout
                        && layout.ext_focus_loss()
                    {
                        rerender_bar = true;
                    }
                }
            }
        }

        if rerender_menu {
            if show_menu.is_none()
                && let Some(layout) = &mut env.bar.layout
                && layout.ext_focus_loss()
            {
                rerender_bar = true;
            }

            if let Some(ShowMenu {
                pix_location: location,
                cached_size: cached_tui_size,
                ref tui,
                ref sizing,
                kind: _,
            }) = show_menu
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

            let action = if show_menu.is_some() { "show" } else { "hide" };
            env.menu
                .term_upd_tx
                .send(TermUpdate::RemoteControl(vec![
                    "resize-os-window".into(),
                    format!("--action={}", action).into(),
                ]))
                .ok_or_debug();
        }

        if rerender_bar && let Some(tui) = &show_bar {
            let mut buf = Vec::new();
            let Some(layout) = tui::render(
                tui,
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
    }
}

async fn init_term(
    sock_path: std::path::PathBuf,
    log_name: String,
    extra_args: impl IntoIterator<Item = OsString>,
    extra_envs: impl IntoIterator<Item = (OsString, OsString)>,
    cancel: &CancellationToken,
) -> anyhow::Result<Term> {
    let (term_upd_tx, term_upd_rx) = unb_chan();
    let (term_ev_tx, mut term_ev_rx) = unb_chan();

    inst::start_generic_panel(
        &sock_path,
        &log_name,
        term_upd_rx,
        extra_args,
        extra_envs,
        term_ev_tx,
        cancel.clone(),
    )
    .await?;

    let sizes = loop {
        match term_ev_rx.next().await {
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
    let mut bar_rx = args.bar_rx.clone();
    let monitor = args.monitor.clone();

    let (intern_upd_tx, intern_upd_rx) = unb_chan();

    let tmpdir = tokio::task::spawn_blocking(TempDir::new).await??;

    let bar_fut = init_term(
        tmpdir.path().join("bar-term-socket.sock"),
        format!("BAR@{}", monitor.name),
        [
            format!("--output-name={}", monitor.name).into(),
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

    let (bar_tui_tx, bar_tui_rx) = watch_chan(tui::Elem::empty());
    tokio::spawn(async move {
        while let Ok(()) = bar_rx.changed().await {
            let tui = {
                let lock = bar_rx.borrow_and_update();
                lock.by_monitor
                    .get(&monitor.name)
                    .unwrap_or(&lock.fallback)
                    .clone()
            };
            bar_tui_tx.send_replace(tui);
        }
    });

    Ok(StartedMonitorEnv {
        bar,
        menu,
        intern_upd_rx,
        bar_tui_rx,
        event_tx: args.event_tx.clone(),
    })
}

#[doc(hidden)]
pub fn __main() -> std::process::ExitCode {
    main_inner().unwrap_or(std::process::ExitCode::FAILURE)
}

const INTERNAL_SOCK_PATH_VAR: &str = "BAR_INTERNAL_SOCK_PATH";
fn main_inner() -> Option<std::process::ExitCode> {
    use std::process::ExitCode;

    if std::env::args_os().nth(1).as_deref() == Some(inst::INTERNAL_INST_ARG.as_ref()) {
        return Some(inst::inst_main());
    }

    use anyhow::Context as _;

    crate::logging::init_logger(crate::logging::ProcKind::Controller, "CONTROLLER".into());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to start the tokio runtime")
        .ok_or_log()?;

    let _guard = runtime.enter();

    // FIXME: Proper arg parsing
    let driver = std::env::args_os().nth(1)?;

    let (mut driver_child, conn) = {
        let socket_dir = tempfile::TempDir::new().ok_or_log()?;
        let sock_path = socket_dir.path().join("driver.sock");
        let socket = tokio::net::UnixListener::bind(&sock_path).ok_or_log()?;

        let child = tokio::process::Command::new(driver)
            .kill_on_drop(true)
            .args(std::env::args_os().skip(2))
            .env(INTERNAL_SOCK_PATH_VAR, sock_path)
            .spawn()
            .ok_or_log()?;

        let (conn, _) = runtime.block_on(socket.accept()).ok_or_log()?;

        (child, conn)
    };
    let (read, write) = conn.into_split();

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

    let (update_tx, update_rx) = unb_chan();
    let (event_tx, event_rx) = unb_chan();

    runtime.spawn(crate::utils::read_cobs(
        tokio::io::BufReader::new(read),
        move |ev| {
            update_tx.send(ev).ok_or_debug();
        },
    ));
    runtime.spawn(crate::utils::write_cobs(write, event_rx));

    let main_task = tokio_util::task::AbortOnDropHandle::new(runtime.spawn(async move {
        run_controller(update_rx, event_tx).await;
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

#[derive(Debug)]
pub struct ControllerEventStream {
    rx: UnbRx<ControllerEvent>,
}
impl Stream for ControllerEventStream {
    type Item = ControllerEvent;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_next_unpin(cx)
    }
}
#[derive(Debug, Clone)]
pub struct ControllerUpdateSender {
    tx: std::sync::mpsc::Sender<ControllerUpdate>,
}
impl ControllerUpdateSender {
    pub fn send(
        &mut self,
        update: ControllerUpdate,
    ) -> Result<(), std::sync::mpsc::SendError<ControllerUpdate>> {
        self.tx.send(update)
    }
}

pub struct WrappedErr(anyhow::Error);
const _: () = {
    use std::fmt;

    impl fmt::Debug for WrappedErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            fmt::Debug::fmt(&self.0, f)
        }
    }
    impl fmt::Display for WrappedErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }
    impl std::error::Error for WrappedErr {}
};

pub async fn run_driver_connection(
    tx: impl Fn(ControllerEvent) -> Option<()> + Send + 'static,
    rx: impl FnMut() -> Option<ControllerUpdate> + Send + 'static,
    on_stop: impl AsyncFnOnce(),
) -> Result<(), WrappedErr> {
    let sock_path = std::env::var_os(INTERNAL_SOCK_PATH_VAR)
        .context("Missing socket path env var")
        .map_err(WrappedErr)?;

    // HACK: This is potentially blocking, but it should be fine if we document it, since
    // this function is only meant to be run during startup? Alternatively, we can
    // spawn a thread for this.
    let socket = std::os::unix::net::UnixStream::connect(sock_path)
        .context("Failed to connect to controller socket")
        .map_err(WrappedErr)?;

    crate::utils::run_cobs_socket(socket, tx, rx, CancellationToken::new()).await;
    on_stop().await;

    Ok(())
}
