use std::{fmt, io::Write};

use crate::tui::*;

pub(super) trait Render {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()>;
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16>;
}

#[derive(Debug)]
pub(super) struct RenderCtx<'a, W> {
    sizing: &'a SizingArgs,
    writer: W,
    layout: &'a mut RenderedLayout,
}
#[derive(Debug, Clone)]
pub(crate) struct SizingArgs {
    pub font_size: Vec2<u16>,
}
pub(crate) fn calc_min_size(elem: &Elem, args: &SizingArgs) -> Vec2<u16> {
    elem.calc_min_size(args)
}
pub(crate) fn render(
    elem: &Elem,
    area: Area,
    writer: &mut impl Write,
    sizing: &SizingArgs,
    old_layout: Option<&RenderedLayout>,
) -> std::io::Result<RenderedLayout> {
    crossterm::queue!(
        writer,
        crossterm::terminal::BeginSynchronizedUpdate,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
    )?;
    let last_mouse_pos = old_layout.as_ref().and_then(|it| it.last_mouse_pos);
    let mut layout = RenderedLayout {
        widgets: Default::default(),
        last_mouse_pos,
        last_hover_elem: None,
    };
    elem.render(
        &mut RenderCtx {
            sizing,
            writer: &mut *writer,
            layout: &mut layout,
        },
        area,
    )?;
    crossterm::execute!(writer, crossterm::terminal::EndSynchronizedUpdate)?;
    Ok(layout)
}

impl Render for Elem {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        self.0.render(ctx, area)
    }
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        self.0.calc_min_size(args)
    }
}
impl Render for ElemRepr {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        crossterm::queue!(
            ctx.writer,
            crossterm::cursor::MoveTo(area.pos.x, area.pos.y),
        )?;
        match self {
            Self::Stack(subdiv) => subdiv.render(ctx, area),
            Self::Image(image) => image.render(ctx, area),
            Self::Block(block) => block.render(ctx, area),
            Self::Print { raw, .. } => {
                crossterm::queue!(ctx.writer, crossterm::style::Print(raw as &str))
            }
            Self::MinSize { elem, .. } => elem.render(ctx, area),
            Self::Interact(elem) => {
                ctx.layout.insert(area, elem);

                let hovered = if ctx
                    .layout
                    .last_mouse_pos
                    .is_some_and(|it| area.contains(it))
                {
                    if ctx.layout.last_hover_elem.is_some() {
                        log::warn!("Nested interactivity is unsupported");
                        None
                    } else {
                        ctx.layout.last_hover_elem = Some(StoredInteractive::new(elem));
                        elem.hovered.as_ref()
                    }
                } else {
                    None
                };

                hovered.unwrap_or(&elem.normal).render(ctx, area)
            }
        }
    }
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        match self {
            Self::Stack(subdiv) => subdiv.calc_min_size(args),
            Self::Image(image) => image.calc_min_size(args),
            Self::Block(block) => block.calc_min_size(args),
            &Self::Print { width, height, .. } => Vec2 {
                x: width,
                y: height,
            },
            &Self::MinSize {
                width,
                height,
                ref elem,
            } => elem.calc_min_size(args).combine(
                Vec2 {
                    x: width,
                    y: height,
                },
                std::cmp::max,
            ),
            Self::Interact(elem) => elem.normal.calc_min_size(args),
        }
    }
}

impl ImageRepr {
    // Aspect ratio of the image in cells
    fn img_cell_ratio(&self, sizing: &SizingArgs) -> f64 {
        let Vec2 {
            x: font_w,
            y: font_h,
        } = sizing.font_size;

        // Aspect ratio of the image in cells
        std::ops::Mul::mul(
            f64::from(self.dimensions.x) / f64::from(self.dimensions.y),
            f64::from(font_h) / f64::from(font_w),
        )
    }
    fn max_fit_to_fill_axis(size: Vec2<u16>, img_cell_ratio: f64) -> (Axis, u16) {
        let w = size.x;
        let h = size.y;

        // larger aspect ratio means wider.
        // if the aspect ratio of the bounding box is wider than that of the image,
        // it is effectively unconstrained along the horizontal axis. That makes
        // it the flex axis, the other the fill axis.
        match f64::from(w) / f64::from(h) > img_cell_ratio {
            true => (Axis::Y, h),
            false => (Axis::X, w),
        }
    }
    fn fill_axis_to_min_size(
        fill_axis: Axis,
        fill_axis_len: u16,
        img_cell_ratio: f64,
    ) -> Vec2<u16> {
        match fill_axis {
            Axis::Y => Vec2 {
                y: fill_axis_len,
                // cell ratio is width over height, so we get the flex dimension by multiplying
                x: (img_cell_ratio * f64::from(fill_axis_len)).ceil() as _,
            },
            Axis::X => Vec2 {
                x: fill_axis_len,
                // likewise, but by division
                y: (img_cell_ratio / f64::from(fill_axis_len)).ceil() as _,
            },
        }
    }
}
impl Render for ImageRepr {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let img_cell_ratio = self.img_cell_ratio(ctx.sizing);
        let (fill_axis, fill_axis_len) = Self::max_fit_to_fill_axis(area.size, img_cell_ratio);

        crossterm::queue!(
            ctx.writer,
            crossterm::cursor::MoveTo(area.pos.x, area.pos.y),
        )?;

        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
        // - \x1b_G...\x1b\\: kitty graphics apc
        // - a=T: Transfer and display
        // - f=32: 32-bit RGBA
        // - C=1: Do not move the cursor behind the image after drawing. If the image is on the
        //   last line, the first line would move to scrollback (effectively a clear if there is
        //   only one line, like in the bar).
        // - s and v specify the image's dimensions
        write!(
            ctx.writer,
            "\x1b_Ga=T,f=32,C=1,s={},v={},{}={};",
            self.dimensions.x,
            self.dimensions.y,
            match fill_axis {
                Axis::X => "c",
                Axis::Y => "r",
            },
            fill_axis_len,
        )?;
        {
            let mut encoder_writer = base64::write::EncoderWriter::new(
                &mut ctx.writer,
                &base64::engine::general_purpose::STANDARD,
            );
            encoder_writer.write_all(&self.buf)?;
        }
        write!(ctx.writer, "\x1b\\")?;

        Ok(())
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let img_cell_ratio = self.img_cell_ratio(args);
        match self.layout {
            ImageLayoutMode::FillAxis(axis, len) => {
                Self::fill_axis_to_min_size(axis, len, img_cell_ratio)
            }
        }
    }
}
impl Render for StackRepr {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let mut lens = Vec::with_capacity(self.items.len());
        let mut total_weight = 0u64;
        let mut rem_len = Some(area.size[self.axis]);
        for part in self.items.iter() {
            total_weight += u64::from(part.fill_weight);
            let len = part.elem.calc_min_size(ctx.sizing)[self.axis];
            if let Some(rlen) = rem_len {
                rem_len = rlen.checked_sub(len);
            }
            lens.push(len)
        }
        assert_eq!(lens.len(), self.items.len());

        let tot_fill_len = rem_len.unwrap_or_else(|| {
            log::warn!("Stack does not fit into {area:?}: {self:?}");
            0
        });

        if total_weight > 0 {
            let mut rem_fill_len = tot_fill_len;

            for (part, len) in self.items.iter().zip(&mut lens) {
                let extra_len = u16::try_from(
                    u64::from(tot_fill_len) * u64::from(part.fill_weight) / total_weight,
                )
                .expect("bounded by render area");
                *len = len.checked_add(extra_len).unwrap_or_else(|| {
                    log::error!("Element is way too large");
                    *len
                });
                rem_fill_len = rem_fill_len
                    .checked_sub(extra_len)
                    .expect("bounded by partition via floor div");
            }
            if rem_fill_len > 0 {
                let mut fills: Vec<_> = self
                    .items
                    .iter()
                    .zip(&mut lens)
                    .filter_map(|(part, len)| {
                        (part.fill_weight > 0).then_some((part.fill_weight, len))
                    })
                    .collect();
                fills.sort();
                for (_, len) in fills.into_iter().take(rem_fill_len.into()) {
                    *len += 1;
                }
            }
        }

        let mut offset = 0;
        for (part, len) in self.items.iter().zip(lens) {
            let mut subarea = area;
            subarea.size[self.axis] = len;
            subarea.pos[self.axis] += offset;

            part.elem.render(ctx, subarea)?;

            offset += len;
        }

        Ok(())
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let mut tot = Vec2::default();
        for part in self.items.iter() {
            let size = part.elem.calc_min_size(args);

            tot[self.axis] = size[self.axis].saturating_add(tot[self.axis]);

            tot[self.axis.other()] = size[self.axis.other()].max(tot[self.axis.other()]);
        }
        tot
    }
}
impl Render for BlockRepr {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let Self {
            borders,
            border_style,
            border_set:
                BlockLineSet {
                    vertical,
                    horizontal,
                    top_right,
                    top_left,
                    bottom_right,
                    bottom_left,
                },
            inner,
        } = self;

        let BlockBorders {
            top,
            bottom,
            left,
            right,
        } = *borders;

        if let Some(inner) = inner {
            inner.render(
                ctx,
                Area {
                    pos: Vec2 {
                        x: area.pos.x.saturating_add(left.into()),
                        y: area.pos.y.saturating_add(top.into()),
                    },
                    size: Vec2 {
                        x: area.size.x.saturating_sub(right.into()),
                        y: area.size.y.saturating_sub(bottom.into()),
                    },
                },
            )?;
        }

        let mut horiz_border = |l: &str, r: &str, y: u16| {
            let m = border_style.apply(fmt::from_fn(|f| {
                for _ in 0..area
                    .size
                    .x
                    .saturating_sub(left.into())
                    .saturating_sub(right.into())
                {
                    write!(f, "{horizontal}")?;
                }
                Ok(())
            }));

            let l = border_style.apply(if left { l } else { "" });
            let r = border_style.apply(if right { r } else { "" });

            crossterm::queue!(
                ctx.writer,
                crossterm::cursor::MoveTo(area.pos.x, y),
                crossterm::style::Print(format_args!("{l}{m}{r}")),
            )
        };
        if top {
            horiz_border(top_left, top_right, area.pos.y)?;
        }
        if bottom {
            horiz_border(bottom_left, bottom_right, area.y_bottom())?;
        }
        let mut vert_border = |x: u16| -> std::io::Result<()> {
            let lo = area.pos.y.saturating_add(top.into());
            let hi = area
                .pos
                .y
                .saturating_add(area.size.y)
                .saturating_sub(bottom.into());
            for y in lo..hi {
                crossterm::queue!(
                    ctx.writer,
                    crossterm::cursor::MoveTo(x, y),
                    crossterm::style::Print(border_style.apply(vertical as &str)),
                )?;
            }
            Ok(())
        };
        if left {
            vert_border(area.pos.x)?;
        }
        if right {
            vert_border(area.x_right())?;
        }

        Ok(())
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let mut size = self
            .inner
            .as_ref()
            .map(|it| it.calc_min_size(args))
            .unwrap_or_default();
        let Vec2 { x: w, y: h } = self.extra_dim();
        size.x = size.x.saturating_add(w);
        size.y = size.y.saturating_add(h);
        size
    }
}
impl BlockRepr {
    fn extra_dim(&self) -> Vec2<u16> {
        let BlockBorders {
            top,
            bottom,
            left,
            right,
        } = self.borders;
        Vec2 {
            x: u16::from(left) + u16::from(right),
            y: u16::from(top) + u16::from(bottom),
        }
    }
}
impl StyleRepr {
    fn apply(&self, d: impl fmt::Display) -> impl fmt::Display {
        let Self {
            begin: open,
            end: close,
        } = self;
        fmt::from_fn(move |f| write!(f, "{open}{d}{close}"))
    }
}
