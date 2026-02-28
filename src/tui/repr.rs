use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::tui::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ElemRepr {
    Print(PrintRepr),
    Stack(StackRepr),
    Interact(InteractRepr),
    Fill(FillRepr),
    MinSize(MinSizeRepr),
    MinAxis(MinAxisRepr),
}

impl From<ElemRepr> for Elem {
    fn from(value: ElemRepr) -> Self {
        Self(Arc::new(value))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MinSizeRepr {
    pub elem: Elem,
    pub size: Vec2<u16>,
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
pub(crate) struct FillRepr {
    pub symbol: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MinAxisRepr {
    pub elem: Elem,
    pub axis: Axis,
    pub len: u16,
    pub aspect: Vec2<u32>,
}

// TODO: Use a DST struct to hold the tail of these
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StackRepr {
    pub axis: Axis,
    pub items: Vec<StackItemRepr>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct PrintRepr {
    pub raw: Vec<u8>,
}
impl std::fmt::Debug for PrintRepr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.raw.utf8_chunks(), f)
    }
}
