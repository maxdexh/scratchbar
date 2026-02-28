use std::io::Write;

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
impl<T> Vec2<T> {
    fn combine<U, R>(self, other: Vec2<U>, mut f: impl FnMut(T, U) -> R) -> Vec2<R> {
        Vec2 {
            x: f(self.x, other.x),
            y: f(self.y, other.y),
        }
    }
}
pub(crate) fn calc_min_size(elem: &Elem, args: &SizingArgs) -> Vec2<u16> {
    elem.calc_min_size(args)
        .combine(Vec2 { x: 1, y: 1 }, std::cmp::max)
}
pub(crate) fn render(
    elem: &Elem,
    area: Area,
    writer: &mut impl Write,
    sizing: &SizingArgs,
    old_layout: &RenderedLayout,
) -> std::io::Result<RenderedLayout> {
    crossterm::queue!(
        writer,
        crossterm::terminal::BeginSynchronizedUpdate,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
    )?;
    let mut layout = RenderedLayout {
        widgets: Default::default(),
        last_mouse_pos: old_layout.last_mouse_pos,
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
            Self::Stack(repr) => repr.render(ctx, area),
            Self::Print(PrintRepr { raw }) => {
                if raw.starts_with(b"\x1b_") {
                    log::debug!("{area:?}");
                }
                ctx.writer.write_all(raw)
            }
            Self::MinSize(MinSizeRepr { elem, .. }) => elem.render(ctx, area),
            Self::Interact(repr) => {
                ctx.layout.insert(area, repr);

                let hovered = if ctx
                    .layout
                    .last_mouse_pos
                    .is_some_and(|it| area.contains(it))
                {
                    if ctx.layout.last_hover_elem.is_some() {
                        log::warn!("Nested interactivity is unsupported");
                        None
                    } else {
                        ctx.layout.last_hover_elem = Some(StoredInteractive::new(repr));
                        repr.hovered.as_ref()
                    }
                } else {
                    None
                };

                hovered.unwrap_or(&repr.normal).render(ctx, area)
            }
            Self::Fill(FillRepr { symbol }) => {
                log::debug!("{symbol:?}, {area:?}");
                for y_off in 0..area.size.y {
                    crossterm::queue!(
                        ctx.writer,
                        crossterm::cursor::MoveTo(area.pos.x, area.pos.y.saturating_add(y_off))
                    )?;
                    for _ in 0..area.size.x {
                        ctx.writer.write_all(symbol.as_bytes())?;
                    }
                }
                Ok(())
            }
            Self::MinAxis(repr) => repr.render(ctx, area),
        }
    }
    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        match self {
            Self::Stack(subdiv) => subdiv.calc_min_size(args),
            Self::Print(..) => Vec2::default(),
            Self::MinSize(MinSizeRepr { elem, size }) => {
                elem.calc_min_size(args).combine(*size, std::cmp::max)
            }
            Self::Interact(repr) => repr.normal.calc_min_size(args),
            Self::Fill(_) => Vec2::default(),
            Self::MinAxis(repr) => repr.calc_min_size(args),
        }
    }
}

impl Render for MinAxisRepr {
    fn render(&self, ctx: &mut RenderCtx<impl Write>, area: Area) -> std::io::Result<()> {
        self.elem.render(ctx, area)
    }

    fn calc_min_size(&self, args: &SizingArgs) -> Vec2<u16> {
        let &Self {
            ref elem,
            axis,
            len,
            aspect,
        } = self;

        fn widen_mul32(a: u32, b: u32) -> u64 {
            u64::from(a)
                .checked_mul(u64::from(b))
                .expect("u32 multiplication cannot overflow a u64")
        }

        let mut size = Vec2::default();
        size[axis] = len;

        // Find the length of the fill axis in pixels
        let pixel_axis_len = u32::from(len)
            .checked_mul(u32::from(args.font_size[axis]))
            .expect("u16 multiplication cannot overflow a u32");

        // Invert the aspect ratio to find the pixel length of the other axis, then find the cell length.
        //
        // We assume that the aspect ratio is exact.
        // Hence, part of the next cell is used when the remainder is nonzero.
        // Thus we round up. Doing the divisions in one step avoids inaccuracy.
        size[axis.flip()] = widen_mul32(pixel_axis_len, aspect[axis.flip()])
            .div_ceil(widen_mul32(
                aspect[axis],
                args.font_size[axis.flip()].into(),
            ))
            .try_into()
            .unwrap_or(u16::MAX);

        log::debug!("{size:?}");

        size.combine(elem.calc_min_size(args), std::cmp::max)
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

            tot[self.axis.flip()] = size[self.axis.flip()].max(tot[self.axis.flip()]);
        }
        tot
    }
}
