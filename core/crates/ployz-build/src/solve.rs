//! BuildKit `rawjson` progress: one solve status per line, parsed into
//! structured steps so every consumer sees the same tree Buildx renders.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use base64::Engine as _;
use serde::Deserialize;

use crate::{BuildStep, Progress};

/// Longest unterminated line held back before it is passed on as plain output.
const MAX_LINE: usize = 64 * 1024;

#[derive(Deserialize)]
struct SolveStatus {
    #[serde(default)]
    vertexes: Vec<Vertex>,
    #[serde(default)]
    logs: Vec<VertexLog>,
}

#[derive(Deserialize)]
struct Vertex {
    digest: String,
    name: String,
    started: Option<String>,
    completed: Option<String>,
    #[serde(default)]
    cached: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct VertexLog {
    vertex: String,
    stream: u8,
    data: String,
}

/// Splits Buildx output into lines and turns each solve status into progress.
/// Lines that are not solve statuses (warnings, plugin errors) stay `Output`.
#[derive(Default)]
pub(crate) struct SolveParser {
    pending: Vec<u8>,
}

impl SolveParser {
    pub(crate) fn feed(&mut self, bytes: &[u8], progress: &dyn Fn(Progress)) {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<u8>>();
            emit_line(&line, progress);
        }
        if self.pending.len() > MAX_LINE {
            progress(Progress::Output(std::mem::take(&mut self.pending)));
        }
    }

    pub(crate) fn finish(&mut self, progress: &dyn Fn(Progress)) {
        if !self.pending.is_empty() {
            progress(Progress::Output(std::mem::take(&mut self.pending)));
        }
    }
}

fn emit_line(line: &[u8], progress: &dyn Fn(Progress)) {
    let Ok(status) = serde_json::from_slice::<SolveStatus>(line) else {
        progress(Progress::Output(line.to_vec()));
        return;
    };
    for vertex in status.vertexes {
        progress(Progress::Step(BuildStep {
            id: vertex.digest,
            name: vertex.name,
            started: vertex.started,
            completed: vertex.completed,
            cached: vertex.cached,
            error: vertex.error,
        }));
    }
    for log in status.logs {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&log.data)
            .unwrap_or_default();
        progress(Progress::StepOutput {
            step: log.vertex,
            stderr: log.stream == 2,
            text: String::from_utf8_lossy(&data).into_owned(),
        });
    }
}

/// Renders structured progress the way `--progress=plain` would, for terminals.
#[derive(Default)]
pub struct PlainRenderer {
    /// Step number, first start, and last reported completion, so repeated
    /// reports of one step print each transition once.
    steps: Mutex<HashMap<String, (usize, Option<String>, Option<String>)>>,
}

impl PlainRenderer {
    /// Lines to print for this event, if any.
    #[must_use]
    pub fn render(&self, event: &Progress) -> Option<String> {
        let mut steps = self.steps.lock().expect("plain renderer lock");
        match event {
            Progress::Step(step) => {
                let count = steps.len() + 1;
                let (number, started, completed) = steps
                    .entry(step.id.clone())
                    .or_insert_with(|| (count, None, None));
                let mut lines = String::new();
                if started.is_none() && step.started.is_some() {
                    *started = step.started.clone();
                    lines.push_str(&format!("#{number} {}\n", step.name));
                }
                if step.completed.is_some() && *completed != step.completed {
                    *completed = step.completed.clone();
                    if let Some(error) = &step.error {
                        lines.push_str(&format!("#{number} ERROR: {error}\n"));
                    } else if step.cached {
                        lines.push_str(&format!("#{number} CACHED\n"));
                    } else if let (Some(started), Some(completed)) =
                        (&step.started, &step.completed)
                    {
                        lines.push_str(&format!(
                            "#{number} DONE {:.1}s\n",
                            elapsed(started, completed).as_secs_f64()
                        ));
                    }
                }
                (!lines.is_empty()).then_some(lines)
            }
            Progress::StepOutput { step, text, .. } => {
                let number = steps.get(step).map_or(0, |(number, ..)| *number);
                Some(
                    text.lines()
                        .map(|line| format!("#{number} {line}\n"))
                        .collect(),
                )
            }
            Progress::Output(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            Progress::Stage(_) | Progress::Timing { .. } | Progress::Target { .. } => None,
        }
    }
}

fn elapsed(started: &str, completed: &str) -> Duration {
    match (parse_rfc3339(started), parse_rfc3339(completed)) {
        (Some(started), Some(completed)) => completed.duration_since(started).unwrap_or_default(),
        _ => Duration::ZERO,
    }
}

/// BuildKit emits `YYYY-MM-DDTHH:MM:SS.fffffffffZ`; parsing only that shape
/// avoids a date crate for a display-only duration.
fn parse_rfc3339(value: &str) -> Option<SystemTime> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock = clock.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    let nanos: u32 = format!("{fraction:0<9}").get(..9)?.parse().ok()?;
    // Days from the Unix epoch, via the civil-from-days algorithm.
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    let seconds = u64::try_from(seconds).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vertexes_and_logs_and_keeps_other_lines_as_output() {
        let mut parser = SolveParser::default();
        let events = Mutex::new(Vec::new());
        let progress = |event| events.lock().unwrap().push(event);
        let status = concat!(
            r#"{"vertexes":[{"digest":"sha256:a","name":"[sdk 1/2] RUN cargo build","started":"2026-09-22T21:09:06.000000000Z"}]}"#,
            "\n",
            r#"{"logs":[{"vertex":"sha256:a","stream":1,"data":"aGVsbG8K","timestamp":"2026-09-22T21:09:07.000000000Z"}]}"#,
            "\nWARNING: not json\n",
            r#"{"vertexes":[{"digest":"sha256:a","name":"[sdk 1/2] RUN cargo build","started":"2026-09-22T21:09:06.000000000Z","completed":"2026-09-22T21:11:28.500000000Z"}]}"#,
            "\n",
        );
        // Split mid-line so reassembly is exercised.
        let (head, tail) = status.as_bytes().split_at(40);
        parser.feed(head, &progress);
        parser.feed(tail, &progress);
        parser.finish(&progress);
        let mut events = events.into_inner().unwrap();
        // BuildKit re-reports a finished vertex; the renderer prints DONE once.
        events.push(events.last().cloned().unwrap());
        let mut seen = events.iter();
        assert!(
            matches!(seen.next(), Some(Progress::Step(step)) if step.id == "sha256:a" && step.completed.is_none())
        );
        assert!(
            matches!(seen.next(), Some(Progress::StepOutput { step, stderr: false, text }) if step == "sha256:a" && text == "hello\n")
        );
        assert!(
            matches!(seen.next(), Some(Progress::Output(bytes)) if bytes == b"WARNING: not json\n")
        );
        assert!(matches!(seen.next(), Some(Progress::Step(step)) if step.completed.is_some()));
        assert!(matches!(seen.next(), Some(Progress::Step(_))));
        assert!(seen.next().is_none());

        let renderer = PlainRenderer::default();
        let rendered: String = events
            .iter()
            .filter_map(|event| renderer.render(event))
            .collect();
        assert_eq!(
            rendered,
            "#1 [sdk 1/2] RUN cargo build\n#1 hello\nWARNING: not json\n#1 DONE 142.5s\n"
        );
    }
}
