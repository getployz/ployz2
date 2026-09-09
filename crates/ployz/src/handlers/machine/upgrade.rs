//! Explicit, bounded Machine upgrade sequencing and inspection.

use std::{collections::BTreeSet, time::Duration};

use clap::ArgMatches;
use ployz_core::{
    InspectMachineUpgradeRequest, Machine, MachineRelease, MachineTarget, MachineUpgradeAttempt,
    MachineUpgradeAttemptId, NameMatches, RequestMachineUpgradeRequest, op,
};
use tokio::time::Instant;

use crate::{cluster::Client, connect::ConnectError};

use super::super::{Error, leaf_matches, string_values, with_client};

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(16 * 60);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(in crate::handlers) fn upgrade(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let release = matches
        .get_one::<MachineRelease>("version")
        .cloned()
        .ok_or_else(|| Error::usage("upgrade version is required"))?;
    let selectors = string_values(matches, "machine");
    with_client(root, |client| {
        Box::pin(async move {
            let machines = selected_machines(client, &selectors).await?;
            for machine in machines {
                let attempt_id = MachineUpgradeAttemptId::random();
                let attempt = run_one(client, &machine, release.clone(), attempt_id).await?;
                print_attempt(&machine, &attempt);
                match attempt {
                    MachineUpgradeAttempt::Succeeded { .. } => {}
                    MachineUpgradeAttempt::Failed { error, .. } => {
                        return Err(Error::usage(error));
                    }
                    MachineUpgradeAttempt::Interrupted { .. } => {
                        return Err(Error::usage(format!(
                            "Machine {} upgrade was interrupted; {}",
                            machine.name,
                            journal_hint(attempt_id)
                        )));
                    }
                    MachineUpgradeAttempt::Accepted { .. }
                    | MachineUpgradeAttempt::Running { .. } => {
                        unreachable!("run_one returns only terminal evidence")
                    }
                }
            }
            Ok(())
        })
    })
}

pub(in crate::handlers) fn inspect(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let target = MachineTarget::parse(
        matches
            .get_one::<String>("machine")
            .ok_or_else(|| Error::usage("machine is required"))?,
    )?;
    let attempt_id = matches
        .get_one::<MachineUpgradeAttemptId>("attempt")
        .copied();
    let json = matches.get_one::<String>("output").is_some();
    with_client(root, |client| {
        Box::pin(async move {
            let attempt = client
                .call_repeatable::<op::InspectMachineUpgrade>(
                    InspectMachineUpgradeRequest { attempt_id },
                    Some(&target),
                )
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&attempt)?);
            } else {
                print_attempt_target(target.as_str(), &attempt);
            }
            Ok(())
        })
    })
}

async fn selected_machines(
    client: &mut Client,
    selectors: &[String],
) -> Result<Vec<Machine>, Error> {
    let visible = client.machines().await?;
    let mut selected = Vec::with_capacity(selectors.len());
    let mut ids = BTreeSet::new();
    for selector in selectors {
        let target = MachineTarget::parse(selector)?;
        let machine = match target.resolve(visible.iter().map(|entry| &entry.machine)) {
            NameMatches::None => {
                return Err(Error::usage(format!(
                    "Machine {} was not found",
                    selector.escape_debug()
                )));
            }
            NameMatches::One(machine) => machine,
            matches @ NameMatches::Ambiguous { .. } => {
                return Err(Error::usage(format!(
                    "Machine name {} is ambiguous: {}",
                    selector.escape_debug(),
                    matches
                        .iter()
                        .map(|machine| machine.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        if !ids.insert(machine.id) {
            return Err(Error::usage(format!(
                "Machine {} was selected more than once",
                machine.name
            )));
        }
        selected.push(machine.clone());
    }
    Ok(selected)
}

async fn run_one(
    client: &mut Client,
    machine: &Machine,
    release: MachineRelease,
    attempt_id: MachineUpgradeAttemptId,
) -> Result<MachineUpgradeAttempt, Error> {
    let target = MachineTarget::from(&machine.id);
    let deadline = Instant::now() + OBSERVATION_TIMEOUT;
    let request = RequestMachineUpgradeRequest {
        attempt_id,
        release,
    };
    let accepted = request_until(deadline, client, &target, request)
        .await
        .map_err(|error| uncertain(machine, attempt_id, error))?;
    print_attempt(machine, &accepted);
    if accepted.is_terminal() {
        return Ok(accepted);
    }

    loop {
        if tokio::time::timeout_at(deadline, tokio::time::sleep(POLL_INTERVAL))
            .await
            .is_err()
        {
            return Err(uncertain_timeout(machine, attempt_id));
        }
        let observed = inspect_until(deadline, client, &target, attempt_id)
            .await
            .map_err(|error| uncertain(machine, attempt_id, error))?;
        if observed.is_terminal() {
            return Ok(observed);
        }
    }
}

async fn request_until(
    deadline: Instant,
    client: &mut Client,
    target: &MachineTarget,
    request: RequestMachineUpgradeRequest,
) -> Result<MachineUpgradeAttempt, ConnectError> {
    loop {
        let outcome = tokio::time::timeout_at(
            deadline,
            client.call_repeatable::<op::RequestMachineUpgrade>(request.clone(), Some(target)),
        )
        .await;
        match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if !error.is_setup_retryable() => return Err(error),
            Ok(Err(_)) => {}
            Err(_) => {
                return Err(
                    tonic::Status::deadline_exceeded("upgrade observation timed out").into(),
                );
            }
        }
    }
}

async fn inspect_until(
    deadline: Instant,
    client: &mut Client,
    target: &MachineTarget,
    attempt_id: MachineUpgradeAttemptId,
) -> Result<MachineUpgradeAttempt, ConnectError> {
    loop {
        let outcome = tokio::time::timeout_at(
            deadline,
            client.call_repeatable::<op::InspectMachineUpgrade>(
                InspectMachineUpgradeRequest {
                    attempt_id: Some(attempt_id),
                },
                Some(target),
            ),
        )
        .await;
        match outcome {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if !error.is_setup_retryable() => return Err(error),
            Ok(Err(_)) => {}
            Err(_) => {
                return Err(
                    tonic::Status::deadline_exceeded("upgrade observation timed out").into(),
                );
            }
        }
    }
}

fn print_attempt(machine: &Machine, attempt: &MachineUpgradeAttempt) {
    print_attempt_target(&format!("{} ({})", machine.name, machine.id), attempt);
}

fn print_attempt_target(machine: &str, attempt: &MachineUpgradeAttempt) {
    match attempt {
        MachineUpgradeAttempt::Accepted { attempt_id, target } => println!(
            "Machine {machine}: upgrade {attempt_id} accepted for {target}; {}",
            journal_hint(*attempt_id)
        ),
        MachineUpgradeAttempt::Running {
            attempt_id,
            target,
            stage,
        } => println!(
            "Machine {machine}: upgrade {attempt_id} is {} for {target}; {}",
            stage.as_str(),
            journal_hint(*attempt_id)
        ),
        MachineUpgradeAttempt::Succeeded {
            attempt_id,
            version,
        } => {
            println!("Machine {machine}: upgrade {attempt_id} succeeded; running version {version}")
        }
        MachineUpgradeAttempt::Failed {
            attempt_id,
            target,
            stage,
            error,
        } => println!(
            "Machine {machine}: upgrade {attempt_id} failed at {} for {target}: {error}; {}",
            stage.as_str(),
            journal_hint(*attempt_id)
        ),
        MachineUpgradeAttempt::Interrupted {
            attempt_id,
            target,
            stage,
        } => println!(
            "Machine {machine}: upgrade {attempt_id} was interrupted at {} for {target}; {}",
            stage.as_str(),
            journal_hint(*attempt_id)
        ),
    }
}

fn journal_hint(attempt_id: MachineUpgradeAttemptId) -> String {
    format!("inspect locally with `journalctl -u ployz-upgrade-{attempt_id}.service`")
}

fn uncertain(machine: &Machine, attempt_id: MachineUpgradeAttemptId, error: ConnectError) -> Error {
    Error::usage(format!(
        "Machine {} ({}) upgrade {attempt_id} outcome is uncertain: {error}; reconnect and run `ployz machine upgrade inspect {} --attempt {attempt_id}`; {}",
        machine.name,
        machine.id,
        machine.id,
        journal_hint(attempt_id)
    ))
}

fn uncertain_timeout(machine: &Machine, attempt_id: MachineUpgradeAttemptId) -> Error {
    Error::usage(format!(
        "Machine {} ({}) upgrade {attempt_id} outcome is uncertain after {} minutes; reconnect and run `ployz machine upgrade inspect {} --attempt {attempt_id}`; {}",
        machine.name,
        machine.id,
        OBSERVATION_TIMEOUT.as_secs() / 60,
        machine.id,
        journal_hint(attempt_id)
    ))
}
