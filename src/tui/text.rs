use crate::tui::*;

#[derive(Default, Debug, Clone)]
pub struct TextOpts {
    pub style: Option<TextStyle>,
    pub trim_trailing_line: bool,
    //pub sizing: Option<KittyTextSize>,
    #[deprecated = warn_non_exhaustive!()]
    #[doc(hidden)]
    pub __non_exhaustive_struct_update: (),
}
impl From<TextStyle> for TextOpts {
    fn from(value: TextStyle) -> Self {
        Self {
            style: Some(value),
            ..Default::default()
        }
    }
}
impl From<TextModifiers> for TextOpts {
    fn from(value: TextModifiers) -> Self {
        TextStyle::from(value).into()
    }
}

// FIXME: Remove serialize, convert to Print
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct TextStyle {
    pub fg: Option<TermColor>,
    pub bg: Option<TermColor>,
    pub modifiers: Option<TextModifiers>,
    pub underline_color: Option<TermColor>,

    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}
impl From<TextModifiers> for TextStyle {
    fn from(modifier: TextModifiers) -> Self {
        Self {
            modifiers: Some(modifier),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TermColor {
    Reset,
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
    Rgb { r: u8, g: u8, b: u8 },
    AnsiValue(u8),
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct TextModifiers {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub hidden: bool,
    pub strike: bool,

    #[doc(hidden)]
    #[deprecated = warn_non_exhaustive!()]
    pub __non_exhaustive_struct_update: (),
}

pub(crate) struct PlainTextWriter {
    lines: Vec<StackItemRepr>,
    cur_line: String,
    content_offset: usize,
    opts: TextOpts,
}
impl TextStyle {
    pub(crate) fn begin(&self, f: &mut impl fmt::Write) -> fmt::Result {
        use crossterm::Command as _;
        let Self {
            fg,
            bg,
            modifiers,
            underline_color,
            #[expect(deprecated)]
                __non_exhaustive_struct_update: (),
        } = self;

        if let Some(bg) = bg {
            crossterm::style::SetBackgroundColor(bg.to_crossterm()).write_ansi(f)?;
        }
        if let Some(fg) = fg {
            crossterm::style::SetForegroundColor(fg.to_crossterm()).write_ansi(f)?;
        }
        if let Some(ul) = underline_color {
            crossterm::style::SetUnderlineColor(ul.to_crossterm()).write_ansi(f)?;
        }

        if let Some(&TextModifiers {
            bold,
            dim,
            italic,
            underline,
            hidden,
            strike,
            #[expect(deprecated)]
                __non_exhaustive_struct_update: (),
        }) = modifiers.as_ref()
        {
            let mut attrs = crossterm::style::Attributes::none();
            if bold {
                attrs.set(crossterm::style::Attribute::Bold);
            }
            if dim {
                attrs.set(crossterm::style::Attribute::Dim);
            }
            if italic {
                attrs.set(crossterm::style::Attribute::Italic);
            }
            if underline {
                attrs.set(crossterm::style::Attribute::Underlined);
            }
            if hidden {
                attrs.set(crossterm::style::Attribute::Hidden);
            }
            if strike {
                attrs.set(crossterm::style::Attribute::CrossedOut);
            }
            crossterm::style::SetAttributes(attrs).write_ansi(f)?;
        }

        Ok(())
    }
    pub(crate) fn end(&self, f: &mut impl fmt::Write) -> fmt::Result {
        crossterm::Command::write_ansi(&crossterm::style::ResetColor, f)
    }
}
impl PlainTextWriter {
    fn finish_line(&mut self) {
        let (open, content) = self.cur_line.split_at(self.content_offset);

        let content_width = unicode_width::UnicodeWidthStr::width(content);

        let mut line = {
            let open = open.into();
            std::mem::replace(&mut self.cur_line, open)
        };

        if let Some(style) = &self.opts.style {
            style.end(&mut line).unwrap();
        }

        self.lines.push(StackItemRepr {
            elem: ElemRepr::Print {
                raw: line,
                size: Vec2 {
                    x: content_width.try_into().unwrap_or(u16::MAX),
                    y: 1,
                },
            }
            .into(),
            fill_weight: 0,
        })
    }
    pub fn finish(mut self) -> Elem {
        self.finish_line();
        ElemRepr::Stack(StackRepr {
            axis: Axis::Y,
            items: self.lines,
        })
        .into()
    }
    pub fn push_str(&mut self, s: &str) {
        // FIXME: ESCAPE
        // TODO: Implement handling for \r\n (requires remembering whether
        // last char was \r).
        // NOTE: We want this method to be invariant to splitting the string,
        // which is not possible with str::lines(), since it ignores trailing empty lines.
        let mut lines = s.split('\n');
        if let Some(ext_cur) = lines.next() {
            self.cur_line.push_str(ext_cur);
        }
        for new_line in lines {
            self.finish_line();
            self.cur_line.push_str(new_line);
        }
    }
    pub fn with_opts(opts: TextOpts) -> Self {
        let mut cur_line = String::new();
        if let Some(style) = &opts.style {
            style.begin(&mut cur_line).unwrap();
        }
        Self {
            content_offset: cur_line.len(),
            cur_line,
            opts,
            lines: Vec::new(),
        }
    }
}
impl fmt::Write for PlainTextWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}
impl TermColor {
    fn to_crossterm(&self) -> crossterm::style::Color {
        type Out = crossterm::style::Color;
        match *self {
            Self::Reset => Out::Reset,
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
        }
    }
}
