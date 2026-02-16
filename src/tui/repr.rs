use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::tui::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ElemRepr {
    Print {
        raw: String,
        width: u16,
        height: u16,
    },
    MinSize {
        elem: Elem,
        width: u16,
        height: u16,
    },
    Image(ImageRepr),
    Stack(StackRepr),
    Block(BlockRepr),
    Interact(InteractRepr),
}
impl From<ElemRepr> for Elem {
    fn from(value: ElemRepr) -> Self {
        Self(Arc::new(value))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlockRepr {
    pub borders: BlockBorders,
    pub border_style: StyleRepr,
    pub border_set: BlockLineSet,
    pub inner: Option<Elem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StackRepr {
    pub axis: Axis,
    pub items: Vec<StackItemRepr>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StackItemRepr {
    pub fill_weight: u16,
    pub elem: Elem,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InteractRepr {
    pub tag: CustomId,
    pub normal: Elem,
    pub hovered: Option<Elem>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct StyleRepr {
    pub begin: String,
    pub end: String,
}
impl From<TextStyle> for StyleRepr {
    fn from(value: TextStyle) -> Self {
        Self {
            begin: {
                let mut begin = String::new();
                value.begin(&mut begin).unwrap();
                begin
            },
            end: {
                let mut end = String::new();
                value.end(&mut end).unwrap();
                end
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ImageRepr {
    pub buf: Vec<u8>,
    pub dimensions: Vec2<u32>,
    pub layout: ImageLayoutMode,
}
