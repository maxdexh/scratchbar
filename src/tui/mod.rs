mod render;
pub use render::*;
mod layout;
pub use layout::*;
mod text;
pub use text::*;

use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

#[derive(Debug, Serialize, Deserialize)]
enum ElemKind {
    Print { raw: String, size: Vec2<u16> },
    Image(Image),
    Stack(Stack),
    Block(Block),
    MinSize { size: Vec2<u16>, elem: Elem },
    Interact(InteractElem),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractElem {
    tag: InteractTag,
    normal: Elem,
    hovered: Option<Elem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elem(Arc<ElemKind>);
impl Default for Elem {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InteractTag(Arc<[u8]>);

impl InteractTag {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

impl From<ElemKind> for Elem {
    fn from(value: ElemKind) -> Self {
        Self(Arc::new(value))
    }
}

impl Elem {
    pub fn with_min_size(self, min_size: Vec2<u16>) -> Self {
        ElemKind::MinSize {
            size: min_size,
            elem: self,
        }
        .into()
    }

    pub fn empty() -> Self {
        ElemKind::Print {
            raw: Default::default(),
            size: Default::default(),
        }
        .into()
    }
    pub fn image(img: image::RgbaImage, sizing: ImageSizeMode) -> Self {
        ElemKind::Image(Image {
            img: RgbaImageWrap(img),
            sizing,
        })
        .into()
    }

    pub fn interactive(self, tag: InteractTag) -> Self {
        ElemKind::Interact(InteractElem {
            tag,
            normal: self,
            hovered: None,
        })
        .into()
    }

    pub fn interactive_hover(self, tag: InteractTag, hovered: Elem) -> Self {
        ElemKind::Interact(InteractElem {
            tag,
            normal: self,
            hovered: Some(hovered),
        })
        .into()
    }

    pub fn build_block(init: impl FnOnce(&mut Block)) -> Self {
        let mut builder = Block {
            borders: Default::default(),
            border_style: Default::default(),
            border_set: LineSet::normal(),
            inner: None,
        };
        init(&mut builder);
        ElemKind::Block(builder).into()
    }
    #[deprecated]
    pub fn build_stack(axis: Axis, init: impl FnOnce(&mut StackBuilder)) -> Self {
        let mut builder = StackBuilder::new(axis);
        init(&mut builder);
        builder.build()
    }
    pub fn raw_print(raw: impl fmt::Display, size: Vec2<u16>) -> Self {
        ElemKind::Print {
            raw: raw.to_string(),
            size,
        }
        .into()
    }
    pub fn text(plain: impl fmt::Display, opts: impl Into<TextOptions>) -> Self {
        let mut writer = PlainTextWriter::with_opts(opts.into());
        fmt::write(&mut writer, format_args!("{plain}")).unwrap();
        writer.finish()
    }
}

#[derive(Clone, Debug)]
pub struct StackBuilder {
    axis: Axis,
    parts: Vec<StackItem>,
}
impl StackBuilder {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            parts: Default::default(),
        }
    }
    pub fn fit(&mut self, elem: Elem) {
        self.parts.push(StackItem {
            fill_weight: 0,
            elem,
        });
    }
    pub fn fill(&mut self, weight: u16, elem: Elem) {
        self.parts.push(StackItem {
            fill_weight: weight,
            elem,
        });
    }
    pub fn spacing(&mut self, len: u16) {
        self.fit(Elem::empty().with_min_size({
            let mut size = Vec2::default();
            size[self.axis] = len;
            size
        }));
    }
    pub fn build(self) -> Elem {
        let Self { axis, parts } = self;
        ElemKind::Stack(Stack { axis, parts }).into()
    }
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
    pub fn delete_last(&mut self) {
        self.parts.pop();
    }
}

impl From<Stack> for Elem {
    fn from(value: Stack) -> Self {
        ElemKind::Stack(value).into()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ImageSizeMode {
    FillAxis(Axis, u16),
}

mod image_serialize {
    use std::borrow::Cow;

    use super::RgbaImageWrap;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct RgbaImageDefer<'a> {
        width: u32,
        height: u32,
        buf: Cow<'a, [u8]>,
    }

    impl Serialize for RgbaImageWrap {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            RgbaImageDefer {
                width: self.width(),
                height: self.height(),
                buf: self.as_raw().into(),
            }
            .serialize(serializer)
        }
    }
    impl<'de> Deserialize<'de> for RgbaImageWrap {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let RgbaImageDefer { width, height, buf } = Deserialize::deserialize(deserializer)?;
            image::RgbaImage::from_raw(width, height, buf.into())
                .ok_or_else(|| {
                    serde::de::Error::custom("Image buffer is smaller than image dimensions")
                })
                .map(Self)
        }
    }
}
struct RgbaImageWrap(image::RgbaImage);

#[derive(Debug, Serialize, Deserialize)]
struct Image {
    img: RgbaImageWrap,
    sizing: ImageSizeMode,
}

impl std::ops::Deref for RgbaImageWrap {
    type Target = image::RgbaImage;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl fmt::Debug for RgbaImageWrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RgbaImage")
            .field("width", &self.width())
            .field("height", &self.height())
            .field("hash", &{
                let mut hasher = std::hash::DefaultHasher::new();
                std::hash::Hasher::write(&mut hasher, &self.0);
                std::hash::Hasher::finish(&hasher)
            })
            .finish()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub borders: Borders,
    pub border_style: Option<Style>,
    pub border_set: LineSet,
    pub inner: Option<Elem>,
}
impl Block {
    pub fn set_borders_at(&mut self, borders: Borders) {
        self.borders = borders;
    }
    pub fn set_style(&mut self, style: Style) {
        self.border_style = Some(style);
    }
    pub fn set_lines(&mut self, lines: LineSet) {
        self.border_set = lines;
    }
    pub fn set_inner(&mut self, inner: Elem) {
        self.inner = Some(inner);
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Borders {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}
impl Borders {
    pub fn all() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stack {
    axis: Axis,
    parts: Vec<StackItem>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StackItem {
    fill_weight: u16,
    elem: Elem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSet {
    pub vertical: Arc<str>,
    pub horizontal: Arc<str>,
    pub top_right: Arc<str>,
    pub top_left: Arc<str>,
    pub bottom_right: Arc<str>,
    pub bottom_left: Arc<str>,
}

macro_rules! lazy_str {
    ($s:expr) => {{
        static VALUE: std::sync::OnceLock<std::sync::Arc<str>> = std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| std::sync::Arc::<str>::from($s))
            .clone()
    }};
}

impl LineSet {
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
