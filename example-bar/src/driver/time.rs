use crate::{
    driver::{BarTuiElem, ModuleArgs, mk_fresh_interact_tag},
    utils::ResultExt as _,
    xtui,
};
use anyhow::Context as _;
use ctrl::{api, tui};
use tokio_util::task::AbortOnDropHandle;

pub async fn time_module(
    ModuleArgs {
        tui_tx,
        mut reload_rx,
        ctrl_tx,
        ..
    }: ModuleArgs,
) {
    use chrono::{Datelike, Timelike};
    use std::time::Duration;
    const WEEK_DAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    fn make_calendar(month: chrono::NaiveDate, today: chrono::NaiveDate) -> Option<tui::Elem> {
        let title = month.format("%B %Y").to_string();

        let first_weekday_offset = month
            .with_day(1)
            .with_context(|| format!("Failed to set day 1 of month {month}"))
            .ok_or_log()?
            .weekday() as u16;

        let mut tui_ystack = xtui::StackBuilder::new(tui::Axis::Y);
        tui_ystack.push(tui::Elem::text(
            title,
            tui::TextModifiers {
                bold: true,
                ..Default::default()
            },
        ));
        tui_ystack.push({
            let mut xstack = xtui::StackBuilder::new(tui::Axis::X);
            for day in WEEK_DAYS {
                xstack.push(tui::Elem::text(day, tui::TextOpts::default()));
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
            week_xstack.push(tui::Elem::text(
                text,
                tui::TextStyle {
                    fg: (day == today).then_some(tui::TermColor::Green),
                    ..Default::default()
                },
            ));

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

    let interact_tag = mk_fresh_interact_tag();

    let interact_tag_clone = interact_tag.clone();
    let _menu_task = AbortOnDropHandle::new({
        tokio::spawn(async move {
            let now = chrono::Local::now().date_naive();
            if let Some(tui) = make_calendar(now, now) {
                ctrl_tx.set_menu(api::RegisterMenu {
                    on_tag: interact_tag_clone.clone(),
                    on_kind: tui::InteractKind::Hover,
                    tui,
                    menu_kind: api::MenuKind::Tooltip,
                    options: Default::default(),
                });
            }
            // FIXME: Schedule regular update
        })
    });

    let mut prev_minutes = 61;
    loop {
        let now = chrono::Local::now();
        let minute = now.minute();
        if prev_minutes != minute {
            let tui = tui::Elem::text(now.format("%H:%M %d/%m"), tui::TextOpts::default())
                .interactive(interact_tag.clone());

            tui_tx.send_replace(BarTuiElem::Shared(tui));

            prev_minutes = minute;
        } else {
            let seconds_until_minute = 60 - u64::from(now.second());
            let timeout_ms = std::cmp::max(750 * seconds_until_minute, 100);

            tokio::select! {
                Some(()) = reload_rx.wait() => {}
                () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {}
            }
        }
    }
}
