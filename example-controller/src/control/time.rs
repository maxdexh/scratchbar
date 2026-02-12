use crate::{
    control::{BarTuiElem, ModuleArgs, interact_callback_with, mk_fresh_interact_tag},
    utils::ResultExt as _,
    xtui,
};
use anyhow::Context as _;
use chrono::{Datelike as _, Timelike as _};
use scratchbar::{host, tui};
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

pub async fn time_module(
    ModuleArgs {
        tui_tx,
        mut reload_rx,
        ctrl_tx,
        tag_callback_tx,
        ..
    }: ModuleArgs,
) {
    let bar_tag = mk_fresh_interact_tag();
    let ctx_menu_ctrls = CalendarControls {
        reset_now: mk_fresh_interact_tag(),
        next_month: mk_fresh_interact_tag(),
        prev_month: mk_fresh_interact_tag(),
    };
    let (ctx_menu_month_tx, mut ctx_menu_month_rx) =
        watch::channel(chrono::Local::now().date_naive().with_day(1).unwrap());

    let callbacks: [(_, fn(chrono::NaiveDate) -> _); _] = [
        (ctx_menu_ctrls.prev_month.clone(), |month| {
            month
                .checked_sub_months(chrono::Months::new(1))
                .context("Failed to decrement month")
        }),
        (ctx_menu_ctrls.next_month.clone(), |month| {
            month
                .checked_add_months(chrono::Months::new(1))
                .context("Failed to increment month")
        }),
        (ctx_menu_ctrls.reset_now.clone(), |_| {
            chrono::Local::now()
                .with_day(1)
                .map(|it| it.date_naive())
                .context("Failed to increment month")
        }),
    ];
    for (tag, cb) in callbacks {
        let cb = interact_callback_with(ctx_menu_month_tx.clone(), move |month_tx, interact| {
            if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                return;
            }
            month_tx.send_modify(|month| {
                if let Some(new) = cb(*month).ok_or_log() {
                    *month = new
                }
            });
        });
        tag_callback_tx.send((tag, Some(cb))).ok_or_log();
    }

    tag_callback_tx
        .send((
            ctx_menu_ctrls.prev_month.clone(),
            Some(interact_callback_with(
                ctx_menu_month_tx.clone(),
                |month_tx, interact| {
                    if interact.kind != tui::InteractKind::Click(tui::MouseButton::Left) {
                        return;
                    }
                    month_tx.send_modify(|month| {
                        if let Some(new) = month
                            .checked_sub_months(chrono::Months::new(1))
                            .context("Failed to decrement month")
                            .ok_or_log()
                        {
                            *month = new
                        }
                    });
                },
            )),
        ))
        .ok_or_log();

    let _ctx_menu_task = {
        let ctrl_tx = ctrl_tx.clone();
        let bar_tag = bar_tag.clone();
        AbortOnDropHandle::new(tokio::spawn(async move {
            while let Ok(()) = ctx_menu_month_rx.changed().await {
                let now = chrono::Local::now().date_naive();
                let month = *ctx_menu_month_rx.borrow_and_update();
                if let Some(tui) = mk_calendar(month, now, Some(&ctx_menu_ctrls)) {
                    ctrl_tx.set_menu(host::RegisterMenu {
                        on_tag: bar_tag.clone(),
                        on_kind: tui::InteractKind::Click(tui::MouseButton::Right),
                        tui,
                        menu_kind: host::MenuKind::Context,
                        options: Default::default(),
                    });
                }
            }
        }))
    };

    let mut prev = chrono::DateTime::<chrono::Local>::default();
    loop {
        let now = chrono::Local::now();
        if prev.day() != now.day() {
            if let Some(tui) = mk_calendar(now.date_naive(), now.date_naive(), None) {
                ctrl_tx.set_menu(host::RegisterMenu {
                    on_tag: bar_tag.clone(),
                    on_kind: tui::InteractKind::Hover,
                    tui,
                    menu_kind: host::MenuKind::Tooltip,
                    options: Default::default(),
                });
            }
            ctx_menu_month_tx.send_modify(|_| {});
        }

        if prev.minute() != now.minute() {
            let tui = tui::Elem::text(now.format("%H:%M %d/%m"), tui::TextOpts::default())
                .interactive(bar_tag.clone());

            tui_tx.send_replace(BarTuiElem::Shared(tui));

            prev = now;
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

struct CalendarControls {
    // TODO: Reset on menu close
    reset_now: tui::InteractTag,
    next_month: tui::InteractTag,
    prev_month: tui::InteractTag,
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
            let mut title_stack = xtui::StackBuilder::new(tui::Axis::X);
            title_stack.push(xtui::underline_hovered(
                "<<",
                tui::TextOpts::default(),
                prev_month.clone(),
            ));
            title_stack.spacing(1);
            title_stack.push(xtui::underline_hovered(
                month.format("%B %Y"),
                tui::TextModifiers {
                    bold: true,
                    ..Default::default()
                },
                reset_now.clone(),
            ));
            title_stack.fill(1, tui::Elem::empty());
            title_stack.spacing(1);
            title_stack.push(xtui::underline_hovered(
                ">>",
                tui::TextOpts::default(),
                next_month.clone(),
            ));
            title_stack.build()
        }
        None => tui::Elem::text(
            month.format("%B %Y"),
            tui::TextModifiers {
                bold: true,
                ..Default::default()
            },
        ),
    });
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
