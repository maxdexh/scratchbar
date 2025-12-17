use std::{ffi::OsString, sync::Arc};

use ratatui::{Terminal, prelude::*, widgets::Paragraph};
use ratatui_image::FontSize;
use serde::{Deserialize, Serialize};
use system_tray::item::StatusNotifierItem;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::{
    clients::{
        pulse::{PulseDeviceKind, PulseDeviceState, PulseState},
        upower::{BatteryState, EnergyState},
    },
    data::{BasicDesktopState, InteractKind, Location, WorkspaceId},
    utils::rect_center,
};

pub async fn controller_spawn_panel(
    _: &std::path::Path,
    display: &str,
    envs: Vec<(OsString, OsString)>,
    _: &tokio::sync::mpsc::UnboundedSender<BarEvent>,
) -> anyhow::Result<tokio::process::Child> {
    let child = tokio::process::Command::new("kitten")
        .envs(envs)
        .stdout(std::io::stderr())
        .args([
            "panel",
            &format!("--output-name={display}"),
            // Allow logging to $KITTY_STDIO_FORWARDED
            "-o=forward_stdio=yes",
            // Do not use the system's kitty.conf
            "--config=NONE",
            // Basic look of the bar
            "-o=foreground=white",
            "-o=background=black",
            // location of the bar
            &format!("--edge={}", super::EDGE),
            // disable hiding the mouse
            "-o=mouse_hide_wait=0",
        ])
        .arg(&std::env::current_exe()?)
        .args(["internal", super::INTERNAL_BAR_PANEL_ARG])
        .kill_on_drop(true)
        .spawn()?;

    Ok(child)
}

type Interact = crate::data::InteractGeneric<BarInteractTarget>;

#[derive(Serialize, Deserialize, Debug)]
pub enum BarEvent {
    Interact(Interact),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BarUpdate {
    SysTray(Arc<[(Arc<str>, StatusNotifierItem)]>),
    Desktop(BasicDesktopState),
    Energy(EnergyState),
    Pulse(PulseState),
    Ppd(Arc<str>),
    Time(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BarInteractTarget {
    None,
    HyprWorkspace(WorkspaceId),
    Time,
    Energy,
    Ppd,
    Audio(PulseDeviceKind),
    Tray(Arc<str>),
}

#[derive(Debug, Default)]
pub struct RenderedLayout {
    widgets: Vec<(Rect, BarInteractTarget)>,
}
impl RenderedLayout {
    pub fn insert(&mut self, rect: Rect, widget: BarInteractTarget) {
        self.widgets.push((rect, widget));
    }

    // TODO: Delay until hover
    // TODO: Pre-filter and pre-chunk interactions here, especially scrolls
    pub fn interpret_mouse_event(
        &mut self,
        event: crossterm::event::MouseEvent,
        font_size: FontSize,
    ) -> Option<Interact> {
        use crossterm::event::*;

        let MouseEvent {
            kind,
            column,
            row,
            modifiers: _,
        } = event;
        let pos = Position { x: column, y: row };

        let (rect, widget) = self
            .widgets
            .iter()
            .find(|(r, _)| r.contains(pos))
            .map_or_else(
                || {
                    (
                        Rect {
                            x: pos.x,
                            y: pos.y,
                            ..Default::default()
                        },
                        &BarInteractTarget::None,
                    )
                },
                |(r, w)| (*r, w),
            );

        type DR = crate::data::Direction;
        type IK = crate::data::InteractKind;
        type MK = crossterm::event::MouseEventKind;
        let kind = match kind {
            MK::Down(button) => IK::Click(button),
            MK::Moved => IK::Hover,
            MK::ScrollDown => IK::Scroll(DR::Down),
            MK::ScrollUp => IK::Scroll(DR::Up),
            MK::ScrollLeft => IK::Scroll(DR::Left),
            MK::ScrollRight => IK::Scroll(DR::Right),
            MK::Up(_) | MK::Drag(_) => {
                return None;
            }
        };

        Some(Interact {
            location: rect_center(rect, font_size),
            target: widget.clone(),
            kind,
        })
    }
}

#[derive(Debug, Default, Clone)]
struct BarState {
    monitor: Arc<str>,
    systray: Arc<[(Arc<str>, StatusNotifierItem)]>,
    desktop: BasicDesktopState,
    ppd_profile: Arc<str>,
    energy: EnergyState,
    pulse: PulseState,
    time: String,
}

// FIXME: Debounce all rendering events
fn render(
    picker: &ratatui_image::picker::Picker,
    frame: &mut ratatui::Frame,
    state: &BarState,
) -> RenderedLayout {
    let square_icon_len = {
        let (font_w, font_h) = picker.font_size();
        font_h.div_ceil(font_w)
    };

    let mut ui = RenderedLayout::default();

    let [mut ui_area, _] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(frame.area());

    // Margin of one cell from both edges
    [_, ui_area, _] = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(ui_area);

    let active_ws = state
        .desktop
        .monitors
        .iter()
        .find(|mr| mr.name == state.monitor)
        .map(|mr| &mr.active_workspace);
    for ws in state.desktop.workspaces.iter() {
        let ws_area;
        [ws_area, _, ui_area] = Layout::horizontal([
            Constraint::Length(ws.name.chars().count() as _),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(ui_area);
        let mut pg = Paragraph::new(&ws.name as &str);
        if active_ws == Some(&ws.id) {
            pg = pg.green();
        }
        frame.render_widget(pg, ws_area);
        ui.insert(ws_area, BarInteractTarget::HyprWorkspace(ws.id.clone()));
    }

    const SPACING: u16 = 3;

    if !state.time.is_empty() {
        let time_area;
        [ui_area, _, time_area] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(SPACING),
            Constraint::Length(state.time.chars().count() as _),
        ])
        .areas(ui_area);

        frame.render_widget(Paragraph::new(&state.time as &str), time_area);
        ui.insert(time_area, BarInteractTarget::Time);
    }

    if state.energy.should_show {
        // TODO: Time estimate tooltip
        let percentage = state.energy.percentage.round() as i64;
        let sign = match state.energy.bstate {
            BatteryState::Discharging | BatteryState::PendingDischarge => '-',
            _ => '+',
        };
        let rate = format!("{sign}{:.1}W", state.energy.rate);
        let energy = format!("{percentage:>3}% {rate:<6}");

        let ppd_symbol = match &state.ppd_profile as &str {
            "balanced" => " ",
            "performance" => " ",
            "power-saver" => " ",
            _ => "",
        };

        let (ppd_area, energy_area);
        [ui_area, _, ppd_area, energy_area] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(SPACING),
            Constraint::Length(ppd_symbol.chars().count() as _),
            Constraint::Length(energy.chars().count() as _),
        ])
        .areas(ui_area);

        frame.render_widget(Paragraph::new(energy), energy_area);
        ui.insert(energy_area, BarInteractTarget::Energy);

        frame.render_widget(Paragraph::new(ppd_symbol), ppd_area);
        ui.insert(ppd_area, BarInteractTarget::Ppd);
    }

    {
        fn fmt_audio_device<const N: usize>(
            &PulseDeviceState { muted, volume, .. }: &PulseDeviceState,
            muted_symbol: &str,
            normal_symbols: [&str; N],
        ) -> String {
            format!(
                "{}{:>3}%",
                if muted {
                    muted_symbol
                } else {
                    normal_symbols[((N as f64 * volume) as usize).clamp(0, N - 1)]
                },
                (volume * 100.0).round() as u32
            )
        }
        let sink = fmt_audio_device(&state.pulse.sink, " ", [" "]); // " ", " ", 
        // FIXME: The muted symbol is double-width, the regular symbol is not
        let source = fmt_audio_device(&state.pulse.source, " ", [" "]);

        let (source_area, sink_area);
        [ui_area, _, source_area, _, sink_area] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(SPACING),
            Constraint::Length(source.chars().count() as _),
            Constraint::Length(SPACING),
            Constraint::Length(sink.chars().count() as _),
        ])
        .areas(ui_area);

        frame.render_widget(Paragraph::new(sink), sink_area);
        ui.insert(sink_area, BarInteractTarget::Audio(PulseDeviceKind::Sink));

        frame.render_widget(Paragraph::new(source), source_area);
        ui.insert(
            source_area,
            BarInteractTarget::Audio(PulseDeviceKind::Source),
        );
    }

    for (addr, item) in state.systray.iter() {
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

            let icon_area;
            [ui_area, _, icon_area] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Length(square_icon_len),
            ])
            .areas(ui_area);

            // https://users.rust-lang.org/t/argb32-color-model/92061/4
            for image::Rgba(pixel) in img.pixels_mut() {
                *pixel = u32::from_be_bytes(*pixel).rotate_left(8).to_be_bytes();
            }
            let img = image::DynamicImage::ImageRgba8(img);
            if let Ok(img) = picker
                .new_protocol(img, icon_area, ratatui_image::Resize::Fit(None))
                .map_err(|err| log::error!("Failed to create image: {err}"))
            {
                frame.render_widget(ratatui_image::Image::new(&img), icon_area);
            }
            ui.insert(icon_area, BarInteractTarget::Tray(addr.clone()));
        }
    }

    ui
}

// TODO: Spawn as needed for monitors
pub async fn main(
    ctrl_tx: mpsc::UnboundedSender<BarEvent>,
    mut ctrl_rx: mpsc::UnboundedReceiver<BarUpdate>,
    monitor: Arc<str>,
) -> anyhow::Result<()> {
    log::info!("Starting bar");

    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::cursor::Hide,
        crossterm::event::EnableMouseCapture,
    )?;
    crossterm::terminal::enable_raw_mode()?;

    let picker = ratatui_image::picker::Picker::from_query_stdio()?;
    let mut state = BarState {
        monitor,
        ..Default::default()
    };
    let mut ui = RenderedLayout::default();

    let mut term = Terminal::new(CrosstermBackend::new(std::io::stdout().lock()))?;

    // HACK: There is a bug that causes double width characters to be
    // small when rendered by ratatui on kitty, seemingly because
    // the spaces around them are not drawn at the beginning
    // (since the unfilled cell is seen as a space?). The workaround
    // is to fill the buffer with some non-space character.
    if let Err(err) = term.draw(|frame| {
        let area @ Rect { height, width, .. } = frame.area();
        frame.render_widget(
            Paragraph::new(
                std::iter::repeat_n(
                    std::iter::repeat_n('\u{2800}', width as _).chain(Some('\n')),
                    height as _,
                )
                .flatten()
                .collect::<String>(),
            ),
            area,
        );
    }) {
        log::error!("Failed to prefill terminal: {err}")
    }

    enum Upd {
        Ctrl(BarUpdate),
        Term(crossterm::event::Event),
    }
    let ctrl_stream = futures::stream::poll_fn(move |cx| ctrl_rx.poll_recv(cx)).map(Upd::Ctrl);
    let term_stream = crossterm::event::EventStream::new()
        .filter_map(|res| {
            res.map_err(|err| log::error!("Crossterm stream yielded: {err}"))
                .ok()
        })
        .map(Upd::Term);
    let mut events = term_stream.merge(ctrl_stream);

    while let Some(bar_event) = events.next().await {
        match bar_event {
            Upd::Ctrl(update) => match update {
                BarUpdate::SysTray(systray) => state.systray = systray,
                BarUpdate::Desktop(hypr) => state.desktop = hypr,
                BarUpdate::Energy(energy) => state.energy = energy,
                BarUpdate::Ppd(profile) => state.ppd_profile = profile,
                BarUpdate::Pulse(pulse) => state.pulse = pulse,
                BarUpdate::Time(time) => state.time = time,
            },
            Upd::Term(event) => match event {
                crossterm::event::Event::Paste(_) => continue,
                crossterm::event::Event::FocusGained => continue,
                crossterm::event::Event::Key(_) => continue,
                crossterm::event::Event::FocusLost => {
                    if let Err(err) = ctrl_tx.send(BarEvent::Interact(Interact {
                        location: Location::ZERO,
                        target: BarInteractTarget::None,
                        kind: InteractKind::Hover,
                    })) {
                        log::warn!("Failed to send interaction: {err}");
                        break;
                    }
                }
                crossterm::event::Event::Mouse(event) => {
                    let Some(interact) = ui.interpret_mouse_event(event, picker.font_size()) else {
                        continue;
                    };

                    if let Err(err) = ctrl_tx.send(BarEvent::Interact(interact)) {
                        log::warn!("Failed to send interaction: {err}")
                    }

                    continue;
                }
                crossterm::event::Event::Resize(_, _) => (),
            },
        }

        if let Err(err) = term.draw(|frame| ui = render(&picker, frame, &state)) {
            log::error!("Failed to draw: {err}");
        }
    }

    unreachable!()
}
