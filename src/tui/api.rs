use crate::tui::*;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

/// Custom ID specified by the user. Holds custom bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CustomId(Arc<[u8]>);

impl fmt::Debug for CustomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x")?;
        for byte in self.0.iter() {
            write!(f, "{byte:x}")?;
        }
        Ok(())
    }
}

impl CustomId {
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
    pub fn flip(self) -> Self {
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
#[non_exhaustive]
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

#[derive(Debug)]
pub struct RgbaImage {
    pub buf: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub layout: ImageLayoutMode,
    pub opts: ImageOpts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ImageLayoutMode {
    FillAxis(Axis, u16),
}

#[derive(Debug, Default)]
pub struct ImageOpts {
    // TODO: Alt elem
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    __non_exhaustive_struct_update: (),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elem(pub(crate) Arc<ElemRepr>);

impl Elem {
    pub fn with_min_size(self, min_size: Size) -> Self {
        ElemRepr::MinSize(MinSizeRepr {
            elem: self,
            size: min_size.into(),
        })
        .into()
    }

    pub fn empty() -> Self {
        ElemRepr::Print(PrintRepr {
            raw: Default::default(),
        })
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

    pub fn interactive(self, tag: CustomId) -> Self {
        ElemRepr::Interact(InteractRepr {
            tag,
            normal: self,
            hovered: None,
        })
        .into()
    }

    pub fn interactive_hover(self, tag: CustomId, hovered: Elem) -> Self {
        ElemRepr::Interact(InteractRepr {
            tag,
            normal: self,
            hovered: Some(hovered),
        })
        .into()
    }

    pub fn fill_cells_single(symbol: impl fmt::Display) -> Self {
        ElemRepr::Fill(FillRepr {
            symbol: symbol.to_string(),
        })
        .into()
    }

    pub fn raw_print(raw: impl fmt::Display, size: Size) -> Self {
        Elem::from(ElemRepr::Print(PrintRepr {
            raw: raw.to_string(),
        }))
        .with_min_size(size)
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
