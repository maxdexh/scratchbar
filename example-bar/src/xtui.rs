use ctrl::tui;

pub fn tui_center_symbol(sym: impl std::fmt::Display, width: u16) -> tui::Elem {
    tui::Elem::raw_print(
        format_args!("\x1b]66;w={width}:h=2:n=1:d=1;{sym}\x07"),
        tui::Vec2 { x: width, y: 1 },
    )
}
