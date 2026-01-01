use crate::tui::*;

#[derive(Clone, Copy)]
pub struct SizingContext {
    pub font_size: Size,
    pub div_w: Option<u16>,
    pub div_h: Option<u16>,
}
pub struct RatatuiRenderContext<'a, 'b> {
    pub picker: &'a ratatui_image::picker::Picker,
    pub frame: &'a mut ratatui::Frame<'b>,
    pub layout: crate::display_panel::RenderedLayout,
}
type RatatuiArea = ratatui::layout::Rect;

impl Tui {
    pub fn calc_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        self.root.calc_auto_size(ctx)
    }
    pub fn render_ratatui(&mut self, ctx: &mut RatatuiRenderContext, sizing: SizingContext) {
        self.root.render_ratatui(ctx, sizing, ctx.frame.area())
    }
}
impl Elem {
    pub fn calc_auto_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        auto_size_invariants(ctx, || match self {
            Self::Subdivide(subdiv) => subdiv.calc_auto_size(ctx),
            Self::Text(text) => text.calc_auto_size(ctx),
            Self::Image(image) => image.calc_auto_size(ctx),
            Self::Block(block) => block.calc_auto_size(ctx),
            Self::Tagged(elem) => elem.elem.calc_auto_size(ctx),
            Self::Empty => Ok(Size::default()),
        })
    }
    pub fn render_ratatui(
        &mut self,
        ctx: &mut RatatuiRenderContext,
        sizing: SizingContext,
        area: RatatuiArea,
    ) {
        match self {
            Self::Subdivide(subdiv) => subdiv.render_ratatui(ctx, sizing, area),
            Self::Image(image) => image.render_ratatui(ctx, sizing, area),
            Self::Block(block) => block.render_ratatui(ctx, sizing, area),
            Self::Text(text) => text.render_ratatui(ctx, sizing, area),
            Self::Tagged(elem) => {
                ctx.layout.insert(area, elem.tag.clone());
                elem.elem.render_ratatui(ctx, sizing, area)
            }
            Self::Empty => (),
        }
    }
}
impl Image {
    pub fn calc_auto_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        auto_size_invariants(ctx, || {
            let mut fit = |axis, other_axis_size| {
                let Size {
                    w: font_w,
                    h: font_h,
                } = ctx.font_size;

                let it = self.load()?;
                let mut ratio = f64::from(it.width()) / f64::from(it.height());
                ratio *= f64::from(font_h) / f64::from(font_w);
                let cells = f64::from(other_axis_size)
                    * match axis {
                        Axis::Horizontal => ratio,
                        Axis::Vertical => 1.0 / ratio,
                    };

                Ok::<_, anyhow::Error>(cells.ceil() as u16)
            };
            Ok(match (ctx.div_w, ctx.div_h) {
                (Some(w), Some(h)) => Size { w, h },
                (Some(w), None) => Size {
                    w,
                    h: fit(Axis::Vertical, w)?,
                },
                (None, Some(h)) => Size {
                    h,
                    w: fit(Axis::Horizontal, h)?,
                },
                (None, None) => anyhow::bail!("Cannot fit image without a fixed dimension"),
            })
        })
    }
    pub fn render_ratatui(
        &mut self,
        ctx: &mut RatatuiRenderContext,
        _: SizingContext,
        area: RatatuiArea,
    ) {
        let Ok(img) = self
            .load()
            .map_err(|err| log::error!("Failed to load image: {err}"))
        else {
            return;
        };
        let Ok(img) = ctx
            .picker
            .new_protocol(img.clone(), area, ratatui_image::Resize::Fit(None))
            .map_err(|err| log::error!("Failed to create protocol: {err}"))
        else {
            return;
        };
        ctx.frame
            .render_widget(ratatui_image::Image::new(&img), area);
    }
}
impl Subdiv {
    fn calc_elem_size(part: &mut DivPart, ctx: SizingContext, axis: Axis) -> anyhow::Result<Size> {
        part.elem
            .calc_auto_size(Self::inner_sizing_arg(&part.constr, ctx, axis))
    }
    fn inner_sizing_arg(constr: &Constr, ctx: SizingContext, axis: Axis) -> SizingContext {
        match *constr {
            Constr::Length(l) => match axis {
                Axis::Horizontal => SizingContext {
                    div_w: Some(l),
                    ..ctx
                },
                Axis::Vertical => SizingContext {
                    div_h: Some(l),
                    ..ctx
                },
            },
            Constr::Fill(_) | Constr::Auto => match axis {
                Axis::Horizontal => SizingContext { div_w: None, ..ctx },
                Axis::Vertical => SizingContext { div_h: None, ..ctx },
            },
        }
    }
    pub fn calc_auto_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        auto_size_invariants(ctx, || {
            let mut size = Size::default();
            for part in &mut self.parts {
                let elem_size = Self::calc_elem_size(part, ctx, self.axis)?;
                let horiz = (&mut size.w, elem_size.w);
                let vert = (&mut size.h, elem_size.h);
                let ((adst, asrc), (mdst, msrc)) = match self.axis {
                    Axis::Horizontal => (horiz, vert),
                    Axis::Vertical => (vert, horiz),
                };
                *adst += asrc;
                *mdst = msrc.max(*mdst);
            }
            Ok(size)
        })
    }
    pub fn render_ratatui(
        &mut self,
        ctx: &mut RatatuiRenderContext,
        sizing: SizingContext,
        area: RatatuiArea,
    ) {
        let areas = ratatui::layout::Layout::default()
            .direction(match self.axis {
                Axis::Horizontal => ratatui::layout::Direction::Horizontal,
                Axis::Vertical => ratatui::layout::Direction::Vertical,
            })
            .constraints(self.parts.iter_mut().map(|part| match part.constr {
                Constr::Length(l) => ratatui::layout::Constraint::Length(l),
                Constr::Fill(n) => ratatui::layout::Constraint::Fill(n),
                Constr::Auto => ratatui::layout::Constraint::Length(
                    match Self::calc_elem_size(part, sizing, self.axis) {
                        Ok(elem_size) => elem_size.get(self.axis),
                        Err(err) => {
                            log::error!("Skipping element {part:?} with broken size: {err}");
                            0
                        }
                    },
                ),
            }))
            .split(area);

        assert_eq!(areas.len(), self.parts.len());
        for (area, part) in areas.iter().zip(&mut self.parts) {
            part.elem.render_ratatui(
                ctx,
                Self::inner_sizing_arg(&part.constr, sizing, self.axis),
                *area,
            );
        }
    }
}
impl Text {
    pub fn calc_auto_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        auto_size_invariants(ctx, || {
            let mut size = Size::default();
            for line in self.body.lines() {
                size.w = size
                    .w
                    .max(line.chars().count().try_into().unwrap_or(u16::MAX));
                size.h = size.h.saturating_add(1);
            }
            Ok(size)
        })
    }
    pub fn render_ratatui(
        &mut self,
        ctx: &mut RatatuiRenderContext,
        _: SizingContext,
        area: RatatuiArea,
    ) {
        ctx.frame
            .render_widget(ratatui::widgets::Paragraph::new(&self.body as &str), area)
    }
}
impl Block {
    fn extra_dim(&self) -> Size {
        let Borders {
            top,
            bottom,
            left,
            right,
        } = self.borders;
        Size {
            w: u16::from(left) + u16::from(right),
            h: u16::from(top) + u16::from(bottom),
        }
    }
    fn inner_sizing_arg(&self, mut ctx: SizingContext) -> SizingContext {
        let Size { w, h } = self.extra_dim();
        if let Some(div_w) = &mut ctx.div_w {
            *div_w -= w;
        }
        if let Some(div_h) = &mut ctx.div_h {
            *div_h -= h;
        }
        ctx
    }
    pub fn calc_auto_size(&mut self, ctx: SizingContext) -> anyhow::Result<Size> {
        auto_size_invariants(ctx, || {
            let inner_ctx = self.inner_sizing_arg(ctx);
            let mut size = self
                .inner
                .as_mut()
                .map(|it| it.calc_auto_size(inner_ctx))
                .transpose()?
                .unwrap_or_default();
            let Size { w, h } = self.extra_dim();
            size.w = size.w.saturating_add(w);
            size.h = size.h.saturating_add(h);
            Ok(size)
        })
    }
    pub fn render_ratatui(
        &mut self,
        ctx: &mut RatatuiRenderContext,
        sizing: SizingContext,
        area: RatatuiArea,
    ) {
        let inner_sizing_arg = self.inner_sizing_arg(sizing);
        let Self {
            borders,
            border_style,
            border_set,
            inner,
        } = self;
        let block = ratatui::widgets::Block::new()
            .borders({
                let Borders {
                    top,
                    bottom,
                    left,
                    right,
                } = *borders;
                let mut borders = ratatui::widgets::Borders::default();
                borders.set(ratatui::widgets::Borders::TOP, top);
                borders.set(ratatui::widgets::Borders::BOTTOM, bottom);
                borders.set(ratatui::widgets::Borders::LEFT, left);
                borders.set(ratatui::widgets::Borders::RIGHT, right);
                borders
            })
            .border_style(convert_style(border_style))
            .border_set({
                let LineSet {
                    vertical,
                    horizontal,
                    top_right,
                    top_left,
                    bottom_right,
                    bottom_left,
                    ..
                } = border_set;
                ratatui::symbols::border::Set {
                    top_left,
                    top_right,
                    bottom_left,
                    bottom_right,
                    vertical_left: vertical,
                    vertical_right: vertical,
                    horizontal_top: horizontal,
                    horizontal_bottom: horizontal,
                }
            });
        if let Some(inner) = inner {
            inner.render_ratatui(ctx, inner_sizing_arg, block.inner(area));
        }
        ctx.frame.render_widget(block, area);
    }
}
fn auto_size_invariants(
    ctx: SizingContext,
    f: impl FnOnce() -> anyhow::Result<Size>,
) -> anyhow::Result<Size> {
    if let (Some(w), Some(h)) = (ctx.div_w, ctx.div_h) {
        return Ok(Size { w, h });
    }
    let mut size = f()?;
    if let Some(w) = ctx.div_w {
        size.w = w;
    }
    if let Some(h) = ctx.div_h {
        size.h = h;
    }
    Ok(size)
}
fn convert_color(color: &Color) -> ratatui::style::Color {
    use crate::tui::Color as IC;
    use ratatui::style::Color as OC;
    match *color {
        IC::Reset => OC::Reset,
        IC::Black => OC::Black,
        IC::Red => OC::Red,
        IC::Green => OC::Green,
        IC::Yellow => OC::Yellow,
        IC::Blue => OC::Blue,
        IC::Magenta => OC::Magenta,
        IC::Cyan => OC::Cyan,
        IC::Gray => OC::Gray,
        IC::DarkGray => OC::DarkGray,
        IC::LightRed => OC::LightRed,
        IC::LightGreen => OC::LightGreen,
        IC::LightYellow => OC::LightYellow,
        IC::LightBlue => OC::LightBlue,
        IC::LightMagenta => OC::LightMagenta,
        IC::LightCyan => OC::LightCyan,
        IC::White => OC::White,
        IC::Rgb(r, g, b) => OC::Rgb(r, g, b),
        IC::Indexed(i) => OC::Indexed(i),
    }
}
fn convert_style(style: &Style) -> ratatui::style::Style {
    let Style {
        fg,
        bg,
        modifier,
        underline_color,
    } = style;
    ratatui::style::Style {
        fg: fg.as_ref().map(convert_color),
        bg: bg.as_ref().map(convert_color),
        underline_color: underline_color.as_ref().map(convert_color),
        add_modifier: {
            let Modifier {
                bold,
                dim,
                italic,
                underline,
                hidden,
                strike,
            } = *modifier;
            use ratatui::style::Modifier as OM;
            let mut m = OM::default();
            m.set(OM::BOLD, bold);
            m.set(OM::ITALIC, italic);
            m.set(OM::DIM, dim);
            m.set(OM::UNDERLINED, underline);
            m.set(OM::HIDDEN, hidden);
            m.set(OM::CROSSED_OUT, strike);
            m
        },
        sub_modifier: Default::default(),
    }
}
