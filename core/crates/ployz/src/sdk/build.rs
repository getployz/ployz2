//! One Image Build: the per-Service call Cloud fans out, over `prepare`'s build path.
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ployz_build::{Progress as BuildProgress, Stage};
use ployz_core::RpcError;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::preparation::{self, BuildReceipt, PreparationInput};
use super::prepare::{Progress, build_images};
use super::running::Reporter;
use crate::connect::Client;

/// How one Image Build call ended, short of failure.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuildOutcome {
    /// No Build Machine admitted the build within its start limit; it was withdrawn.
    Queued,
    /// The build finished; its receipt identifies the image.
    Built { receipt: BuildReceipt },
}

pub(super) async fn run(
    mut client: Client,
    input: PreparationInput,
    start_within: Option<Duration>,
    token: CancellationToken,
    reporter: Reporter,
) -> Result<BuildOutcome, RpcError> {
    let captured = super::capture(input).await?;
    if captured.build.targets().count() != 1 || captured.fingerprints.len() != 1 {
        return Err(super::invalid_argument(
            "build input must hold exactly one Git Service with its source commit".into(),
        ));
    }
    let started = AtomicBool::new(false);
    let withdraw = token.child_token();
    let progress = |progress| {
        // The adapter reports Upload in the same poll that observes admission.
        if matches!(
            progress,
            Progress::Build(BuildProgress::Stage(Stage::Upload))
        ) {
            started.store(true, Ordering::Relaxed);
        }
        reporter.report(progress);
    };
    let work = build_images(
        &mut client,
        &captured.intent,
        captured.build,
        &captured.reusable,
        &withdraw,
        &progress,
    );
    tokio::pin!(work);
    let result = match start_within {
        None => work.await,
        Some(limit) => tokio::select! {
            biased;
            result = &mut work => result,
            () = tokio::time::sleep(limit) => {
                if !started.load(Ordering::Relaxed) {
                    withdraw.cancel();
                }
                work.await
            }
        },
    };
    match result {
        Ok(builds) => preparation::receipts(&captured.fingerprints, &builds)
            .into_values()
            .next()
            .map(|receipt| BuildOutcome::Built { receipt })
            .ok_or_else(|| super::invalid_argument("Build produced no receipt".into())),
        // Withdrawn, not cancelled by the caller, and no source left the client.
        Err(error)
            if withdraw.is_cancelled()
                && !token.is_cancelled()
                && error.cancelled_before_upload() =>
        {
            Ok(BuildOutcome::Queued)
        }
        Err(error) => Err(super::preparation_error(error, token.is_cancelled())),
    }
}
