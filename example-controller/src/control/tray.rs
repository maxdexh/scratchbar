use std::sync::Arc;

use crate::{
    clients,
    control::{
        BarTuiElem, InteractArgs, InteractTagRegistry, MenuKind, ModuleArgs, RegisterMenu,
        mk_fresh_interact_tag,
    },
    utils::ResultExt as _,
    xtui,
};
use anyhow::Context as _;
use scratchbar::tui;
use tokio::sync::watch;

pub async fn tray_module(
    ModuleArgs {
        tui_tx,
        reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    use crate::clients::tray::*;
    let tray = Arc::new(clients::tray::connect(reload_rx));

    let mut entry_reg = InteractTagRegistry::new();

    let mut state_rx = tray.state_rx.clone();
    while state_rx.changed().await.is_ok() {
        let state = state_rx.borrow_and_update().clone();

        let mut tui_stack = xtui::StackBuilder::new(tui::Axis::X);
        for TrayEntry { addr, item, menu } in state.entries.iter() {
            let (tag, ()) = entry_reg.get_or_init(addr, |_| ());

            // FIXME: Handle icon attrs
            if let Some(system_tray::item::Tooltip {
                icon_name: _,
                icon_data: _,
                title,
                description,
            }) = item.tool_tip.as_ref()
            {
                let menu_tui = {
                    let mut menu_tui_stack = xtui::StackBuilder::new(tui::Axis::Y);
                    menu_tui_stack.push({
                        let mut hstack = xtui::StackBuilder::new(tui::Axis::X);
                        hstack.fill(1, tui::Elem::empty());
                        hstack.push(tui::Elem::text(
                            title,
                            tui::TextModifiers {
                                bold: true,
                                ..Default::default()
                            },
                        ));
                        hstack.fill(1, tui::Elem::empty());
                        hstack.build()
                    });
                    menu_tui_stack.push(tui::Elem::text(description, tui::TextOpts::default()));
                    menu_tui_stack.build()
                };
                ctrl_tx.set_menu(RegisterMenu {
                    on_tag: tag.clone(),
                    on_kind: tui::InteractKind::Hover,
                    menu_kind: MenuKind::Tooltip,
                    tui_rx: watch::channel(menu_tui).1,
                    opts: Default::default(),
                });
            }

            if let Some(TrayMenuExt {
                menu_path,
                submenus,
                ..
            }) = menu.as_ref()
            {
                let menu_tui = tray_menu_to_tui(0, submenus, &|id| {
                    let tag = mk_fresh_interact_tag();
                    let Some(menu_path) = menu_path.clone() else {
                        return tag;
                    };

                    let tray = tray.clone();
                    let addr = addr.clone();
                    let icb = Arc::new(move |interact: InteractArgs| {
                        if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                            return;
                        }
                        let addr = addr.clone();
                        let menu_path = menu_path.clone();
                        tray.sched_with_client(async move |client| {
                            client
                                .activate(system_tray::client::ActivateRequest::MenuItem {
                                    address: str::to_owned(&addr),
                                    menu_path: str::to_owned(&menu_path),
                                    submenu_id: id,
                                })
                                .await
                                .context("Failed to send ActivateRequest")
                                .ok_or_log();
                        });
                    });
                    ctrl_tx.register_callback(tag.clone(), Some(icb));
                    tag
                });

                let tui = tui::Elem::block(tui::BlockOpts {
                    border_style: Some(tui::TextStyle {
                        fg: Some(tui::TermColor::DarkGrey),
                        ..Default::default()
                    }),
                    borders: tui::BlockBorders::all(),
                    lines: tui::BlockLineSet::thick(),
                    inner: Some(menu_tui),
                    ..Default::default()
                });
                ctrl_tx.set_menu(RegisterMenu {
                    on_tag: tag.clone(),
                    on_kind: tui::InteractKind::Click(tui::MouseButton::Right),
                    menu_kind: MenuKind::Context,
                    tui_rx: watch::channel(tui).1,
                    opts: Default::default(),
                });
            }

            // FIXME: Handle the other options
            // FIXME: Why are we showing all icons?
            for system_tray::item::IconPixmap {
                width,
                height,
                pixels,
            } in item.icon_pixmap.as_deref().unwrap_or(&[])
            {
                let mut img = match image::RgbaImage::from_vec(
                    width.cast_unsigned(),
                    height.cast_unsigned(),
                    pixels.clone(),
                ) {
                    Some(img) => img,
                    None => {
                        log::error!("Failed to load image from bytes");
                        continue;
                    }
                };

                // https://users.rust-lang.org/t/argb32-color-model/92061/4
                for image::Rgba(pixel) in img.pixels_mut() {
                    *pixel = u32::from_be_bytes(*pixel).rotate_left(8).to_be_bytes();
                }

                tui_stack.push(
                    xtui::rgba_image(
                        img, //
                        tui::ImageLayoutMode::FillAxis(tui::Axis::Y, 1),
                    )
                    .interactive(tag.clone()),
                );
                tui_stack.spacing(1);
            }
        }
        tui_tx.send_replace(BarTuiElem::Shared(tui_stack.build()));
    }
    fn tray_menu_item_to_tui(
        depth: u16,
        item: &system_tray::menu::MenuItem,
        mk_interact: &impl Fn(i32) -> tui::CustomId,
    ) -> Option<tui::Elem> {
        use system_tray::menu::*;
        let main_elem = match item {
            MenuItem { visible: false, .. } => return None,
            MenuItem {
                visible: true,
                menu_type: MenuType::Separator,
                ..
            } => tui::Elem::block(tui::BlockOpts {
                borders: tui::BlockBorders {
                    top: true,
                    ..Default::default()
                },
                border_style: Some(tui::TextStyle {
                    fg: Some(tui::TermColor::DarkGrey),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            MenuItem {
                id,
                menu_type: MenuType::Standard,
                label: Some(label),
                enabled: _,
                visible: true,
                icon_name: _,
                icon_data,
                shortcut: _,
                toggle_type: _, // TODO: implement toggle
                toggle_state: _,
                children_display: _,
                disposition: _, // TODO: what to do with this?
                submenu: _,
            } => {
                let mut stack = xtui::StackBuilder::new(tui::Axis::X);
                stack.spacing(depth + 1);
                if let Some(icon) = icon_data
                    && let Some(img) =
                        image::load_from_memory_with_format(icon, image::ImageFormat::Png)
                            .context("Systray icon has invalid png data")
                            .ok_or_log()
                {
                    stack.push(xtui::rgba_image(
                        img.into_rgba8(),
                        tui::ImageLayoutMode::FillAxis(tui::Axis::Y, 1),
                    ));
                    stack.spacing(1);
                }
                stack.push(tui::Elem::text(label, tui::TextOpts::default()));
                // FIXME: Add hover
                stack.build().interactive(mk_interact(*id))
            }

            _ => {
                log::error!("Unhandled menu item: {item:#?}");
                return None;
            }
        };

        Some(if item.submenu.is_empty() {
            main_elem
        } else {
            let mut stack = xtui::StackBuilder::new(tui::Axis::Y);
            stack.push(main_elem);
            stack.push(tray_menu_to_tui(depth + 1, &item.submenu, mk_interact));
            stack.build()
        })
    }

    fn tray_menu_to_tui(
        depth: u16,
        items: &[system_tray::menu::MenuItem],
        mk_interact: &impl Fn(i32) -> tui::CustomId,
    ) -> tui::Elem {
        let mut stack = xtui::StackBuilder::new(tui::Axis::Y);
        for item in items {
            if let Some(item) = tray_menu_item_to_tui(depth, item, mk_interact) {
                stack.push(item)
            }
        }
        stack.build()
    }
}
