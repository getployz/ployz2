//! tabled + cliclack + anstream. Compact is the same tables with a pipe style.

use std::io::{self, IsTerminal};

use anstyle::{AnsiColor, Color, Style};
use tabled::{
    builder::Builder,
    settings::{Alignment, Padding, Style as TableStyle, object::Columns},
};

use crate::error::SpikeError;
use crate::view::{ImageRow, PlanView, ServiceRow, op_mark, progress_at, progress_done};

fn heading(text: &str) {
    let style = Style::new()
        .bold()
        .fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
    anstream::println!("{style}{text}{style:#}");
}

fn render_table(
    headers: &[&str],
    rows: impl IntoIterator<Item = Vec<String>>,
    compact: bool,
) -> String {
    let mut builder = Builder::default();
    builder.push_record(headers.iter().copied());
    for row in rows {
        builder.push_record(row);
    }
    let mut table = builder.build();
    if compact {
        table.with(TableStyle::empty());
        table.modify(Columns::first(), Alignment::left());
    } else {
        table.with(TableStyle::rounded());
        table.with(Padding::new(1, 1, 0, 0));
    }
    table.to_string()
}

#[must_use]
pub fn services_table(rows: &[ServiceRow], compact: bool) -> String {
    render_table(
        &["SERVICE ID", "SERVICE", "CONTAINERS", "HOOKS"],
        rows.iter().map(|row| {
            vec![
                row.service_id.to_string(),
                row.service.to_string(),
                row.containers.to_string(),
                row.hooks.to_string(),
            ]
        }),
        compact,
    )
}

#[must_use]
pub fn images_table(rows: &[ImageRow], compact: bool) -> String {
    render_table(
        &[
            "MACHINE",
            "ID",
            "REPOSITORY:TAG",
            "CREATED",
            "SIZE",
            "PLATFORMS",
        ],
        rows.iter().map(|row| {
            vec![
                row.machine.to_string(),
                row.id.to_string(),
                row.repository.to_string(),
                row.created.to_string(),
                row.size.to_string(),
                row.platforms.to_string(),
            ]
        }),
        compact,
    )
}

pub fn services(rows: &[ServiceRow], compact: bool) -> Result<(), SpikeError> {
    heading("ls");
    println!("{}", services_table(rows, compact));
    Ok(())
}

pub fn images(rows: &[ImageRow], compact: bool) -> Result<(), SpikeError> {
    heading("images");
    println!("{}", images_table(rows, compact));
    Ok(())
}

pub fn plan(plan: &PlanView, compact: bool) -> Result<(), SpikeError> {
    if !compact && io::stdout().is_terminal() {
        cliclack::intro("ployz deploy").map_err(SpikeError::message)?;
        cliclack::note(
            "plan",
            format!(
                "context  {}\nproject  {}\n{}",
                plan.context,
                plan.project,
                plan_lines(plan)
            ),
        )
        .map_err(SpikeError::message)?;
        return Ok(());
    }
    heading("Deployment plan");
    println!("context  {}", plan.context);
    println!("project  {}", plan.project);
    print!("{}", plan_lines(plan));
    Ok(())
}

#[must_use]
fn plan_lines(plan: &PlanView) -> String {
    let mut out = String::new();
    for op in plan.operations {
        out.push_str(&format!(
            "  {} {}  {} on {}\n",
            op_mark(op.kind),
            op.service,
            op.detail,
            op.machine
        ));
    }
    out
}

pub fn confirm_deploy(skip: bool) -> Result<bool, SpikeError> {
    if skip {
        return Ok(true);
    }
    if !io::stdout().is_terminal() {
        println!("confirm skipped (not a TTY; pass --yes)");
        return Ok(false);
    }
    cliclack::confirm("Proceed with deployment to prod?")
        .initial_value(false)
        .interact()
        .map_err(SpikeError::message)
}

pub fn progress(compact: bool, instant: bool) -> Result<(), SpikeError> {
    if !compact && io::stdout().is_terminal() {
        let rows = progress_done();
        let Some(first_row) = rows.first() else {
            return Ok(());
        };
        let Some(second_row) = rows.get(1) else {
            return Ok(());
        };
        let multi = cliclack::multi_progress("Deploying");
        let first = multi.add(cliclack::progress_bar(2));
        let second = multi.add(cliclack::progress_bar(2));
        first.start(format!("{} {}", first_row.name, first_row.phase));
        second.start(format!("{} {}", second_row.name, second_row.phase));
        if !instant {
            std::thread::sleep(std::time::Duration::from_millis(280));
        }
        first.inc(2);
        first.stop(format!("{} {}", first_row.name, first_row.status));
        if !instant {
            std::thread::sleep(std::time::Duration::from_millis(280));
        }
        second.inc(2);
        second.stop(format!("{} {}", second_row.name, second_row.status));
        multi.stop();
        cliclack::outro("endpoints  https://api.prod.example").map_err(SpikeError::message)?;
        return Ok(());
    }
    for tick in 0..=2 {
        let rows = progress_at(tick);
        heading(&format!("[+] Deploying {tick}/2"));
        for row in rows {
            println!("  {}  {}  {}", row.status, row.name, row.phase);
        }
        if !instant && tick != 2 {
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
    }
    println!("endpoints  https://api.prod.example");
    Ok(())
}

pub fn deploy(
    plan_view: &PlanView,
    compact: bool,
    skip_confirm: bool,
    instant: bool,
) -> Result<(), SpikeError> {
    plan(plan_view, compact)?;
    if !confirm_deploy(skip_confirm)? {
        if !compact && io::stdout().is_terminal() {
            cliclack::outro_cancel("cancelled").map_err(SpikeError::message)?;
        }
        return Ok(());
    }
    progress(compact, instant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view;

    #[test]
    fn compact_table_keeps_headers_and_rows() {
        let text = services_table(view::services(), true);
        assert!(text.contains("SERVICE"));
        assert!(text.contains("web/api"));
        assert!(text.contains("HOOKS"));
    }

    #[test]
    fn classic_table_is_boxed() {
        let text = images_table(view::images(), false);
        assert!(text.contains("traefik/whoami:latest"));
        assert!(text.contains('╭') || text.contains('┌'));
    }
}
