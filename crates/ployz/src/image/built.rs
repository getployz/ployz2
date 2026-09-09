//! Deliver a completed Build from the Machine that holds its exact content.

use ployz_build::BuiltImage;
use ployz_core::{ListImagesRequest, MachineObservation};

use super::*;

/// Transfer the completed image directly from its Build Machine. The client
/// neither inspects Docker nor carries image bytes or selects another source.
/// # Errors
/// Reports selection, source capability, and source image-store failures.
/// Destination failures and unattempted destinations remain in the partial result.
pub async fn push_from_machine(
    client: &mut Client,
    image: &BuiltImage,
    source: MachineId,
    selectors: &[String],
) -> Result<PartialResult<(), PushError>, PushError> {
    let machines = client.machines().await?;
    push_from_machine_using_machines(client, image, source, selectors, &machines).await
}

pub(crate) async fn push_from_machine_using_machines(
    client: &mut Client,
    image: &BuiltImage,
    source: MachineId,
    selectors: &[String],
    machines: &[MachineObservation],
) -> Result<PartialResult<(), PushError>, PushError> {
    let selection = list_selection(machines, selectors)?;
    let mut result = PartialResult {
        successes: Vec::new(),
        failures: Vec::new(),
        omissions: selection.omissions,
    };
    if selection.targets.is_empty() {
        return Ok(result);
    }
    let mut cancellation = Cancellation::new();
    let source = serve_build_image(client, image, source).await?;
    let mut remaining = selection.targets.into_iter();
    while let Some(machine) = remaining.next() {
        match pull_on_machine(
            client,
            &image.reference,
            &machine,
            source,
            &mut cancellation,
        )
        .await
        {
            Ok(()) => result.successes.push(MachineSuccess {
                machine_id: machine.id,
                value: (),
            }),
            Err(error) => {
                let cancelled = error.is_cancellation();
                result.failures.push(MachineFailure {
                    machine_id: machine.id,
                    error,
                });
                if cancelled {
                    result.omissions.extend(remaining.map(|machine| machine.id));
                    break;
                }
            }
        }
    }
    Ok(result)
}

/// Open the actual complete Build source for either delivery or a later Build.
pub(crate) async fn serve_build_image(
    client: &mut Client,
    image: &BuiltImage,
    source: MachineId,
) -> Result<ImageIngestDestination, PushError> {
    let mut cancellation = Cancellation::new();
    let reference =
        image
            .reference
            .parse::<Reference>()
            .map_err(|error| PushError::InvalidReference {
                reference: image.reference.clone(),
                message: error.to_string(),
            })?;
    let digest = reference
        .digest()
        .ok_or_else(|| PushError::InvalidReference {
            reference: image.reference.clone(),
            message: "a completed Build must have an immutable digest".into(),
        })?;
    let target = MachineTarget::from(&source);
    // Docker's reference filter does not match repository@digest. Compare the
    // manifest identity and available platforms in the source's actual store.
    let stored = cancellation
        .race(client.call::<op::ListImages>(ListImagesRequest { reference: None }, Some(&target)))
        .await??;
    if !stored.containerd_store {
        return Err(PushError::UnsupportedImageStore);
    }
    if !stored.images.iter().any(|stored| {
        stored.id == digest
            && stored.platforms.iter().any(|platform| {
                platform == &image.platform
                    || matches!(
                        (platform.as_str(), image.platform.as_str()),
                        ("linux/arm64", "linux/arm64/v8") | ("linux/arm64/v8", "linux/arm64")
                    )
            })
    }) {
        return Err(PushError::BuildImageUnavailable {
            image: image.reference.clone(),
            machine_id: source,
            platform: image.platform.clone(),
        });
    }
    let opened = cancellation
        .race(client.call::<op::EnsureImageIngest>(EnsureImageIngestRequest {}, Some(&target)))
        .await?
        .map_err(|error| ingest_error(rpc_error(error)))?;
    Ok(opened.destination)
}
