//! Bounded retries for safe operations after setup has begun.

use std::{fmt::Display, time::Duration};

use tokio::time::{Instant, sleep, timeout_at};

use crate::failure::Failure;

pub(crate) const WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error<E> {
    #[error("{0}")]
    Permanent(E),
    #[error("{0}")]
    Exhausted(String),
}

impl<E: Display> From<Error<E>> for Failure {
    fn from(error: Error<E>) -> Self {
        Self::usage(error.to_string())
    }
}

/// Only pass reads or operations known to be safe to repeat. The deadline also
/// bounds any retries inside the operation; it must not wrap a whole setup flow.
pub(crate) async fn run<C, T, E: Display>(
    context: &mut C,
    operation: &str,
    wait: Duration,
    retryable: impl Fn(&E) -> bool,
    mut attempt: impl AsyncFnMut(&mut C) -> Result<T, E>,
) -> Result<T, Error<E>> {
    let deadline = Instant::now() + wait;
    let mut last = None;
    loop {
        match timeout_at(deadline, attempt(context)).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if !retryable(&error) => {
                return Err(Error::Permanent(error));
            }
            Ok(Err(error)) => {
                if last.is_none() {
                    eprintln!(
                        "{operation}: {error}; retrying for up to {}s. Check outbound firewall access if this connection is blocked.",
                        deadline.saturating_duration_since(Instant::now()).as_secs()
                    );
                }
                last = Some(error.to_string());
            }
            Err(_) => break,
        }
        if timeout_at(deadline, sleep(Duration::from_secs(1)))
            .await
            .is_err()
        {
            break;
        }
    }
    Err(Error::Exhausted(format!(
        "{operation} did not recover within {}s; last error: {}",
        wait.as_secs(),
        last.as_deref().unwrap_or("request timed out")
    )))
}

/// Include the transport cause, which reqwest's top-level Display omits.
pub(crate) fn detail(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        text.push_str(&format!(": {error}"));
        source = error.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retries_transient_failures_but_stops_on_permanent_errors_and_at_deadline() {
        let mut calls = 0;
        let result = run(
            &mut calls,
            "probe",
            WAIT,
            |_| true,
            async |calls| {
                *calls += 1;
                if *calls < 3 {
                    Err("connection refused")
                } else {
                    Ok(42)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!((result, calls), (42, 3));

        calls = 0;
        let error = run(
            &mut calls,
            "probe",
            WAIT,
            |_| false,
            async |calls| {
                *calls += 1;
                Err::<(), _>("wrong identity")
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(error.to_string().contains("wrong identity"));

        let started = Instant::now();
        let error = run(
            &mut (),
            "probe",
            WAIT,
            |_| true,
            async |_| Err::<(), _>("connection refused"),
        )
        .await
        .unwrap_err();
        assert_eq!(Instant::now() - started, WAIT);
        assert!(error.to_string().contains("last error: connection refused"));

        let started = Instant::now();
        let error = run(
            &mut (),
            "probe",
            WAIT,
            |_| true,
            async |_| std::future::pending::<Result<(), &str>>().await,
        )
        .await
        .unwrap_err();
        assert_eq!(Instant::now() - started, WAIT);
        assert!(error.to_string().contains("request timed out"));
    }
}
