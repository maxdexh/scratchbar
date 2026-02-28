use std::{
    borrow::{Borrow, BorrowMut},
    fmt::{self, Write as _},
    num::{NonZeroU16, NonZeroUsize},
};

use scratchbar::tui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum HorizontalAlign {
    Left,
    Right,
    Center,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum VerticalAlign {
    Top,
    Bottom,
    Center,
}

// TODO: Use crossterm
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[allow(dead_code)]
#[non_exhaustive]
pub enum Color {
    /// Specifies that no color should be set.
    #[default]
    Unset,
    Black,
    DarkGrey,
    Red,
    DarkRed,
    Green,
    DarkGreen,
    Yellow,
    DarkYellow,
    Blue,
    DarkBlue,
    Magenta,
    DarkMagenta,
    Cyan,
    DarkCyan,
    White,
    Grey,
    Rgb {
        r: u8,
        g: u8,
        b: u8,
    },
    AnsiValue(u8),
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Attrs {
    flags: u8,
}

#[allow(dead_code)]
const _: () = {
    macro_rules! attr {
        ($name:ident, $flag:expr, $get:ident, $set:ident, $with:ident) => {
            impl Attrs {
                pub fn $name() -> Self {
                    Self { flags: $flag }
                }
                pub fn $get(&self) -> bool {
                    self.flags & $flag != 0
                }
                pub fn $set(&mut self, val: bool) {
                    if self.$get() != val {
                        self.flags ^= $flag;
                    }
                }
                pub fn $with(mut self, val: bool) -> Self {
                    self.$set(val);
                    self
                }
            }
        };
    }
    attr!(bold, 1 << 0, is_bold, set_bold, with_bold);
    attr!(dim, 1 << 1, is_dim, set_dim, with_dim);
    attr!(italic, 1 << 1, is_italic, set_italic, with_italic);
    attr!(crossout, 1 << 1, is_crossout, set_crossout, with_crossout);
    attr!(
        underlined,
        1 << 1,
        is_underlined,
        set_underlined,
        with_underlined
    );
};

#[derive(Clone, Debug)]
pub struct Subscale {
    pub numerator: NonZeroU16,
    pub denominator: NonZeroU16,
    pub vertical: VerticalAlign,
    pub horizontal: HorizontalAlign,
}
impl Default for Subscale {
    fn default() -> Self {
        Self {
            numerator: NonZeroU16::new(1).unwrap(),
            denominator: NonZeroU16::new(1).unwrap(),
            vertical: VerticalAlign::Top,
            horizontal: HorizontalAlign::Left,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TextOpts {
    pub fg_color: Color,
    pub bg_color: Color,
    pub underline_color: Color,
    pub attrs: Attrs,
    pub scale: NonZeroU16,
    pub subscale: Option<Subscale>,
}
impl TextOpts {
    pub fn with(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}
impl Default for TextOpts {
    fn default() -> Self {
        Self {
            fg_color: Color::Unset,
            bg_color: Color::Unset,
            underline_color: Color::Unset,
            attrs: Attrs::default(),
            scale: NonZeroU16::new(1).unwrap(),
            subscale: None,
        }
    }
}
impl From<Attrs> for TextOpts {
    fn from(value: Attrs) -> Self {
        Self {
            attrs: value,
            ..Default::default()
        }
    }
}
impl From<Subscale> for TextOpts {
    fn from(value: Subscale) -> Self {
        Self {
            subscale: Some(value),
            ..Default::default()
        }
    }
}
impl From<VerticalAlign> for TextOpts {
    fn from(value: VerticalAlign) -> Self {
        Subscale {
            vertical: value,
            ..Default::default()
        }
        .into()
    }
}
impl From<HorizontalAlign> for TextOpts {
    fn from(value: HorizontalAlign) -> Self {
        Subscale {
            horizontal: value,
            ..Default::default()
        }
        .into()
    }
}
impl TextOpts {
    pub fn render(&self, lines: &str) -> tui::Elem {
        self.render_lines(lines.lines())
    }

    pub fn render_line(&self, line: &str) -> tui::Elem {
        let mut f = LineFormatter::enter_new(String::new(), self);
        for grapheme in graphemes(line) {
            if let Some(width) = NonZeroUsize::new(width(grapheme)) {
                f.write_cell(grapheme, width);
            } else {
                f.write_direct(grapheme, None);
            }
        }
        f.render()
    }

    pub fn render_lines(&self, lines: impl IntoIterator<Item: AsRef<str>>) -> tui::Elem {
        tui::Elem::stack(
            tui::Axis::Y,
            lines.into_iter().map(|line| tui::StackItem {
                elem: self.render_line(line.as_ref()),
                opts: Default::default(),
            }),
            tui::StackOpts::default(),
        )
    }

    pub fn render_cell(&self, content: impl fmt::Display, width: NonZeroUsize) -> tui::Elem {
        let mut fmt = LineFormatter::enter_new(String::new(), self);
        fmt.write_cell(content, width);
        fmt.render()
    }
}

struct EscapeSafeWriter<W> {
    inner: W,
}
impl<W: BorrowMut<String>> fmt::Write for EscapeSafeWriter<W> {
    fn write_str(&mut self, mut s: &str) -> fmt::Result {
        let inner = self.inner.borrow_mut();
        while let Some((good, rest)) = s.split_once(|c: char| c.is_ascii_control()) {
            s = rest;
            inner.push_str(good);
        }
        inner.push_str(s);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        if !c.is_ascii_control() {
            self.inner.borrow_mut().push(c);
        }
        Ok(())
    }
}

pub fn graphemes(text: &str) -> impl Iterator<Item = &str> {
    unicode_segmentation::UnicodeSegmentation::graphemes(text, true)
}
pub fn width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

// TODO: Consider writing into a fmt::Formatter
// to get rid of a copy.
pub struct LineFormatter<W, O> {
    opts: O,
    out: EscapeSafeWriter<W>,
    width: u16,
    reset_color: bool,
}
impl<W, O> LineFormatter<W, O>
where
    W: BorrowMut<String>,
    O: Borrow<TextOpts>,
{
    pub fn enter_new(mut out: W, opts: O) -> Self {
        let mut reset_color = false;

        {
            use crossterm::Command as _;
            let out = out.borrow_mut();
            let opts = opts.borrow();
            if let Some(bg) = opts.bg_color.to_crossterm() {
                reset_color = true;
                crossterm::style::SetBackgroundColor(bg)
                    .write_ansi(out)
                    .unwrap();
            }
            if let Some(fg) = opts.fg_color.to_crossterm() {
                reset_color = true;
                crossterm::style::SetForegroundColor(fg)
                    .write_ansi(out)
                    .unwrap();
            }
            if let Some(ul) = opts.underline_color.to_crossterm() {
                reset_color = true;
                crossterm::style::SetUnderlineColor(ul)
                    .write_ansi(out)
                    .unwrap();
            }
        }

        Self {
            opts,
            out: EscapeSafeWriter { inner: out },
            width: 0,
            reset_color,
        }
    }

    pub fn finish(self) -> (W, O, tui::Size) {
        let Self {
            opts,
            width,
            reset_color,
            out: EscapeSafeWriter { mut inner },
        } = self;

        if reset_color {
            use crossterm::Command as _;
            crossterm::style::ResetColor
                .write_ansi(inner.borrow_mut())
                .unwrap();
        }

        let size = tui::Size {
            width,
            height: opts.borrow().scale.get(),
        };

        (inner, opts, size)
    }

    pub fn add_width(&mut self, width: usize) {
        self.width = self
            .width
            .saturating_add(width.try_into().unwrap_or(u16::MAX));
    }

    fn open_sizing_context(&mut self, width: Option<NonZeroUsize>) {
        let inner = self.out.inner.borrow_mut();
        let opts = self.opts.borrow();

        inner.push_str("\x1b]66;");

        let mut needs_colon = false;
        let mut flag = |flag: &str, val: usize| {
            if needs_colon {
                inner.push(':');
            } else {
                needs_colon = true;
            }
            inner.push_str(flag);
            write!(inner, "={val}").unwrap();
        };

        if opts.scale.get() != 1 {
            flag("s", opts.scale.get().into());
        }
        if let Some(width) = width {
            flag("w", width.get());
        }
        if let Some(Subscale {
            numerator,
            denominator,
            vertical,
            horizontal,
        }) = opts.subscale
        {
            flag("n", numerator.get().into());
            flag("d", denominator.get().into());
            match vertical {
                VerticalAlign::Top => (),
                VerticalAlign::Bottom => flag("v", 1),
                VerticalAlign::Center => flag("v", 2),
            }
            match horizontal {
                HorizontalAlign::Left => (),
                HorizontalAlign::Right => flag("h", 1),
                HorizontalAlign::Center => flag("h", 2),
            }
        }
        inner.push(';');
    }

    pub fn write_direct(&mut self, text: impl fmt::Display, cell_width: Option<NonZeroUsize>) {
        self.open_sizing_context(cell_width);
        write!(self.out, "{text}").unwrap();
        self.out.inner.borrow_mut().push('\x07')
    }

    pub fn write_cell(&mut self, content: impl fmt::Display, width: NonZeroUsize) {
        self.add_width(width.get());
        self.write_direct(content, Some(width))
    }
}
impl<O: Borrow<TextOpts>, W: fmt::Display + BorrowMut<String>> LineFormatter<W, O> {
    fn render(self) -> tui::Elem {
        let (text, _, size) = self.finish();
        tui::Elem::raw_print(text).with_min_size(size)
    }
}

impl Color {
    fn to_crossterm(self) -> Option<crossterm::style::Color> {
        type Out = crossterm::style::Color;
        Some(match self {
            Self::Unset => return None,
            Self::Black => Out::Black,
            Self::DarkGrey => Out::DarkGrey,
            Self::Red => Out::Red,
            Self::DarkRed => Out::DarkRed,
            Self::Green => Out::Green,
            Self::DarkGreen => Out::DarkGreen,
            Self::Yellow => Out::Yellow,
            Self::DarkYellow => Out::DarkYellow,
            Self::Blue => Out::Blue,
            Self::DarkBlue => Out::DarkBlue,
            Self::Magenta => Out::Magenta,
            Self::DarkMagenta => Out::DarkMagenta,
            Self::Cyan => Out::Cyan,
            Self::DarkCyan => Out::DarkCyan,
            Self::White => Out::White,
            Self::Grey => Out::Grey,
            Self::Rgb { r, g, b } => Out::Rgb { r, g, b },
            Self::AnsiValue(v) => Out::AnsiValue(v),
        })
    }
}

pub fn render_with_hover(
    normal: &TextOpts,
    tag: tui::CustomId,
    hovered: &TextOpts,
    mut render: impl FnMut(&TextOpts) -> tui::Elem,
) -> tui::Elem {
    render(normal).interactive_hover(tag, render(hovered))
}
