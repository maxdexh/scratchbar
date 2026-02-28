use std::fmt;

use scratchbar::tui;

pub mod text;

#[derive(Clone, Debug)]
pub struct StackBuilder {
    axis: tui::Axis,
    pub items: Vec<tui::StackItem>,
    pub opts: tui::StackOpts,
}
impl StackBuilder {
    pub fn new(axis: tui::Axis) -> Self {
        Self {
            axis,
            items: Vec::new(),
            opts: Default::default(),
        }
    }
    pub fn push(&mut self, item: impl Into<tui::StackItem>) {
        self.items.push(item.into());
    }
    pub fn fill(&mut self, fill_weight: u16, elem: tui::Elem) {
        self.push(tui::StackItem {
            elem,
            opts: tui::StackItemOpts {
                fill_weight,
                ..Default::default()
            },
        });
    }
    pub fn spacing(&mut self, len: u16) {
        self.push(tui::Elem::spacing(self.axis, len));
    }
    pub fn build(self) -> tui::Elem {
        tui::Elem::stack(self.axis, self.items, self.opts)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn delete_last(&mut self) {
        self.items.pop();
    }
}

pub fn rgba_img_fill_axis(img: image::RgbaImage, fill_axis: tui::Axis, fill_len: u16) -> tui::Elem {
    // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
    // - \x1b_G...\x1b\\: kitty graphics apc
    // - a=T: Transfer and display
    // - f=32: 32-bit RGBA
    // - C=1: Do not move the cursor behind the image after drawing. If the image is on the
    //   last line, the first line would move to scrollback (effectively a clear if there is
    //   only one line, like in the bar).
    // - s and v specify the image's dimensions
    tui::Elem::raw_print(format_args!(
        "\x1b_Ga=T,f=32,C=1,s={},v={},{}={fill_len};{}\x1b\\",
        img.width(),
        img.height(),
        match fill_axis {
            tui::Axis::X => "c",
            tui::Axis::Y => "r",
        },
        base64::display::Base64Display::new(
            img.as_raw(),
            &base64::engine::general_purpose::STANDARD,
        )
    ))
    .with_min_axis(tui::MinAxis {
        axis: fill_axis,
        len: fill_len,
        aspect_width: img.width(),
        aspect_height: img.height(),
    })
}

#[derive(Debug, Clone, Default)]
pub struct BlockOpts {
    pub borders: BlockBorders,
    pub inner: Option<tui::Elem>,
}
pub fn block<D: fmt::Display>(lines: BlockLines<D>, opts: BlockOpts) -> tui::Elem {
    let BlockLines {
        vertical,
        horizontal,
        top_right,
        top_left,
        bottom_right,
        bottom_left,
    } = lines;
    let BlockOpts {
        borders:
            BlockBorders {
                top,
                bottom,
                left,
                right,
            },
        inner,
    } = opts;

    let mut grid = [const { [const { None }; 3] }; 3];
    if let Some(inner) = inner {
        grid[1][1] = Some(inner.into());
    }

    let byone = tui::Size {
        width: 1,
        height: 1,
    };
    if left && top {
        grid[0][0] = Some(tui::Elem::raw_print(top_left).with_min_size(byone).into());
    }
    if left && bottom {
        grid[0][2] = Some(
            tui::Elem::raw_print(bottom_left)
                .with_min_size(byone)
                .into(),
        );
    }
    if right && top {
        grid[2][0] = Some(tui::Elem::raw_print(top_right).with_min_size(byone).into());
    }
    if right && bottom {
        grid[2][2] = Some(
            tui::Elem::raw_print(bottom_right)
                .with_min_size(byone)
                .into(),
        );
    }
    for (idx, cond) in [(0, left), (2, right)] {
        if cond {
            grid[idx][1] = Some(tui::StackItem {
                elem: tui::Elem::fill_cells_single(&vertical).with_min_size(tui::Size {
                    width: 1,
                    height: 0,
                }),
                opts: tui::StackItemOpts {
                    fill_weight: 1,
                    ..Default::default()
                },
            });
        }
    }
    for (idx, cond) in [(0, top), (2, bottom)] {
        if cond {
            grid[1][idx] = Some(tui::StackItem {
                elem: tui::Elem::fill_cells_single(&horizontal).with_min_size(tui::Size {
                    width: 0,
                    height: 1,
                }),
                opts: tui::StackItemOpts {
                    fill_weight: 1,
                    ..Default::default()
                },
            });
        }
    }
    let [l, m, r] = grid.map(|parts| {
        tui::Elem::stack(
            tui::Axis::Y,
            parts.into_iter().flatten(),
            tui::StackOpts::default(),
        )
    });
    tui::Elem::stack(
        tui::Axis::X,
        [
            l.into(),
            tui::StackItem {
                elem: m,
                opts: tui::StackItemOpts {
                    fill_weight: 1,
                    ..Default::default()
                },
            },
            r.into(),
        ],
        tui::StackOpts::default(),
    )
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Debug, Clone)]
pub struct BlockLines<D> {
    vertical: D,
    horizontal: D,
    top_right: D,
    top_left: D,
    bottom_right: D,
    bottom_left: D,
}
impl<D: fmt::Display> BlockLines<D> {
    pub fn map<E>(self, mut f: impl FnMut(D) -> E) -> BlockLines<E> {
        let Self {
            vertical,
            horizontal,
            top_right,
            top_left,
            bottom_right,
            bottom_left,
        } = self;
        BlockLines {
            vertical: f(vertical),
            horizontal: f(horizontal),
            top_right: f(top_right),
            top_left: f(top_left),
            bottom_right: f(bottom_right),
            bottom_left: f(bottom_left),
        }
    }
    pub fn apply_crossterm(
        self,
        ct: impl Into<crossterm::style::ContentStyle>,
    ) -> BlockLines<crossterm::style::StyledContent<D>> {
        let style = ct.into();
        self.map(|it| style.apply(it))
    }
}

#[allow(dead_code)]
impl BlockLines<&'static str> {
    pub fn normal() -> Self {
        Self {
            vertical: "│",
            horizontal: "─",
            top_right: "┐",
            top_left: "┌",
            bottom_right: "┘",
            bottom_left: "└",
        }
    }

    pub fn rounded() -> Self {
        Self {
            top_right: "╮",
            top_left: "╭",
            bottom_right: "╯",
            bottom_left: "╰",
            ..Self::normal()
        }
    }

    pub fn double() -> Self {
        Self {
            vertical: "║",
            horizontal: "═",
            top_right: "╗",
            top_left: "╔",
            bottom_right: "╝",
            bottom_left: "╚",
        }
    }

    pub fn thick() -> Self {
        Self {
            vertical: "┃",
            horizontal: "━",
            top_right: "┓",
            top_left: "┏",
            bottom_right: "┛",
            bottom_left: "┗",
        }
    }

    pub fn light_double_dashed() -> Self {
        Self {
            vertical: "╎",
            horizontal: "╌",
            ..Self::normal()
        }
    }

    pub fn heavy_double_dashed() -> Self {
        Self {
            vertical: "╏",
            horizontal: "╍",
            ..Self::thick()
        }
    }

    pub fn light_triple_dashed() -> Self {
        Self {
            vertical: "┆",
            horizontal: "┄",
            ..Self::normal()
        }
    }

    pub fn heavy_triple_dashed() -> Self {
        Self {
            vertical: "┇",
            horizontal: "┅",
            ..Self::thick()
        }
    }

    pub fn light_quadruple_dashed() -> Self {
        Self {
            vertical: "┊",
            horizontal: "┈",
            ..Self::normal()
        }
    }

    pub fn heavy_quadruple_dashed() -> Self {
        Self {
            vertical: "┋",
            horizontal: "┉",
            ..Self::thick()
        }
    }
}
