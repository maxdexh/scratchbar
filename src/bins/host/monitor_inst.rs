use tempfile::TempDir;

use std::{ffi::OsString, time::Duration};

use anyhow::Context;
use tokio::{
    sync::{mpsc::UnboundedSender, watch},
    task::JoinSet,
};
use tokio_util::{sync::CancellationToken, time::FutureExt as _};

use crate::{
    bins::{
        host::MonitorInfo,
        inst::{TermEvent, TermUpdate},
    },
    host, tui,
    utils::ResultExt,
};

#[derive(Clone)]
pub(super) struct RunMonitorArgs {
    pub monitor: MonitorInfo,
    pub cancel_monitor: CancellationToken,
    pub bar_state_tx: watch::Sender<super::BarTuiStateSender>,
    pub open_menu_rx: watch::Receiver<Option<host::OpenMenu>>,
    pub event_tx: std::sync::mpsc::Sender<host::HostEvent>,
}
pub(super) async fn run_monitor(mut args: RunMonitorArgs) {
    let monitor = args.monitor.name.clone();
    let _auto_cancel = args.cancel_monitor.clone().drop_guard();

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
    term_upd_tx: UnboundedSender<TermUpdate>,
    sizes: tui::Sizes,
    layout: tui::RenderedLayout,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermKind {
    Menu,
    Bar,
}
impl From<TermKind> for host::TermKind {
    fn from(value: TermKind) -> Self {
        match value {
            TermKind::Menu => Self::Menu,
            TermKind::Bar => Self::Bar,
        }
    }
}
enum Upd {
    Noop,
    Term(TermKind, TermEvent),
}

struct StartedMonitorEnv {
    bar: Term,
    menu: Term,
    bar_tui_rx: watch::Receiver<tui::Elem>,
    bar_hide_rx: watch::Receiver<bool>,
    event_tx: std::sync::mpsc::Sender<host::HostEvent>,
    open_menu_rx: watch::Receiver<Option<host::OpenMenu>>,
}

async fn try_run_monitor(args: &mut RunMonitorArgs) -> anyhow::Result<()> {
    log::debug!("Starting panel manager for monitor {:?}", args.monitor);

    let mut required_tasks = JoinSet::<anyhow::Result<std::convert::Infallible>>::new();
    let cancel = args.cancel_monitor.child_token();
    let _auto_cancel = cancel.clone().drop_guard();
    let env = try_init_monitor(args, &mut required_tasks, &cancel).await?;
    required_tasks.spawn(run_monitor_main(args.monitor.clone(), env));

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
const VERTICAL_PADDING: bool = false;
const HORIZONTAL_PADDING: u16 = 4;

#[derive(Debug)]
struct ShowMenu {
    pix_location: tui::Vec2<u32>,
    cached_size: tui::Vec2<u16>,
    sizing: tui::SizingArgs,
    tui: tui::Elem,
    bar_anchor: tui::CustomId,
}
impl ShowMenu {
    fn update(this: &mut Option<Self>, open: host::OpenMenu, env: &StartedMonitorEnv) {
        let host::OpenMenu {
            tui,
            monitor: _,
            bar_anchor,
            opts:
                host::OpenMenuOpts {
                    #[expect(deprecated)]
                        __non_exhaustive_struct_update: (),
                },
        } = open;

        let pix_location = if let Some(this) = this
            && this.bar_anchor == bar_anchor
        {
            this.pix_location
        } else {
            env.bar
                .layout
                .get_pix_location(env.bar.sizes.font_size(), &bar_anchor)
                .unwrap_or_default()
        };

        let sizing = tui::SizingArgs {
            font_size: env.menu.sizes.font_size(),
        };
        this.replace(ShowMenu {
            pix_location,
            cached_size: tui::calc_min_size(&tui, &sizing),
            sizing,
            tui,
            bar_anchor,
        });
    }
}
// FIXME: This function is way too large
async fn run_monitor_main(
    monitor: MonitorInfo,
    mut env: StartedMonitorEnv,
) -> anyhow::Result<std::convert::Infallible> {
    let mut show_menu = None::<ShowMenu>;
    let mut bar_tui_state = super::BarTuiState {
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
            Ok(()) = env.open_menu_rx.changed() => {
                let open = env.open_menu_rx.borrow_and_update().clone();
                if let Some(open) = open && open.monitor == monitor.name {
                    ShowMenu::update(&mut show_menu, open, &env);
                } else {
                    if show_menu.is_none() {
                        continue;
                    }
                    show_menu = None;
                }
                env.menu.layout = Default::default(); // TODO: optionally keep layout
                rerender_menu = true;
                Upd::Noop
            },
        };
        match upd {
            Upd::Noop => {}
            Upd::Term(term_kind, TermEvent::Crossterm(ev)) => match ev {
                crossterm::event::Event::Mouse(ev) => {
                    let term = match term_kind {
                        TermKind::Menu => &mut env.menu,
                        TermKind::Bar => &mut env.bar,
                    };

                    match term
                        .layout
                        .interpret_mouse_event(ev, term.sizes.font_size())
                    {
                        tui::MouseEventRes::Interact(tui::MouseInteractRes {
                            kind,
                            tag,
                            changed,
                            rerender,
                        }) => {
                            let is_hover = kind == tui::InteractKind::Hover;

                            if rerender {
                                match term_kind {
                                    TermKind::Menu => rerender_menu = true,
                                    TermKind::Bar => bar_tui_changed = true,
                                }
                            }

                            if changed || !is_hover {
                                env.event_tx
                                    .send(host::HostEvent::Term(
                                        host::TermInfo {
                                            monitor: monitor.name.clone(),
                                            kind: term_kind.into(),
                                        },
                                        host::TermEvent::Interact(host::InteractEvent {
                                            kind,
                                            tag,
                                        }),
                                    ))
                                    .ok_or_debug();
                            }
                        }
                        tui::MouseEventRes::MouseLeave => {
                            if term.layout.ext_focus_loss() {
                                match term_kind {
                                    TermKind::Menu => rerender_menu = true,
                                    TermKind::Bar => bar_tui_changed = true,
                                }
                            }

                            env.event_tx
                                .send(host::HostEvent::Term(
                                    host::TermInfo {
                                        monitor: monitor.name.clone(),
                                        kind: term_kind.into(),
                                    },
                                    host::TermEvent::MouseLeave,
                                ))
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
        }

        if rerender_menu {
            if let Some(&ShowMenu {
                pix_location: location,
                cached_size: cached_tui_size,
                ref tui,
                ref sizing,
                bar_anchor: _,
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
                    &env.menu.layout,
                )
                .context("Failed to draw menu")
                .ok_or_log()
                {
                    env.menu.layout = layout;
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
                &env.bar.layout,
            )
            .context("Failed to render bar")
            .ok_or_log() else {
                continue;
            };
            env.bar.layout = layout;

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

    crate::bins::inst::start_generic_panel(
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

const NERD_FONT_CONFIG_OVERRIDE: &str = "-o=symbol_map U+e000-U+e00a,U+ea60-U+ebeb,U+e0a0-U+e0c8,U+e0ca,U+e0cc-U+e0d7,U+e200-U+e2a9,U+e300-U+e3e3,U+e5fa-U+e6b7,U+e700-U+e8ef,U+ed00-U+efc1,U+f000-U+f2ff,U+f000-U+f2e0,U+f300-U+f381,U+f400-U+f533,U+f0001-U+f1af0 Symbols Nerd Font Mono";

async fn try_init_monitor(
    args: &RunMonitorArgs,
    required_tasks: &mut JoinSet<anyhow::Result<std::convert::Infallible>>,
    cancel: &CancellationToken,
) -> anyhow::Result<StartedMonitorEnv> {
    let monitor = args.monitor.clone();

    let tmpdir = tokio::task::spawn_blocking(TempDir::new).await??;

    let bar_fut = init_term(
        tmpdir.path().join("bar-term-socket.sock"),
        format!("BAR@{}", monitor.name),
        [
            NERD_FONT_CONFIG_OVERRIDE.into(),
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
        let menu = init_term(
            tmpdir.path().join("menu-term-socket.sock"),
            format!("MENU@{}", monitor.name),
            [
                NERD_FONT_CONFIG_OVERRIDE.into(),
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
                // Since we control resizes from the program and not from
                // a somewhat continuous drag-resize, debouncing between
                // resize and reloads is completely inappropriate and
                // just results in a larger delay between resize and
                // the old menu content being replaced with the new one.
                "-o=resize_debounce_time=0 0".into(),
                // TODO: Mess with repaint_delay, input_delay
            ],
            [],
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

        anyhow::Ok(menu)
    };

    let res = async { tokio::try_join!(bar_fut, menu_fut) }
        .timeout(Duration::from_secs(10))
        .await;

    // We have connected to the sockets, there is no need to keep the files around.
    tokio::task::spawn_blocking(move || drop(tmpdir));

    let (bar, menu) = res??;

    let (bar_tui_tx, bar_tui_rx) = watch::channel(tui::Elem::empty());
    let (bar_hide_tx, bar_hide_rx) = watch::channel(false);
    {
        let mut bar_state_tx_rx = args.bar_state_tx.subscribe();
        required_tasks.spawn(async move {
            'outer: loop {
                let mut tui_rx;
                let mut hide_rx;
                {
                    let super::BarTuiStateSender { tui, hidden } =
                        &*bar_state_tx_rx.borrow_and_update();

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
        bar_tui_rx,
        bar_hide_rx,
        event_tx: args.event_tx.clone(),
        open_menu_rx: args.open_menu_rx.clone(),
    })
}
