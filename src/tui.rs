use std::sync::Arc;

// TODO: aarc?
// TODO: ElementKind that maps monitor to element

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct InteractTag(Arc<[u8]>);
impl InteractTag {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub enum Elem {
    Subdivide(Subdiv),
    Text(Text),
    Image(Image),
    Block(Block),
    Tagged(Box<TagElem>),
    #[default]
    Empty,
}
impl From<Subdiv> for Elem {
    fn from(value: Subdiv) -> Self {
        Self::Subdivide(value)
    }
}
impl From<Image> for Elem {
    fn from(value: Image) -> Self {
        Self::Image(value)
    }
}
impl From<Block> for Elem {
    fn from(value: Block) -> Self {
        Self::Block(value)
    }
}
impl From<TagElem> for Elem {
    fn from(value: TagElem) -> Self {
        Self::Tagged(Box::new(value))
    }
}
impl From<Text> for Elem {
    fn from(value: Text) -> Self {
        Self::Text(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Text {
    pub body: Arc<str>,
}
impl Text {
    pub fn plain(body: Arc<str>) -> Self {
        if body.contains('\x1b') {
            log::warn!("Call to `plain` with text containing <ESC>: {body:?}");
        }
        Self { body }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TagElem {
    pub tag: InteractTag,
    pub elem: Elem,
}
impl TagElem {
    pub fn new(tag: InteractTag, elem: impl Into<Elem>) -> Self {
        Self {
            tag,
            elem: elem.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Image {
    pub data: Vec<u8>,
    pub format: image::ImageFormat,
    #[serde(skip)]
    pub cached: Option<image::DynamicImage>,
}
impl Image {
    pub fn load(&mut self) -> anyhow::Result<&image::DynamicImage> {
        if self.cached.is_some() {
            // HACK: Borrow checker limitation
            return Ok(self.cached.as_ref().unwrap());
        }
        let img = image::load_from_memory(&self.data)?;
        Ok(self.cached.insert(img))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub borders: Borders,
    pub border_style: Style,
    pub border_set: LineSet,
    pub inner: Option<Box<Elem>>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Borders {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}
impl Borders {
    pub fn all() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Constr {
    Length(u16),
    Fill(u16),
    #[default]
    Auto,
}
// TODO: Builder
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subdiv {
    pub axis: Axis,
    pub parts: Box<[DivPart]>,
}
impl Subdiv {
    pub fn horizontal(parts: impl IntoIterator<Item = DivPart>) -> Self {
        Self {
            axis: Axis::Horizontal,
            parts: FromIterator::from_iter(parts),
        }
    }
    pub fn vertical(parts: impl IntoIterator<Item = DivPart>) -> Self {
        Self {
            axis: Axis::Vertical,
            parts: FromIterator::from_iter(parts),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DivPart {
    pub constr: Constr,
    pub elem: Elem,
}
impl DivPart {
    pub fn spacing(len: u16) -> Self {
        Self::length(len, Elem::Empty)
    }
    pub fn auto(elem: impl Into<Elem>) -> Self {
        Self::new(Constr::Auto, elem)
    }
    pub fn length(len: u16, elem: impl Into<Elem>) -> Self {
        Self::new(Constr::Length(len), elem)
    }
    pub fn new(constr: Constr, elem: impl Into<Elem>) -> Self {
        Self {
            constr,
            elem: elem.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}
impl Size {
    pub fn get_mut(&mut self, axis: Axis) -> &mut u16 {
        let Self { w, h } = self;
        match axis {
            Axis::Horizontal => w,
            Axis::Vertical => h,
        }
    }
    pub fn get(mut self, axis: Axis) -> u16 {
        *self.get_mut(axis)
    }
    pub fn ratatui_picker_font_size(picker: &ratatui_image::picker::Picker) -> Self {
        let (w, h) = picker.font_size();
        Self { w, h }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tui {
    pub root: Elem,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub modifier: Modifier,
    pub underline_color: Option<Color>,
}
#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Modifier {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub hidden: bool,
    pub strike: bool,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct LineSet {
    pub vertical: Box<str>,
    pub horizontal: Box<str>,
    pub top_right: Box<str>,
    pub top_left: Box<str>,
    pub bottom_right: Box<str>,
    pub bottom_left: Box<str>,
    pub vertical_left: Box<str>,
    pub vertical_right: Box<str>,
    pub horizontal_down: Box<str>,
    pub horizontal_up: Box<str>,
    pub cross: Box<str>,
}

impl LineSet {
    pub fn normal() -> Self {
        Self {
            vertical: "│".into(),
            horizontal: "─".into(),
            top_right: "┐".into(),
            top_left: "┌".into(),
            bottom_right: "┘".into(),
            bottom_left: "└".into(),
            vertical_left: "┤".into(),
            vertical_right: "├".into(),
            horizontal_down: "┬".into(),
            horizontal_up: "┴".into(),
            cross: "┼".into(),
        }
    }

    #[expect(dead_code)]
    pub fn rounded() -> Self {
        Self {
            top_right: "╮".into(),
            top_left: "╭".into(),
            bottom_right: "╯".into(),
            bottom_left: "╰".into(),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn double() -> Self {
        Self {
            vertical: "║".into(),
            horizontal: "═".into(),
            top_right: "╗".into(),
            top_left: "╔".into(),
            bottom_right: "╝".into(),
            bottom_left: "╚".into(),
            vertical_left: "╣".into(),
            vertical_right: "╠".into(),
            horizontal_down: "╦".into(),
            horizontal_up: "╩".into(),
            cross: "╬".into(),
        }
    }

    pub fn thick() -> Self {
        Self {
            vertical: "┃".into(),
            horizontal: "━".into(),
            top_right: "┓".into(),
            top_left: "┏".into(),
            bottom_right: "┛".into(),
            bottom_left: "┗".into(),
            vertical_left: "┫".into(),
            vertical_right: "┣".into(),
            horizontal_down: "┳".into(),
            horizontal_up: "┻".into(),
            cross: "╋".into(),
        }
    }

    #[expect(dead_code)]
    pub fn light_double_dashed() -> Self {
        Self {
            vertical: "╎".into(),
            horizontal: "╌".into(),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_double_dashed() -> Self {
        Self {
            vertical: "╏".into(),
            horizontal: "╍".into(),
            ..Self::thick()
        }
    }

    #[expect(dead_code)]
    pub fn light_triple_dashed() -> Self {
        Self {
            vertical: "┆".into(),
            horizontal: "┄".into(),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_triple_dashed() -> Self {
        Self {
            vertical: "┇".into(),
            horizontal: "┅".into(),
            ..Self::thick()
        }
    }

    #[expect(dead_code)]
    pub fn light_quadruple_dashed() -> Self {
        Self {
            vertical: "┊".into(),
            horizontal: "┈".into(),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_quadruple_dashed() -> Self {
        Self {
            vertical: "┋".into(),
            horizontal: "┉".into(),
            ..Self::thick()
        }
    }
}
mod render;
pub use render::*;
