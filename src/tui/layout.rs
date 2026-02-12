use serde::{Deserialize, Serialize};

use crate::tui::*;

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub(crate) struct Area {
    pub pos: Vec2<u16>,
    pub size: Vec2<u16>,
}
impl Area {
    pub(crate) fn y_bottom(&self) -> u16 {
        self.pos.y.saturating_add(self.size.y).saturating_sub(1)
    }
    pub(crate) fn x_right(&self) -> u16 {
        self.pos.x.saturating_add(self.size.x).saturating_sub(1)
    }
    pub(crate) fn contains(self, pos: Vec2<u16>) -> bool {
        pos.x
            .checked_sub(self.pos.x)
            .is_some_and(|it| it < self.size.x)
            && pos
                .y
                .checked_sub(self.pos.y)
                .is_some_and(|it| it < self.size.y)
    }
}

impl<T> Vec2<T> {
    pub(crate) fn combine<U, R>(self, other: Vec2<U>, mut f: impl FnMut(T, U) -> R) -> Vec2<R> {
        Vec2 {
            x: f(self.x, other.x),
            y: f(self.y, other.y),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct StoredInteractive {
    tag: InteractTag,
    has_hover: bool,
}
impl StoredInteractive {
    pub(crate) fn new(elem: &InteractRepr) -> Self {
        Self {
            has_hover: elem.hovered.is_some(),
            tag: elem.tag.clone(),
        }
    }
}
#[derive(Debug)]
pub(crate) struct RenderedLayout {
    pub(super) widgets: Vec<(Area, StoredInteractive)>,
    pub(super) last_mouse_pos: Option<Vec2<u16>>,
    pub(super) last_hover_elem: Option<StoredInteractive>,
}

pub(crate) struct MouseEventResult {
    pub kind: InteractKind,
    pub tag: Option<InteractTag>,
    pub empty: bool,
    pub changed: bool,
    pub rerender: bool,
    pub has_hover: bool,
    pub pix_location: Vec2<u32>,
}

impl RenderedLayout {
    pub(super) fn insert(&mut self, area: Area, elem: &InteractRepr) {
        self.widgets.push((area, StoredInteractive::new(elem)));
    }

    pub(crate) fn ext_focus_loss(&mut self) -> bool {
        let changed = self.last_hover_elem.as_ref().is_some_and(|it| it.has_hover);
        self.last_mouse_pos = None;
        self.last_hover_elem = None;
        changed
    }

    pub(crate) fn interpret_mouse_event(
        &mut self,
        event: crossterm::event::MouseEvent,
        font_size: Vec2<u16>,
    ) -> MouseEventResult {
        let crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: _,
        } = event;

        let pos = Vec2 {
            x: column / font_size.x,
            y: row / font_size.y,
        };

        self.last_mouse_pos = Some(pos);

        type DR = Direction;
        type IK = InteractKind;
        type MK = crossterm::event::MouseEventKind;
        type MB = crossterm::event::MouseButton;

        let kind = match kind {
            MK::Down(MB::Left) => IK::Click(MouseButton::Left),
            MK::Down(MB::Right) => IK::Click(MouseButton::Right),
            MK::Down(MB::Middle) => IK::Click(MouseButton::Middle),
            MK::ScrollDown => IK::Scroll(DR::Down),
            MK::ScrollUp => IK::Scroll(DR::Up),
            MK::ScrollLeft => IK::Scroll(DR::Left),
            MK::ScrollRight => IK::Scroll(DR::Right),
            MK::Moved | MK::Up(_) | MK::Drag(_) => IK::Hover,
            MK::KittyLeaveWindow => unimplemented!(),
        };

        let font_w = u32::from(font_size.x);
        let font_h = u32::from(font_size.y);

        let Some((area, elem)) = self.widgets.iter().find(|(r, _)| r.contains(pos)) else {
            let cur = self.last_hover_elem.take();
            return MouseEventResult {
                kind,
                empty: true,
                tag: None,
                has_hover: false,
                pix_location: Vec2 {
                    x: u32::from(pos.x) * font_w,
                    y: u32::from(pos.y) * font_h,
                },
                changed: cur.is_some(),
                rerender: cur.is_some_and(|it| it.has_hover),
            };
        };

        let pix_location = {
            Vec2 {
                x: u32::from(area.pos.x) * font_w + u32::from(area.size.x) * font_w / 2,
                y: u32::from(area.pos.y) * font_h + u32::from(area.size.y) * font_h / 2,
            }
        };

        let prev = self.last_hover_elem.replace(elem.clone());

        let changed = prev.as_ref().is_none_or(|it| it.tag != elem.tag);

        let rerender = changed && (prev.as_ref().is_some_and(|it| it.has_hover) || elem.has_hover);

        MouseEventResult {
            kind,
            tag: Some(elem.tag.clone()),
            empty: false,
            changed,
            rerender,
            has_hover: elem.has_hover,
            pix_location,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub cell_size: Vec2<u16>,
    pub pix_size: Vec2<u16>,
}
impl Sizes {
    pub(crate) fn font_size(self) -> Vec2<u16> {
        let Self {
            cell_size: Vec2 { x: w, y: h },
            pix_size: Vec2 { x: pw, y: ph },
        } = self;
        Vec2 {
            x: pw / w,
            y: ph / h,
        }
    }
    pub(crate) fn query() -> anyhow::Result<Option<Self>> {
        let crossterm::terminal::WindowSize {
            rows,
            columns,
            width,
            height,
        } = crossterm::terminal::window_size()?;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            cell_size: Vec2 {
                x: columns,
                y: rows,
            },
            pix_size: Vec2 {
                x: width,
                y: height,
            },
        }))
    }
}
