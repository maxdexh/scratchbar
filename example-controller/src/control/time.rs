use crate::{
    control::{
        BarTuiElem, MenuKind, ModuleArgs, RegisterMenu, interact_callback_with,
        mk_fresh_interact_tag,
    },
    utils::ResultExt as _,
    xtui::{self, text},
};
use anyhow::Context as _;
use chrono::{Datelike as _, Timelike as _};
use scratchbar::tui;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

pub async fn time_module(
    ModuleArgs {
        tui_tx,
        mut reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    let bar_tag = mk_fresh_interact_tag();
    let (cal_today_tx, mut cal_today_rx) = watch::channel(chrono::Local::now().date_naive());

    let _cal_task = {
        let cal_menu_ctrls = CalendarControls {
            reset_now: mk_fresh_interact_tag(),
            next_month: mk_fresh_interact_tag(),
            prev_month: mk_fresh_interact_tag(),
        };
        let (cal_menu_month_tx, mut cal_menu_month_rx) = watch::channel(
            chrono::Local::now()
                .date_naive()
                .with_day(1)
                .expect("Should always be able to set day to 1"),
        );
        let cal_callbacks: [(_, fn(chrono::NaiveDate) -> _); _] = [
            (cal_menu_ctrls.prev_month.clone(), |month| {
                month
                    .checked_sub_months(chrono::Months::new(1))
                    .context("Failed to decrement month")
            }),
            (cal_menu_ctrls.next_month.clone(), |month| {
                month
                    .checked_add_months(chrono::Months::new(1))
                    .context("Failed to increment month")
            }),
            (cal_menu_ctrls.reset_now.clone(), |_| {
                chrono::Local::now()
                    .with_day(1)
                    .map(|it| it.date_naive())
                    .context("Failed to increment month")
            }),
        ];
        for (tag, new_date) in cal_callbacks {
            let cb =
                interact_callback_with(cal_menu_month_tx.clone(), move |month_tx, interact| {
                    if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                        return;
                    }
                    month_tx.send_modify(|month| {
                        if let Some(new) = new_date(*month).ok_or_log() {
                            *month = new
                        }
                    });
                });
            ctrl_tx.register_callback(tag, cb);
        }

        let cal_menu_tx = watch::Sender::new(tui::Elem::empty());
        let cal_tooltip_tx = watch::Sender::new(tui::Elem::empty());
        ctrl_tx.register_menu(RegisterMenu {
            on_tag: bar_tag.clone(),
            on_kind: tui::InteractKind::Hover,
            tui_rx: cal_tooltip_tx.subscribe(),
            menu_kind: MenuKind::Tooltip,
            opts: Default::default(),
        });
        ctrl_tx.register_menu(RegisterMenu {
            on_tag: bar_tag.clone(),
            on_kind: tui::InteractKind::Click(tui::MouseButton::Right),
            tui_rx: cal_menu_tx.subscribe(),
            menu_kind: MenuKind::Context,
            opts: Default::default(),
        });

        AbortOnDropHandle::new(tokio::spawn(async move {
            cal_today_rx.mark_changed();
            cal_menu_month_rx.mark_changed();
            loop {
                tokio::select! {
                    Ok(()) = cal_menu_month_rx.changed() => {}
                    Ok(()) = cal_today_rx.changed() => {}
                    else => break,
                }
                let (today, today_has_changed) = {
                    let it = cal_today_rx.borrow_and_update();
                    (*it, it.has_changed())
                };
                let menu_month = *cal_menu_month_rx.borrow_and_update();

                if today_has_changed && let Some(tui) = mk_calendar(today, today, None) {
                    cal_tooltip_tx.send_replace(tui);
                }
                if let Some(tui) = mk_calendar(menu_month, today, Some(&cal_menu_ctrls)) {
                    cal_menu_tx.send_replace(tui);
                }
            }
        }))
    };

    let (clock_time_tx, mut clock_time_rx) = watch::channel(chrono::Local::now());
    let _bar_task = AbortOnDropHandle::new(tokio::spawn(async move {
        clock_time_rx.mark_changed();
        while let Ok(()) = clock_time_rx.changed().await {
            let now = *clock_time_rx.borrow_and_update();
            let tui = text::TextOpts::default()
                .render_line(&now.format("%H:%M %d/%m").to_string())
                .interactive(bar_tag.clone());

            tui_tx.send_replace(BarTuiElem::Shared(tui));
        }
    }));

    loop {
        let now = chrono::Local::now();
        clock_time_tx.send_if_modified(|old| std::mem::replace(old, now).minute() != now.minute());

        let today = now.date_naive();
        cal_today_tx.send_if_modified(|old| std::mem::replace(old, today) != today);

        let seconds_until_minute = 60 - u64::from(now.second());
        let timeout_ms = std::cmp::max(750 * seconds_until_minute, 100);

        tokio::select! {
            Some(()) = reload_rx.wait() => {}
            () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {}
        }
    }
}

struct CalendarControls {
    reset_now: tui::CustomId,
    next_month: tui::CustomId,
    prev_month: tui::CustomId,
}

// TODO: Context menu
const WEEK_DAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
fn mk_calendar(
    month: chrono::NaiveDate,
    today: chrono::NaiveDate,
    controls: Option<&CalendarControls>,
) -> Option<tui::Elem> {
    use chrono::Datelike as _;

    let first_weekday_offset = month
        .with_day(1)
        .with_context(|| format!("Failed to set day 1 of month {month}"))
        .ok_or_log()?
        .weekday() as u16;

    let mut tui_ystack = xtui::StackBuilder::new(tui::Axis::Y);

    tui_ystack.push(match controls {
        Some(CalendarControls {
            reset_now,
            next_month,
            prev_month,
        }) => {
            let normal = text::TextOpts::default();
            let hovered = normal.clone().with(|it| it.attrs.set_underlined(true));

            // TODO: Large title scale
            let mut title_stack = xtui::StackBuilder::new(tui::Axis::X);
            title_stack.push(text::render_with_hover(
                &normal,
                prev_month.clone(),
                &hovered,
                |it| it.render_line("<<"),
            ));
            title_stack.spacing(1);
            title_stack.push(text::render_with_hover(
                &normal,
                reset_now.clone(),
                &hovered,
                |it| it.render_line(&month.format("%B %Y").to_string()),
            ));
            title_stack.fill(1, tui::Elem::empty());
            title_stack.spacing(1);
            title_stack.push(text::render_with_hover(
                &normal,
                next_month.clone(),
                &hovered,
                |it| it.render_line(">>"),
            ));
            title_stack.build()
        }
        None => text::TextOpts::default()
            .with(|it| it.attrs.set_bold(true))
            .render_line(&month.format("%B %Y").to_string()),
    });
    tui_ystack.push({
        let mut xstack = xtui::StackBuilder::new(tui::Axis::X);
        for day in WEEK_DAYS {
            xstack.push(text::TextOpts::default().render_line(day));
            xstack.spacing(1);
        }
        xstack.delete_last();
        xstack.build()
    });

    let mut week_xstack = xtui::StackBuilder::new(tui::Axis::X);
    week_xstack.spacing(3 * first_weekday_offset);
    for d0 in 0u16..month.num_days_in_month().into() {
        let d1 = d0.checked_add(1).context("Overflow").ok_or_log()?;
        let day = month
            .with_day(d1.into())
            .with_context(|| format!("Failed to set day {d1} in month {month}"))
            .ok_or_log()?;

        let text = format!("{d1:>2}");

        week_xstack.push(
            text::TextOpts::default()
                .with(|it| {
                    if day == today {
                        it.fg_color = text::Color::Green
                    }
                })
                .render_line(&text),
        );

        if day.weekday() == chrono::Weekday::Sun {
            tui_ystack.push(week_xstack.build());
            week_xstack = xtui::StackBuilder::new(tui::Axis::X);
        } else {
            week_xstack.spacing(1);
        }
    }
    if !week_xstack.is_empty() {
        tui_ystack.push(week_xstack.build());
    }

    Some(tui_ystack.build())
}
