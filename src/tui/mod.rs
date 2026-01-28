mod render;
pub use render::*;
mod layout;
pub use layout::*;

use std::{fmt, sync::Arc};

use crate::utils::Callback;

#[derive(Debug, Clone)]
enum ElemKind {
    Print { raw: Arc<str>, size: Vec2<u16> },
    Image(Arc<Image>),
    Stack(Stack),
    Block(Arc<BlockBuilder>),
}

#[derive(Debug, Clone)]
pub struct Elem {
    kind: ElemKind,
    elem_id: u64,

    min_size: Vec2<u16>,

    // FIXME: Merge interact attrs,
    // callbacks return optional replacement
    // that stays active while tooltip/menu
    // is active or if none is provided, until
    // element is unhovered
    interact: Option<InteractCallback>,
    tooltip: Option<HoverCallback>,
    hovered: Option<Arc<Elem>>,
}

impl Elem {
    pub fn is_identical(&self, other: &Elem) -> bool {
        self.elem_id == other.elem_id
    }
    fn new(kind: ElemKind) -> Self {
        use std::sync::atomic::*;
        static ELEM_ID: AtomicU64 = AtomicU64::new(0);
        Self {
            kind,
            elem_id: ELEM_ID.fetch_add(1, Ordering::Relaxed),
            min_size: Default::default(),
            interact: None,
            tooltip: None,
            hovered: None,
        }
    }

    pub fn with_min_size(mut self, min_size: Vec2<u16>) -> Self {
        self.min_size = self.min_size.combine(min_size, std::cmp::max);
        self
    }

    pub fn empty() -> Self {
        Self::new(ElemKind::Print {
            raw: Default::default(),
            size: Default::default(),
        })
    }
    pub fn image(img: image::RgbaImage, sizing: ImageSizeMode) -> Self {
        Self::new(ElemKind::Image(Arc::new(Image { img, sizing })))
    }

    pub fn with_interact(mut self, on_interact: impl Into<InteractCallback>) -> Self {
        self.interact = Some(on_interact.into());
        self
    }
    pub fn with_tooltip(mut self, tooltip: impl Into<HoverCallback>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
    pub fn with_hovered(mut self, hovered: impl Into<Elem>) -> Self {
        let hovered = hovered.into();
        self.hovered = Some(Arc::new(hovered));
        self
    }
    pub fn build_block(init: impl FnOnce(&mut BlockBuilder)) -> Self {
        let mut builder = BlockBuilder {
            borders: Default::default(),
            border_style: Default::default(),
            border_set: LineSet::normal(),
            inner: None,
        };
        init(&mut builder);
        Self::new(ElemKind::Block(Arc::new(builder)))
    }
    pub fn build_stack(axis: Axis, init: impl FnOnce(&mut StackBuilder)) -> Self {
        let mut builder = StackBuilder::new(axis);
        init(&mut builder);
        builder.build()
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
        self.push(StackItem {
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
    pub fn push(&mut self, item: StackItem) {
        self.parts.push(item);
    }
    pub fn build(self) -> Elem {
        let Self { axis, parts } = self;
        Elem::new(ElemKind::Stack(Stack {
            axis,
            parts: parts.into(),
        }))
    }
}

impl From<Stack> for Elem {
    fn from(value: Stack) -> Self {
        Self::new(ElemKind::Stack(value))
    }
}
impl<D: fmt::Display> From<RawPrint<D>> for Elem {
    fn from(value: RawPrint<D>) -> Self {
        let RawPrint { raw, size } = value;
        Self::new(ElemKind::Print {
            raw: raw.to_string().into(),
            size,
        })
    }
}
impl<S: fmt::Display> From<PlainLines<S>> for Elem {
    #[track_caller]
    fn from(value: PlainLines<S>) -> Self {
        Elem::build_stack(Axis::Y, |stack| {
            for line in value.text.to_string().lines() {
                stack.fit(RawPrint::plain(line).styled(value.style).into())
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct RawPrint<D> {
    raw: D,
    size: Vec2<u16>,
}
impl<D> RawPrint<D> {
    #[track_caller]
    pub fn plain(text: D) -> Self
    where
        D: AsRef<str>,
    {
        // FIXME: Strip control characters
        let s = text.as_ref();
        if s.chars().any(|c| c.is_ascii_control()) {
            log::warn!("Plain text {s:?} should not contain ascii control chars");
        }
        Self {
            size: Vec2 {
                x: unicode_width::UnicodeWidthStr::width(s)
                    .try_into()
                    .unwrap_or(u16::MAX),
                y: 1,
            },
            raw: text,
        }
    }

    pub fn center_symbol(sym: D, width: u16) -> RawPrint<impl fmt::Display>
    where
        D: fmt::Display,
    {
        RawPrint {
            raw: KittyTextSize::center_width(width).apply(sym),
            size: Vec2 { x: width, y: 1 },
        }
    }

    pub fn styled(self, style: Style) -> RawPrint<impl fmt::Display>
    where
        D: fmt::Display,
    {
        let Self { size, raw } = self;
        RawPrint {
            size,
            raw: style.apply(raw),
        }
    }

    pub fn map_display<T>(self, f: impl FnOnce(D) -> T) -> RawPrint<T> {
        RawPrint {
            raw: f(self.raw),
            size: self.size,
        }
    }
}

#[derive(Default, Debug)]
pub struct PlainLines<S> {
    text: S,
    style: Style,
}
impl<S> PlainLines<S> {
    pub fn new(text: S) -> Self {
        Self {
            text,
            style: Default::default(),
        }
    }
    pub fn styled(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

pub type InteractCallback = Callback<InteractArgs, Option<Elem>>;
pub type HoverCallback = Callback<HoverArgs, Option<Elem>>;
#[derive(Debug)]
pub struct HoverArgs {
    _p: (),
}

#[derive(Debug)]
pub struct InteractArgs {
    pub kind: InteractKind,
    _p: (),
}

#[derive(Clone, Copy, Debug)]
pub enum ImageSizeMode {
    FillAxis(Axis, u16),
}
#[derive(Clone)]
struct Image {
    img: image::RgbaImage,
    sizing: ImageSizeMode,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hasher::write(&mut hasher, &self.img);
        let hash = std::hash::Hasher::finish(&hasher);
        f.debug_tuple("Image").field(&hash).finish()
    }
}
#[derive(Debug, Clone)]
pub struct BlockBuilder {
    borders: Borders,
    border_style: Style,
    border_set: LineSet,
    inner: Option<Elem>,
}
impl BlockBuilder {
    pub fn set_borders_at(&mut self, borders: Borders) {
        self.borders = borders;
    }
    pub fn set_style(&mut self, style: Style) {
        self.border_style = style;
    }
    pub fn set_lines(&mut self, lines: LineSet) {
        self.border_set = lines;
    }
    pub fn set_inner(&mut self, inner: Elem) {
        self.inner = Some(inner);
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Debug, Clone)]
struct Stack {
    axis: Axis,
    parts: Arc<[StackItem]>,
}

#[derive(Debug, Clone)]
pub struct StackItem {
    fill_weight: u16,
    elem: Elem,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub modifier: Modifier,
    pub underline_color: Option<Color>,

    #[doc(hidden)]
    pub __non_exhaustive: (),
}
pub type Color = crossterm::style::Color;
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
#[repr(Rust, packed)]
pub struct Modifier {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub hidden: bool,
    pub strike: bool,

    #[doc(hidden)]
    pub __non_exhaustive: (),
}

// TODO: enums
#[derive(Default, Debug, Clone, Copy)]
pub struct KittyTextSize {
    pub s: Option<u16>,
    pub w: Option<u16>,
    pub n: Option<u16>,
    pub d: Option<u16>,
    pub v: Option<u16>,
    pub h: Option<u16>,
}
impl KittyTextSize {
    pub fn center_width(width: u16) -> Self {
        // https://sw.kovidgoyal.net/kitty/text-sizing-protocol/
        // - w      sets the width of the multicell
        // - h      ceter the text horizontally
        // - n,d    use fractional scale of 1:1. kitty ignores w without this
        Self {
            w: Some(width),
            h: Some(2),
            d: Some(1),
            n: Some(1),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
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

    #[expect(dead_code)]
    pub fn rounded() -> Self {
        Self {
            top_right: lazy_str!("╮"),
            top_left: lazy_str!("╭"),
            bottom_right: lazy_str!("╯"),
            bottom_left: lazy_str!("╰"),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
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

    #[expect(dead_code)]
    pub fn light_double_dashed() -> Self {
        Self {
            vertical: lazy_str!("╎"),
            horizontal: lazy_str!("╌"),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_double_dashed() -> Self {
        Self {
            vertical: lazy_str!("╏"),
            horizontal: lazy_str!("╍"),
            ..Self::thick()
        }
    }

    #[expect(dead_code)]
    pub fn light_triple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┆"),
            horizontal: lazy_str!("┄"),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_triple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┇"),
            horizontal: lazy_str!("┅"),
            ..Self::thick()
        }
    }

    #[expect(dead_code)]
    pub fn light_quadruple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┊"),
            horizontal: lazy_str!("┈"),
            ..Self::normal()
        }
    }

    #[expect(dead_code)]
    pub fn heavy_quadruple_dashed() -> Self {
        Self {
            vertical: lazy_str!("┋"),
            horizontal: lazy_str!("┉"),
            ..Self::thick()
        }
    }
}
