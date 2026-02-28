use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::tui::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ElemRepr {
    Print(PrintRepr),
    MinSize(MinSizeRepr),
    Image(ImageRepr),
    Stack(StackRepr),
    Interact(InteractRepr),
    Fill(FillRepr),
}

impl From<ElemRepr> for Elem {
    fn from(value: ElemRepr) -> Self {
        Self(Arc::new(value))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PrintRepr {
    pub raw: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MinSizeRepr {
    pub elem: Elem,
    pub size: Vec2<u16>,
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

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ImageRepr {
    pub buf: Vec<u8>,
    pub dimensions: Vec2<u32>,
    pub layout: ImageLayoutMode,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct FillRepr {
    pub symbol: String,
}
