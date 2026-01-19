use std::io::Write;

use crate::tui::*;

pub(super) trait Render {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()>;
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16>;
}

#[derive(Debug)]
pub struct RenderCtx<'a, W> {
    pub sizing: &'a SizingArgs,
    pub writer: W,
    pub layout: &'a mut RenderedLayout,
}
#[derive(Debug)]
pub struct SizingArgs {
    pub font_size: Vec2<u16>,
}

impl Tui {
    pub fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        self.root.calc_min_size(args)
    }
    pub fn render2(
        &self,
        cell_size: Vec2<u16>,
        writer: &mut impl Write,
        sizing: &SizingArgs,
    ) -> std::io::Result<RenderedLayout> {
        crossterm::queue!(
            writer,
            crossterm::terminal::BeginSynchronizedUpdate,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        )?;
        let mut layout = RenderedLayout::default();
        self.root.render2(
            &mut RenderCtx {
                sizing,
                writer: &mut *writer,
                layout: &mut layout,
            },
            Area {
                pos: Default::default(),
                size: cell_size,
            },
        )?;
        crossterm::execute!(writer, crossterm::terminal::EndSynchronizedUpdate)?;
        Ok(layout)
    }
}
impl Render for Elem {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        match self {
            Self::Stack(subdiv) => subdiv.render2(ctx, area),
            Self::Image(image) => image.render2(ctx, area),
            Self::Block(block) => block.render2(ctx, area),
            Self::Text(text) => text.render2(ctx, area),
            Self::Interact(elem) => {
                ctx.layout.insert(area, elem.payload.clone());
                elem.elem.render2(ctx, area)
            }
            Self::Shared(elem) => elem.render2(ctx, area),
            Self::Empty => Ok(()),
        }
    }
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        match self {
            Self::Stack(subdiv) => subdiv.calc_min_size(args),
            Self::Text(text) => text.calc_min_size(args),
            Self::Image(image) => image.calc_min_size(args),
            Self::Block(block) => block.calc_min_size(args),
            Self::Interact(elem) => elem.elem.calc_min_size(args),
            Self::Shared(elem) => elem.calc_min_size(args),
            Self::Empty => Vec2::default(),
        }
    }
}

impl Image {
    // Aspect ratio of the image in cells
    fn img_cell_ratio(&self, sizing: &SizingArgs) -> f64 {
        let Vec2 {
            x: font_w,
            y: font_h,
        } = sizing.font_size;

        // Aspect ratio of the image in cells
        std::ops::Mul::mul(
            f64::from(self.img.width()) / f64::from(self.img.height()),
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
impl Render for Image {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
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
            self.img.width(),
            self.img.height(),
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
            encoder_writer.write_all(self.img.as_raw())?;
        }
        write!(ctx.writer, "\x1b\\")?;

        Ok(())
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let img_cell_ratio = self.img_cell_ratio(args);
        match self.sizing {
            ImageSizeMode::FillAxis(axis, len) => {
                Self::fill_axis_to_min_size(axis, len, img_cell_ratio)
            }
        }
    }
}
impl Render for Stack {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let mut lens = Vec::with_capacity(self.parts.len());
        let mut total_weight = 0u64;
        let mut rem_len = Some(area.size[self.axis]);
        for part in &self.parts {
            if let Constr::Fill(weight) = part.constr {
                total_weight += u64::from(weight);
            }
            let len = Self::calc_min_part_size(part, self.axis, ctx.sizing)[self.axis];
            if let Some(rlen) = rem_len {
                rem_len = rlen.checked_sub(len);
            }
            lens.push(len)
        }
        assert_eq!(lens.len(), self.parts.len());

        let tot_fill_len = rem_len.unwrap_or_else(|| {
            log::warn!("Stack does not fit into {area:?}: {self:?}");
            0
        });

        if total_weight > 0 {
            let mut rem_fill_len = tot_fill_len;

            for (part, len) in self.parts.iter().zip(&mut lens) {
                if let Constr::Fill(weight) = part.constr {
                    let extra_len =
                        u16::try_from(u64::from(tot_fill_len) * u64::from(weight) / total_weight)
                            .expect("bounded by render area");
                    *len = len.checked_add(extra_len).unwrap_or_else(|| {
                        log::error!("Element is way too large");
                        *len
                    });
                    rem_fill_len = rem_fill_len
                        .checked_sub(extra_len)
                        .expect("bounded by partition via floor div");
                }
            }
            if rem_fill_len > 0 {
                let mut fills: Vec<_> = self
                    .parts
                    .iter()
                    .zip(&mut lens)
                    .filter_map(|(part, len)| match part.constr {
                        Constr::Fill(weight) if weight > 0 => Some((weight, len)),
                        _ => None,
                    })
                    .collect();
                fills.sort();
                for (_, len) in fills.into_iter().take(rem_fill_len.into()) {
                    *len += 1;
                }
            }
        }

        let mut offset = 0;
        for (part, len) in self.parts.iter().zip(lens) {
            let mut subarea = area;
            subarea.size[self.axis] = len;
            subarea.pos[self.axis] += offset;

            part.elem.render2(ctx, subarea)?;

            offset += len;
        }

        Ok(())
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let mut tot = Vec2::default();
        for part in &self.parts {
            let size = Self::calc_min_part_size(part, self.axis, args);

            tot[self.axis] = size[self.axis].saturating_add(tot[self.axis]);

            tot[self.axis.other()] = size[self.axis.other()].max(tot[self.axis.other()]);
        }
        tot
    }
}
impl Stack {
    fn calc_min_part_size(part: &StackItem, axis: Axis, args: &SizingArgs) -> Vec2<u16> {
        let mut size = part.elem.calc_min_size(args);
        match part.constr {
            Constr::Length(l) => size[axis] = size[axis].max(l),
            Constr::Fill(_) | Constr::Auto => (),
        }
        size
    }
}
impl Render for Text {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let mut y_off = 0;
        // TODO: Style
        for line in &self.lines {
            let Some(y) = area.pos.y.checked_add(y_off) else {
                log::error!("Vertical position overflow");
                break;
            };
            crossterm::queue!(
                ctx.writer,
                crossterm::cursor::MoveTo(area.pos.x, y),
                crossterm::style::Print(stylize(line.text.as_str(), self.style)),
            )?;
            let Some(new_y_off) = y_off.checked_add(line.height) else {
                log::error!("Vertical position overflow");
                break;
            };
            y_off = new_y_off;
        }

        Ok(())
    }

    fn calc_min_size(&self, _: &SizingArgs) -> Vec2<u16> {
        Vec2 {
            x: self.width,
            y: self.lines.iter().map(|line| line.height).sum(),
        }
    }
}
impl Render for Block {
    fn render2(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        let Borders {
            top,
            bottom,
            left,
            right,
        } = self.borders;

        if let Some(inner) = &self.inner {
            let min_size = self.calc_min_size(ctx.sizing);
            log::debug!("{min_size:?} {area:?}");
            //if min_size.x > area.size.x || min_size.y > area.size.y {
            //    log::error!("Not rendering borders of block elem because area is too small");
            //    return inner.render2(ctx, area);
            //}

            inner.render2(
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
            let m = stylize(
                self.border_set.horizontal.repeat(
                    area.size
                        .x
                        .saturating_sub(left.into())
                        .saturating_sub(right.into())
                        .into(),
                ),
                self.border_style,
            );
            let l = stylize(if left { l } else { "" }, self.border_style);
            let r = stylize(if right { r } else { "" }, self.border_style);

            crossterm::queue!(
                ctx.writer,
                crossterm::cursor::MoveTo(area.pos.x, y),
                crossterm::style::Print(format_args!("{l}{m}{r}")),
            )
        };
        if top {
            horiz_border(
                &self.border_set.top_left,
                &self.border_set.top_right,
                area.pos.y,
            )?;
        }
        if bottom {
            horiz_border(
                &self.border_set.bottom_left,
                &self.border_set.bottom_right,
                area.y_bottom(),
            )?;
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
                    crossterm::style::Print(stylize(
                        &self.border_set.vertical as &str,
                        self.border_style
                    )),
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
impl Block {
    fn extra_dim(&self) -> Vec2<u16> {
        let Borders {
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
fn stylize<S>(
    s: S,
    Style {
        fg,
        bg,
        modifier:
            Modifier {
                bold,
                dim,
                italic,
                underline,
                hidden,
                strike,
            },
        underline_color,
    }: Style,
) -> S::Styled
where
    S: crossterm::style::Stylize<
            Styled: crossterm::style::Stylize<Styled = S::Styled> + std::fmt::Display,
        >,
{
    use crossterm::style::Stylize;

    let mut s = s.stylize();
    if bold {
        s = s.bold();
    }
    if dim {
        s = s.dim();
    }
    if italic {
        s = s.italic();
    }
    if underline {
        s = s.underlined();
    }
    if hidden {
        s = s.hidden();
    }
    if strike {
        s = s.crossed_out();
    }
    if let Some(fg) = fg {
        s = s.with(fg);
    }
    if let Some(bg) = bg {
        s = s.on(bg);
    }
    if let Some(col) = underline_color {
        s = s.underline(col);
    }

    s
}
