//! One Image Build: the per-Service call Cloud fans out, over `prepare`'s build path.
use std::time::Duration;

use ployz_core::RpcError;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::preparation::{self, BuildReceipt, PreparationInput, ReuseInput};
use super::prepare::{PreparationError, build_images};
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

/// Build the one Service; withdraw it as `Queued` if no Build Machine admits
/// it within `start_within`. An admitted build always runs to its end.
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
    let admit_by = start_within.map(|limit| tokio::time::Instant::now() + limit);
    let result = build_images(
        &mut client,
        &captured.intent,
        captured.build,
        &captured.reusable,
        captured.preference,
        admit_by,
        &token,
        &|progress| reporter.report(progress),
    )
    .await;
    match result {
        Ok(builds) => preparation::receipts(&captured.fingerprints, &builds)
            .into_values()
            .next()
            .map(|receipt| BuildOutcome::Built { receipt })
            .ok_or_else(|| super::invalid_argument("Build produced no receipt".into())),
        Err(PreparationError::Build(crate::build::Error::Withdrawn)) => Ok(BuildOutcome::Queued),
        Err(error) => Err(super::preparation_error(error, token.is_cancelled())),
    }
}

/// The Build Platform Requirement of the one Service in `deployment`: the
/// platforms its possible placements run. A Builder outside the Cluster must
/// produce exactly these.
pub(super) async fn platforms(
    mut client: Client,
    deployment: serde_json::Value,
    token: CancellationToken,
) -> Result<Vec<String>, RpcError> {
    let intent = preparation::frozen_intent(deployment)?;
    let machines = super::prepare::observe_machines(&mut client, &intent, &token)
        .await
        .map_err(|error| super::preparation_error(error, token.is_cancelled()))?;
    let platforms = crate::build::placement_platforms(&intent, &machines)
        .map_err(|error| super::invalid_argument(error.to_string()))?;
    Ok(platforms.into_iter().collect())
}

/// The receipt again, naming a Machine that still holds its image, when it stands
/// for the one Service in `input` at its pinned commit and covers every platform
/// the Service's placements run: what [`run`] would reuse, with no checkout and
/// never a build. `None`: a build is needed.
pub(super) async fn reuse(
    mut client: Client,
    input: ReuseInput,
    token: CancellationToken,
) -> Result<Option<BuildReceipt>, RpcError> {
    let ReuseInput {
        deployment,
        source_commits,
        receipt,
    } = input;
    let fingerprints = preparation::expected_fingerprints(deployment.clone(), source_commits)?;
    if fingerprints.len() != 1 || fingerprints.values().next() != Some(&receipt.fingerprint) {
        return Ok(None);
    }
    let intent = preparation::frozen_intent(deployment)?;
    let machines = super::prepare::observe_machines(&mut client, &intent, &token)
        .await
        .map_err(|error| super::preparation_error(error, token.is_cancelled()))?;
    let Ok(required) = crate::build::placement_platforms(&intent, &machines) else {
        return Ok(None);
    };
    if receipt.image.platforms.is_empty()
        || !required
            .iter()
            .all(|platform| receipt.image.platforms.contains(platform))
    {
        return Ok(None);
    }
    let Some(stores) = crate::build::image_stores(&client, &machines, &token).await else {
        return Err(super::preparation_error(PreparationError::Cancelled, true));
    };
    Ok(
        crate::build::holder(&stores, &receipt.image, receipt.machine_id).map(|machine_id| {
            BuildReceipt {
                machine_id,
                ..receipt
            }
        }),
    )
}
