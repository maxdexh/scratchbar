#[cfg(feature = "__bin")]
mod render;
#[cfg(feature = "__bin")]
pub(crate) use render::*;

#[cfg(feature = "__bin")]
mod layout;
#[cfg(feature = "__bin")]
pub(crate) use layout::*;

mod text_impl;
use text_impl::*;

mod tui_api;
pub use tui_api::*;

mod repr;
use repr::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Vec2<T> {
    pub x: T,
    pub y: T,
}
impl From<Size> for Vec2<u16> {
    fn from(value: Size) -> Self {
        let Size { width, height } = value;
        Self {
            x: width,
            y: height,
        }
    }
}
impl From<Vec2<u16>> for Size {
    fn from(value: Vec2<u16>) -> Self {
        let Vec2 { x, y } = value;
        Self {
            width: x,
            height: y,
        }
    }
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
