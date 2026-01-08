use crate::modules::prelude::MenuKind;

impl MenuKind {
    pub fn close_on_unfocus(self) -> bool {
        match self {
            Self::Tooltip => true,
            Self::Context => false,
        }
    }
}
