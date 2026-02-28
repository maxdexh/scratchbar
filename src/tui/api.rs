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

#[derive(Debug, Clone, Copy)]
pub struct MinAxis {
    pub axis: Axis,
    pub len: u16,
    pub aspect_width: u32,
    pub aspect_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elem(pub(crate) Arc<ElemRepr>);

impl Elem {
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

    pub fn raw_print(raw: impl fmt::Display) -> Self {
        ElemRepr::Print(PrintRepr {
            raw: raw.to_string().into(),
        })
        .into()
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

    pub fn with_min_size(self, min_size: Size) -> Self {
        ElemRepr::MinSize(MinSizeRepr {
            elem: self,
            size: min_size.into(),
        })
        .into()
    }
    pub fn with_min_axis(self, min_axis: MinAxis) -> Self {
        let MinAxis {
            axis,
            len,
            aspect_width,
            aspect_height,
        } = min_axis;

        ElemRepr::MinAxis(MinAxisRepr {
            elem: self,
            axis,
            len,
            aspect: Vec2 {
                x: aspect_width,
                y: aspect_height,
            },
        })
        .into()
    }
}
