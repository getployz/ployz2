//! Ink-style TTY layout. Static frames use `to_string` so pipes still work.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use iocraft::prelude::*;

use crate::error::SpikeError;
use crate::view::{ImageRow, PlanView, ProgressRow, ServiceRow, op_mark, plan, progress_at};

#[derive(Default, Props)]
struct ServiceTableProps<'a> {
    rows: Option<&'a [ServiceRow]>,
}

#[component]
fn ServiceTable<'a>(props: &ServiceTableProps<'a>) -> impl Into<AnyElement<'a>> {
    element! {
        View(
            margin_top: 1,
            margin_bottom: 1,
            flex_direction: FlexDirection::Column,
            width: 78,
            border_style: BorderStyle::Round,
            border_color: Color::Cyan,
        ) {
            View(padding_left: 1, padding_right: 1, padding_top: 1) {
                Text(content: "ls", weight: Weight::Bold, color: Color::Cyan)
            }
            View(
                border_style: BorderStyle::Single,
                border_edges: Edges::Bottom,
                border_color: Color::Grey,
                padding_left: 1,
                padding_right: 1,
            ) {
                View(width: 40pct) {
                    Text(content: "SERVICE", weight: Weight::Bold)
                }
                View(width: 36pct) {
                    Text(content: "ID", weight: Weight::Bold)
                }
                View(width: 12pct, justify_content: JustifyContent::End) {
                    Text(content: "N", weight: Weight::Bold)
                }
                View(width: 12pct, justify_content: JustifyContent::End) {
                    Text(content: "HOOKS", weight: Weight::Bold)
                }
            }
            #(props.rows.unwrap_or(&[]).iter().enumerate().map(|(index, row)| {
                let stripe = (index % 2 == 1).then_some(Color::DarkGrey);
                element! {
                    View(background_color: stripe, padding_left: 1, padding_right: 1) {
                        View(width: 40pct) {
                            Text(content: row.service)
                        }
                        View(width: 36pct) {
                            Text(content: short_id(row.service_id), color: Color::Grey)
                        }
                        View(width: 12pct, justify_content: JustifyContent::End) {
                            Text(content: row.containers.to_string())
                        }
                        View(width: 12pct, justify_content: JustifyContent::End) {
                            Text(content: row.hooks.to_string())
                        }
                    }
                }
            }))
        }
    }
}

#[derive(Default, Props)]
struct ImageCardsProps<'a> {
    rows: Option<&'a [ImageRow]>,
}

#[component]
fn ImageCards<'a>(props: &ImageCardsProps<'a>) -> impl Into<AnyElement<'a>> {
    element! {
        View(flex_direction: FlexDirection::Column, margin_top: 1, margin_bottom: 1, gap: 1) {
            Text(content: "images", weight: Weight::Bold, color: Color::Cyan)
            #(props.rows.unwrap_or(&[]).iter().map(|row| {
                element! {
                    View(
                        flex_direction: FlexDirection::Column,
                        border_style: BorderStyle::Round,
                        border_color: Color::Grey,
                        padding_left: 1,
                        padding_right: 1,
                        width: 78,
                    ) {
                        View {
                            Text(content: row.repository, weight: Weight::Bold)
                            Text(content: "  ")
                            Text(content: row.machine, color: Color::Cyan)
                        }
                        Text(
                            content: format!("{} · {} · {} · {}", row.id, row.created, row.size, row.platforms),
                            color: Color::Grey,
                        )
                    }
                }
            }))
        }
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[derive(Default, Props)]
struct PlanProps {
    plan: PlanView,
}

#[component]
fn PlanCard(props: &PlanProps) -> impl Into<AnyElement<'static>> {
    let PlanView {
        context,
        project,
        operations,
    } = props.plan;
    element! {
        View(
            flex_direction: FlexDirection::Column,
            margin_top: 1,
            margin_bottom: 1,
            width: 78,
            border_style: BorderStyle::Round,
            border_color: Color::Blue,
            padding: 1,
        ) {
            Text(content: "Deployment plan", weight: Weight::Bold, color: Color::Blue)
            Text(content: format!("context  {context}"), color: Color::Grey)
            Text(content: format!("project  {project}"), color: Color::Grey)
            #(operations.iter().map(|op| {
                let mark = op_mark(op.kind);
                element! {
                    View {
                        Text(content: format!("  {mark} {0}  {1} on {2}", op.service, op.detail, op.machine))
                    }
                }
            }))
        }
    }
}

#[derive(Default, Props)]
struct ProgressProps<'a> {
    title: Option<&'a str>,
    completed: u32,
    total: u32,
    rows: Option<&'a [ProgressRow]>,
}

#[component]
fn ProgressCard<'a>(props: &ProgressProps<'a>) -> impl Into<AnyElement<'a>> {
    let title = props.title.unwrap_or("Deploying");
    element! {
        View(flex_direction: FlexDirection::Column, margin_top: 1, width: 78) {
            Text(
                content: format!("[+] {title} {}/{}", props.completed, props.total),
                weight: Weight::Bold,
                color: Color::Green,
            )
            #(props.rows.unwrap_or(&[]).iter().map(|row| {
                let color = match row.status {
                    "done" => Color::Green,
                    "running" => Color::Yellow,
                    _ => Color::Grey,
                };
                element! {
                    View(padding_left: 2) {
                        Text(content: format!("{}  {}  {}", glyph(row.status), row.name, row.phase), color: color)
                    }
                }
            }))
        }
    }
}

fn glyph(status: &str) -> &'static str {
    match status {
        "done" => "✓",
        "running" => "…",
        _ => "·",
    }
}

#[derive(Default, Props)]
struct DeployFlowProps {
    skip_confirm: bool,
    instant: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Confirm,
    Running,
    Done,
}

#[component]
fn DeployFlow(props: &DeployFlowProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut system = hooks.use_context_mut::<SystemContext>();
    let phase = hooks.use_state(|| {
        if props.skip_confirm {
            Phase::Running
        } else {
            Phase::Confirm
        }
    });
    let tick = hooks.use_state(|| 0u32);
    let delay = if props.instant {
        Duration::from_millis(1)
    } else {
        Duration::from_millis(280)
    };

    hooks.use_terminal_events({
        let mut phase = phase;
        move |event| {
            if phase.get() != Phase::Confirm {
                return;
            }
            let TerminalEvent::Key(KeyEvent { code, kind, .. }) = event else {
                return;
            };
            if kind == KeyEventKind::Release {
                return;
            }
            if matches!(code, KeyCode::Char('y' | 'Y')) {
                phase.set(Phase::Running);
            } else if matches!(code, KeyCode::Char('n' | 'N') | KeyCode::Esc) {
                phase.set(Phase::Done);
            }
        }
    });

    hooks.use_future({
        let mut tick = tick;
        let mut phase = phase;
        async move {
            loop {
                tokio::time::sleep(delay).await;
                if phase.get() != Phase::Running {
                    continue;
                }
                let next = tick.get() + 1;
                tick.set(next);
                if next >= 2 {
                    phase.set(Phase::Done);
                }
            }
        }
    });

    if phase.get() == Phase::Done {
        system.exit();
    }

    let snapshot = progress_at(tick.get());
    let completed = snapshot.iter().filter(|row| row.status == "done").count() as u32;
    let total = 2u32;

    element! {
        View(flex_direction: FlexDirection::Column) {
            PlanCard(plan: plan())
            #( (phase.get() == Phase::Confirm).then(|| element! {
                View(padding_left: 1, margin_bottom: 1) {
                    Text(content: "Proceed with deployment to prod? [y/N]", color: Color::Yellow)
                }
            }))
            #( (phase.get() != Phase::Confirm).then(|| element! {
                ProgressCard(
                    title: "Deploying",
                    completed: completed,
                    total: total,
                    rows: snapshot,
                )
            }))
        }
    }
}

fn emit(element: &mut impl ElementExt) -> Result<(), SpikeError> {
    element.write_to_is_terminal(io::stdout())?;
    Ok(())
}

pub fn services(rows: &[ServiceRow]) -> Result<(), SpikeError> {
    emit(&mut element!(ServiceTable(rows: rows)))
}

pub fn images(rows: &[ImageRow]) -> Result<(), SpikeError> {
    emit(&mut element!(ImageCards(rows: rows)))
}

pub fn plan_view(plan: PlanView) -> Result<(), SpikeError> {
    emit(&mut element!(PlanCard(plan: plan)))
}

pub fn progress(rows: &[ProgressRow], completed: u32, total: u32) -> Result<(), SpikeError> {
    emit(&mut element!(ProgressCard(
        title: "Deploying",
        completed: completed,
        total: total,
        rows: rows,
    )))
}

pub async fn deploy(skip_confirm: bool, instant: bool) -> Result<(), SpikeError> {
    if io::stdout().is_terminal() {
        element!(DeployFlow(skip_confirm: skip_confirm, instant: instant))
            .render_loop()
            .await
            .map_err(SpikeError::message)?;
        let mut stdout = io::stdout();
        writeln!(stdout, "endpoints  https://api.prod.example")?;
        return Ok(());
    }
    plan_view(plan())?;
    let rows = progress_at(2);
    progress(rows, 2, 2)?;
    println!("endpoints  https://api.prod.example");
    Ok(())
}

#[cfg(test)]
#[must_use]
fn services_text(rows: &[ServiceRow]) -> String {
    element!(ServiceTable(rows: rows)).to_string()
}

#[cfg(test)]
#[must_use]
fn images_text(rows: &[ImageRow]) -> String {
    element!(ImageCards(rows: rows)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view;

    #[test]
    fn service_table_names_the_services() {
        let text = services_text(view::services());
        assert!(text.contains("web/api"));
        assert!(text.contains("web/web"));
        assert!(text.contains("SERVICE"));
    }

    #[test]
    fn image_cards_keep_tag_and_machine() {
        let text = images_text(view::images());
        assert!(text.contains("traefik/whoami:latest"));
        assert!(text.contains("ord1"));
        assert!(text.contains("ghcr.io/acme/api:1.4"));
    }
}
