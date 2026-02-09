mod render;
pub(crate) use render::*;
mod layout;
pub(crate) use layout::*;
mod text;
pub use text::*;

mod repr;
use repr::*;

use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elem(Arc<ElemRepr>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InteractTag(Arc<[u8]>);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vec2<T> {
    pub x: T,
    pub y: T,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
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

impl<T> std::ops::Index<Axis> for Vec2<T> {
    type Output = T;

    fn index(&self, index: Axis) -> &Self::Output {
        let Self { x, y } = self;
        match index {
            Axis::X => x,
            Axis::Y => y,
        }
    }
}
impl<T> std::ops::IndexMut<Axis> for Vec2<T> {
    fn index_mut(&mut self, index: Axis) -> &mut Self::Output {
        let Self { x, y } = self;
        match index {
            Axis::X => x,
            Axis::Y => y,
        }
    }
}

impl InteractTag {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

impl Elem {
    pub fn with_min_size(self, min_size: Vec2<u16>) -> Self {
        ElemRepr::MinSize {
            size: min_size,
            elem: self,
        }
        .into()
    }

    pub fn empty() -> Self {
        ElemRepr::Print {
            raw: Default::default(),
            size: Default::default(),
        }
        .into()
    }

    pub fn spacing(axis: Axis, len: u16) -> Self {
        Elem::empty().with_min_size({
            let mut size = Vec2::default();
            size[axis] = len;
            size
        })
    }

    pub fn image(img: image::RgbaImage, sizing: ImageSizeMode) -> Self {
        ElemRepr::Image(ImageRepr {
            img: RgbaImageWrap(img),
            sizing,
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

    pub fn raw_print(raw: impl fmt::Display, size: Vec2<u16>) -> Self {
        ElemRepr::Print {
            raw: raw.to_string(),
            size,
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ImageSizeMode {
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

// TODO: Decide on the internals of LineSet.
// Currently only single-width strings work
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockLineSet {
    vertical: Arc<str>,
    horizontal: Arc<str>,
    top_right: Arc<str>,
    top_left: Arc<str>,
    bottom_right: Arc<str>,
    bottom_left: Arc<str>,
}

macro_rules! lazy_str {
    ($s:expr) => {{
        static VALUE: std::sync::OnceLock<std::sync::Arc<str>> = std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| std::sync::Arc::<str>::from($s))
            .clone()
    }};
}

impl Default for BlockLineSet {
    fn default() -> Self {
        Self::normal()
    }
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
