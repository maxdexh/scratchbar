use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::{
    data::{InteractKind, Position32},
    tui,
};

// TODO: Partial updates?
#[derive(Serialize, Deserialize)]
pub enum PanelEvent {
    Interact(tui::TuiInteract),
}
#[derive(Serialize, Deserialize)]
pub enum PanelUpdate {
    Display(tui::Tui),
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

    let font_size = tui::Size::query_font_size()?;
    let mut ui = tui::RenderedLayout::default();

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
                match tui::draw(|ctx| {
                    let size = tui::Size::query_term_size()?;
                    tui.render(
                        ctx,
                        tui::SizingContext {
                            font_size,
                            div_w: Some(size.w),
                            div_h: Some(size.h),
                        },
                        tui::Area {
                            size,
                            pos: Default::default(),
                        },
                    )
                }) {
                    Err(err) => log::error!("Failed to draw: {err}"),
                    Ok(layout) => ui = layout,
                }
            }
            Upd::Term(event) => match event {
                crossterm::event::Event::FocusGained => continue,
                crossterm::event::Event::Key(_) => continue,
                crossterm::event::Event::FocusLost => {
                    if let Err(err) = ctrl_tx.send(PanelEvent::Interact(tui::TuiInteract {
                        location: Position32::ZERO,
                        target: None,
                        kind: InteractKind::Hover,
                    })) {
                        log::warn!("Failed to send interaction: {err}");
                        break;
                    }
                }
                crossterm::event::Event::Mouse(event) => {
                    let Some(interact) = ui.interpret_mouse_event(event, font_size) else {
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
