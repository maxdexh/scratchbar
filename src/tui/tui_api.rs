use crate::tui::*;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InteractTag(Arc<[u8]>);

impl InteractTag {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
}
impl Axis {
    pub fn other(self) -> Self {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::X,
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum InteractKind {
    Click(MouseButton),
    Scroll(Direction),
    Hover,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct StackItem {
    pub elem: Elem,
    pub opts: StackItemOpts,
}
impl From<Elem> for StackItem {
    fn from(elem: Elem) -> Self {
        Self {
            elem,
            opts: Default::default(),
        }
    }
}
#[derive(Default, Debug, Clone)]
pub struct StackItemOpts {
    pub fill_weight: u16,
    // TODO: Spacing
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    pub __non_exhaustive_struct_update: (),
}
#[derive(Default, Debug, Clone)]
pub struct StackOpts {
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    // TODO: Spacing
    pub __non_exhaustive_struct_update: (),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ImageLayoutMode {
    FillAxis(Axis, u16),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockOpts {
    pub borders: BlockBorders,
    pub border_style: Option<TextStyle>,
    pub lines: BlockLineSet,
    pub inner: Option<Elem>,

    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    pub __non_exhaustive_struct_update: (),
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BlockBorders {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}
impl BlockBorders {
    pub fn all() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
        }
    }
}

// TODO: Decide on the internals of LineSet. Currently only single-width strings work
// It would be cool if we could make multi-character borders work, e.g. to use
// alternating +=+=+=+ borders. Ideally, ElemRepr would have a Fill variant that
// just fills whatever area it is rendered to with some symbols. Then we can represent
// blocks as stacks and fills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockLineSet {
    pub(crate) vertical: Arc<str>,
    pub(crate) horizontal: Arc<str>,
    pub(crate) top_right: Arc<str>,
    pub(crate) top_left: Arc<str>,
    pub(crate) bottom_right: Arc<str>,
    pub(crate) bottom_left: Arc<str>,
}
impl Default for BlockLineSet {
    fn default() -> Self {
        Self::normal()
    }
}
macro_rules! lazy_str {
    ($s:expr) => {{
        static VALUE: std::sync::OnceLock<std::sync::Arc<str>> = std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| std::sync::Arc::<str>::from($s))
            .clone()
    }};
}
impl BlockLineSet {
    pub fn normal() -> Self {
        Self {
            vertical: lazy_str!("│"),
            horizontal: lazy_str!("─"),
            top_right: lazy_str!("┐"),
            top_left: lazy_str!("┌"),
            bottom_right: lazy_str!("┘"),
            bottom_left: lazy_str!("└"),
        }
    }

    pub fn rounded() -> Self {
        Self {
            top_right: lazy_str!("╮"),
            top_left: lazy_str!("╭"),
            bottom_right: lazy_str!("╯"),
            bottom_left: lazy_str!("╰"),
            ..Self::normal()
        }
    }

    pub fn double() -> Self {
        Self {
            vertical: lazy_str!("║"),
            horizontal: lazy_str!("═"),
            top_right: lazy_str!("╗"),
            top_left: lazy_str!("╔"),
            bottom_right: lazy_str!("╝"),
            bottom_left: lazy_str!("╚"),
        }
    }

    pub fn thick() -> Self {
        Self {
            vertical: lazy_str!("┃"),
            horizontal: lazy_str!("━"),
            top_right: lazy_str!("┓"),
            top_left: lazy_str!("┏"),
            bottom_right: lazy_str!("┛"),
            bottom_left: lazy_str!("┗"),
        }
    }

    pub fn light_double_dashed() -> Self {
        Self {
            vertical: lazy_str!("╎"),
            horizontal: lazy_str!("╌"),
            ..Self::normal()
        }
    }

    pub fn heavy_double_dashed() -> Self {
        Self {
            vertical: lazy_str!("╏"),
            horizontal: lazy_str!("╍"),
            ..Self::thick()
        }
    }

    pub fn light_triple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┆"),
            horizontal: lazy_str!("┄"),
            ..Self::normal()
        }
    }

    pub fn heavy_triple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┇"),
            horizontal: lazy_str!("┅"),
            ..Self::thick()
        }
    }

    pub fn light_quadruple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┊"),
            horizontal: lazy_str!("┈"),
            ..Self::normal()
        }
    }

    pub fn heavy_quadruple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┋"),
            horizontal: lazy_str!("┉"),
            ..Self::thick()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elem(pub(crate) Arc<ElemRepr>);

#[derive(Debug)]
pub struct RgbaImage {
    pub buf: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub layout: ImageLayoutMode,
    pub opts: ImageOpts,
}

#[derive(Debug, Default)]
pub struct ImageOpts {
    // TODO: Alt elem
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    __non_exhaustive_struct_update: (),
}

impl Elem {
    pub fn with_min_size(self, min_size: Size) -> Self {
        ElemRepr::MinSize {
            width: min_size.width,
            height: min_size.height,
            elem: self,
        }
        .into()
    }

    pub fn empty() -> Self {
        ElemRepr::Print {
            raw: Default::default(),
            width: 0,
            height: 0,
        }
        .into()
    }

    pub fn spacing(axis: Axis, len: u16) -> Self {
        Elem::empty().with_min_size({
            let mut size = Vec2::default();
            size[axis] = len;
            size.into()
        })
    }

    pub fn rgba_image(image: RgbaImage) -> Self {
        let RgbaImage {
            buf,
            layout,
            width,
            height,
            opts,
        } = image;

        let ImageOpts {
            #[expect(deprecated)]
                __non_exhaustive_struct_update: (),
        } = opts;

        ElemRepr::Image(ImageRepr {
            buf,
            layout,
            dimensions: Vec2 {
                x: width,
                y: height,
            },
        })
        .into()
    }

    pub fn interactive(self, tag: InteractTag) -> Self {
        ElemRepr::Interact(InteractRepr {
            tag,
            normal: self,
            hovered: None,
        })
        .into()
    }

    pub fn interactive_hover(self, tag: InteractTag, hovered: Elem) -> Self {
        ElemRepr::Interact(InteractRepr {
            tag,
            normal: self,
            hovered: Some(hovered),
        })
        .into()
    }

    pub fn block(opts: BlockOpts) -> Self {
        let BlockOpts {
            borders,
            border_style,
            lines: border_set,
            inner,
            #[expect(deprecated)]
                __non_exhaustive_struct_update: (),
        } = opts;

        ElemRepr::Block(BlockRepr {
            borders,
            border_style: border_style.map(Into::into).unwrap_or_default(),
            border_set,
            inner,
        })
        .into()
    }

    pub fn raw_print(raw: impl fmt::Display, size: Size) -> Self {
        ElemRepr::Print {
            raw: raw.to_string(),
            width: size.width,
            height: size.height,
        }
        .into()
    }

    pub fn text(plain: impl fmt::Display, opts: impl Into<TextOpts>) -> Self {
        let mut writer = PlainTextWriter::with_opts(opts.into());
        fmt::write(&mut writer, format_args!("{plain}")).unwrap();
        writer.finish()
    }

    pub fn stack(
        axis: Axis,
        items: impl IntoIterator<Item: Into<StackItem>>,
        opts: impl Into<StackOpts>,
    ) -> Self {
        let StackOpts {
            #[expect(deprecated)]
                __non_exhaustive_struct_update: (),
        } = opts.into();

        let items = items
            .into_iter()
            .map(|item| {
                let StackItem {
                    elem,
                    opts:
                        StackItemOpts {
                            fill_weight,
                            #[expect(deprecated)]
                                __non_exhaustive_struct_update: (),
                        },
                } = item.into();

                StackItemRepr { fill_weight, elem }
            })
            .collect();

        ElemRepr::Stack(StackRepr { axis, items }).into()
    }
}

#[derive(Default, Debug, Clone)]
pub struct TextOpts {
    pub style: Option<TextStyle>,
    pub trim_trailing_line: bool,
    // TODO: Sizing support
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    pub __non_exhaustive_struct_update: (),
}
impl From<TextStyle> for TextOpts {
    fn from(value: TextStyle) -> Self {
        Self {
            style: Some(value),
            ..Default::default()
        }
    }
}
impl From<TextModifiers> for TextOpts {
    fn from(value: TextModifiers) -> Self {
        TextStyle::from(value).into()
    }
}

// FIXME: Remove serialize, convert to Print
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct TextStyle {
    pub fg: Option<TermColor>,
    pub bg: Option<TermColor>,
    pub modifiers: Option<TextModifiers>,
    pub underline_color: Option<TermColor>,

    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
impl From<TextModifiers> for TextStyle {
    fn from(modifier: TextModifiers) -> Self {
        Self {
            modifiers: Some(modifier),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TermColor {
    Reset,
    Black,
    DarkGrey,
    Red,
    DarkRed,
    Green,
    DarkGreen,
    Yellow,
    DarkYellow,
    Blue,
    DarkBlue,
    Magenta,
    DarkMagenta,
    Cyan,
    DarkCyan,
    White,
    Grey,
    Rgb { r: u8, g: u8, b: u8 },
    AnsiValue(u8),
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct TextModifiers {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub hidden: bool,
    pub strike: bool,

    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
