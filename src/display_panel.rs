use std::sync::Arc;

use ratatui::{Terminal, prelude::*, widgets::Paragraph};
use ratatui_image::{FontSize, picker::Picker};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::{
    data::{InteractGeneric, InteractKind, Location},
    tui,
    utils::rect_center,
};
pub type PanelInteract = InteractGeneric<Option<tui::InteractTag>>;

// TODO: Partial updates?
#[derive(Serialize, Deserialize)]
pub enum PanelEvent {
    Interact(PanelInteract),
}
#[derive(Serialize, Deserialize)]
pub enum PanelUpdate {
    Display(tui::Tui),
}
// FIXME: Make private
#[derive(Debug, Default)]
pub struct RenderedLayout {
    pub widgets: Vec<(Rect, tui::InteractTag)>,
}
impl RenderedLayout {
    pub fn insert(&mut self, rect: Rect, widget: tui::InteractTag) {
        self.widgets.push((rect, widget));
    }

    // TODO: Delay until hover
    pub fn interpret_mouse_event(
        &mut self,
        event: crossterm::event::MouseEvent,
        font_size: FontSize,
    ) -> Option<PanelInteract> {
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
                        None,
                    )
                },
                |(r, w)| (*r, Some(w)),
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

        Some(PanelInteract {
            location: rect_center(rect, font_size),
            target: widget.cloned(),
            kind,
        })
    }
}

// TODO: Spawn as needed for monitors
pub async fn main(
    ctrl_tx: mpsc::UnboundedSender<PanelEvent>,
    mut ctrl_rx: mpsc::UnboundedReceiver<PanelUpdate>,
    monitor: Arc<str>, // TODO: Use for monitor-dependent elements
) -> anyhow::Result<()> {
    log::info!("Starting display panel");

    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::cursor::Hide,
        crossterm::event::EnableMouseCapture,
    )?;
    crossterm::terminal::enable_raw_mode()?;

    let picker = Picker::from_query_stdio()?;
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
        Ctrl(PanelUpdate),
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
            Upd::Ctrl(PanelUpdate::Display(mut tui)) => {
                if let Err(err) = term.draw(|frame| {
                    let area = frame.area();
                    let mut ctx = tui::RatatuiRenderContext {
                        picker: &picker,
                        frame,
                        layout: Default::default(),
                    };
                    tui.render_ratatui(
                        &mut ctx,
                        tui::SizingContext {
                            font_size: tui::Size::ratatui_picker_font_size(&picker),
                            div_w: Some(area.width),
                            div_h: Some(area.height),
                        },
                    );
                    ui = ctx.layout;
                }) {
                    log::error!("Failed to draw: {err}");
                }
            }
            Upd::Term(event) => match event {
                crossterm::event::Event::Paste(_) => continue,
                crossterm::event::Event::FocusGained => continue,
                crossterm::event::Event::Key(_) => continue,
                crossterm::event::Event::FocusLost => {
                    if let Err(err) = ctrl_tx.send(PanelEvent::Interact(PanelInteract {
                        location: Location::ZERO,
                        target: None,
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

                    if let Err(err) = ctrl_tx.send(PanelEvent::Interact(interact)) {
                        log::warn!("Failed to send interaction: {err}")
                    }

                    continue;
                }
                crossterm::event::Event::Resize(_, _) => (),
            },
        }
    }

    unreachable!()
}
