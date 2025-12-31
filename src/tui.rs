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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    pub tag: Option<InteractTag>,
    pub kind: ElementKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ElementKind {
    Subdivide(Subdiv),
    Raw(Arc<str>),
    Image(Image),
    Block(Block),
    Spacing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Image {
    pub data: Vec<u8>,
    pub format: image::ImageFormat,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub borders: Borders,
    pub border_style: Style,
    pub border_set: LineSet,
    pub inner: Option<Box<Element>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Borders {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Constraint {
    Length(u16),
    Fill(u16),
    FitImage,
    //Percentage(u16),
    //Min(u16),
    //Max(u16),
    //Ratio(u32, u32),
}
// TODO: Builder
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subdiv {
    pub axis: Axis,
    pub parts: Box<[SubPart]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubPart {
    pub constr: Constraint,
    pub elem: Element,
}
impl SubPart {
    pub fn spacing(constr: Constraint) -> Self {
        Self {
            constr,
            elem: Element {
                kind: ElementKind::Spacing,
                tag: None,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tui {
    pub root: Element,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
