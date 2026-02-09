use ctrl::tui;

pub fn tui_center_symbol(sym: impl std::fmt::Display, width: u16) -> tui::Elem {
    tui::Elem::raw_print(
        format_args!("\x1b]66;w={width}:h=2:n=1:d=1;{sym}\x07"),
        tui::Vec2 { x: width, y: 1 },
    )
}

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
