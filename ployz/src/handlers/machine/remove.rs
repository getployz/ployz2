use std::io::{self, IsTerminal, Write};

use clap::ArgMatches;
use ployz_core::{
    DataLoss, DescribeContractRequest, LiveServices, Machine, MachineId, MachineName,
    MachineTarget, NameMatches, ObservedDataLoss, RemoveMachineRequest, RpcError, RpcErrorCode,
    ServiceName, UnconfirmedDataLoss, op,
};

use super::super::{connect_client, runtime, string_values};
use super::{ConnectionOptions, helpers, machine_list, target};
use crate::handlers::{Error, leaf_matches};

pub(in crate::handlers) fn remove(root: &ArgMatches) -> Result<(), Error> {
    let options = ConnectionOptions::from_matches(root)?;
    let matches = leaf_matches(root);
    let selector = target(matches, "machine")?.to_owned();
    let no_reset = matches.get_flag("no-reset");
    let yes = matches.get_flag("yes");
    let named = string_values(matches, "data-loss");
    runtime()?.block_on(async {
        let mut client = connect_client(matches, options.context()).await?;
        let machines = machine_list(&mut client).await?;
        let selected = select_machine(&machines, &selector)?;
        let selected_target = MachineTarget::from(&selected.id);
        let current = client
            .call::<op::DescribeContract>(DescribeContractRequest {}, None)
            .await?
            .machine_id;
        if selected.id == current && machines.len() > 1 {
            return Err(Error::usage(
                "the current entry Machine cannot be removed while another Machine is visible",
            ));
        }
        let selected_observation = machines
            .iter()
            .find(|entry| entry.machine.id == selected.id)
            .expect("selected Machine came from this list");
        let confirmation = if no_reset {
            Vec::new()
        } else {
            let observed = client
                .data_loss_if_machine_removed(&selected_target)
                .await?;
            for line in data_loss_listing(&observed) {
                eprintln!("{line}");
            }
            let names = if named.is_empty() && !observed.data_loss.is_empty() {
                read_data_loss_names(&observed)?
            } else {
                named.clone()
            };
            confirm_listed_data_loss(&observed, &names)?
        };
        let services = services_on(
            &selected.id,
            &client
                .live_services_from(std::slice::from_ref(selected_observation))
                .await?,
        );
        for line in service_warnings(&selected.name, &services) {
            eprintln!("{line}");
        }
        helpers::confirm(
            yes,
            &format!("Remove Machine {} ({})?", selected.name, selected.id),
        )?;

        // TODO(UT-055): do not reroute away from the current entry before removal.
        // TODO(UT-056): there is no drain or unschedulable phase before cleanup.
        if no_reset {
            client
                .call::<op::RemoveMachine>(
                    RemoveMachineRequest {
                        machine_id: selected.id,
                    },
                    None,
                )
                .await?;
        } else {
            let removed = client
                .remove_machine(&selected_target, &confirmation)
                .await
                .map_err(refusal_from_rpc)?;
            if let Some(warning) = removed.reset_warning {
                eprintln!("WARNING: target cleanup/reset failed: {warning}");
            }
        }

        // Drop the local connection before DNS refresh. A refresh failure
        // must not leave the removed Machine named in the context (#249).
        let mut config = options.load_or_empty_config()?;
        if let Some(context_name) = config.context_name(options.context()).map(str::to_owned)
            && let Some(context) = config.contexts.get_mut(&context_name)
        {
            context.drop_machine(&selected.id);
            config.save()?;
        }
        if let Err(error) =
            crate::dns::update_records_after_removal(&mut client, machines, &selected.id).await
        {
            return Err(Error::warned(
                "hosted DNS refresh failed after removing the Machine",
                error,
            ));
        }
        println!("Removed Machine {} ({})", selected.name, selected.id);
        Ok::<_, Error>(())
    })
}

fn select_machine(
    machines: &[ployz_core::MachineObservation],
    selector: &str,
) -> Result<Machine, Error> {
    let selector = MachineTarget::parse(selector)?;
    match selector.resolve(machines.iter().map(|entry| &entry.machine)) {
        NameMatches::None => Err(Error::usage(format!("Machine {selector:?} was not found"))),
        NameMatches::One(machine) => Ok(machine.clone()),
        NameMatches::Ambiguous(matches) => Err(Error::usage(format!(
            "Machine name {selector:?} is ambiguous: {}",
            matches
                .into_iter()
                .map(|machine| machine.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn data_loss_listing(observed: &ObservedDataLoss) -> Vec<String> {
    if observed.data_loss.is_empty() {
        return Vec::new();
    }
    std::iter::once("Data Loss:".to_owned())
        .chain(
            observed
                .data_loss
                .iter()
                .map(|loss| format!("  {}", loss.name())),
        )
        .collect()
}

fn confirm_listed_data_loss(
    observed: &ObservedDataLoss,
    names: &[String],
) -> Result<Vec<DataLoss>, Error> {
    let confirmation = observed.named(names.iter().map(String::as_str))?;
    let missing = observed.uncovered_by(&confirmation);
    if missing.is_empty() {
        Ok(confirmation)
    } else {
        Err(Error::usage(pass_data_loss_names_message(&missing)))
    }
}

fn read_data_loss_names(observed: &ObservedDataLoss) -> Result<Vec<String>, Error> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::usage(pass_data_loss_names_message(
            &observed.data_loss,
        )));
    }
    print!("Name the Data Loss to continue: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.split_whitespace().map(str::to_owned).collect())
}

fn pass_data_loss_names_message(missing: &[DataLoss]) -> String {
    format!(
        "Data Loss is not covered by the confirmation; pass the names as arguments: {}",
        missing
            .iter()
            .map(DataLoss::name)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn refusal_from_rpc(error: RpcError) -> Error {
    if error.code == RpcErrorCode::InvalidArgument
        && let Ok(unconfirmed) =
            serde_json::from_value::<UnconfirmedDataLoss>(error.details.clone())
        && !unconfirmed.missing.is_empty()
    {
        return Error::usage(pass_data_loss_names_message(&unconfirmed.missing));
    }
    error.into()
}

#[must_use]
fn services_on(machine_id: &MachineId, live: &LiveServices<RpcError>) -> Vec<ServiceName> {
    live.services()
        .into_iter()
        .filter(|service| {
            service
                .containers
                .iter()
                .any(|container| container.as_observation().machine_id == *machine_id)
        })
        .filter_map(|service| service.service_name().cloned())
        .collect()
}

#[must_use]
fn service_warnings(machine: &MachineName, services: &[ServiceName]) -> Vec<String> {
    if services.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "WARNING: Machine {machine} is running Services: {}",
        services
            .iter()
            .map(ServiceName::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    )]
}

#[cfg(test)]
mod tests {
    use ployz_core::{
        ContainerKind, ContainerObservation, ContainerRuntimeObservation, DataLoss, DockerVolumeId,
        DockerVolumeName, HealthObservation, LiveServices, MachineId, MachineName, MachineSuccess,
        ObservedDataLoss, PartialResult, RpcError, RpcErrorCode, ServiceId, ServiceName,
        UnconfirmedDataLoss, derive_live_services,
    };
    use serde_json::{Value, json};

    use super::{
        confirm_listed_data_loss, data_loss_listing, pass_data_loss_names_message,
        refusal_from_rpc, service_warnings, services_on,
    };
    use crate::connect::ConnectError;

    #[test]
    fn data_loss_listing_is_silent_when_nothing_would_be_destroyed() {
        assert_eq!(
            data_loss_listing(&ObservedDataLoss {
                data_loss: Vec::new()
            }),
            Vec::<String>::new()
        );
    }

    #[test]
    fn data_loss_listing_names_entries_before_any_prompt() {
        assert_eq!(
            data_loss_listing(&ObservedDataLoss {
                data_loss: vec![loss('a', "ams-critical"), loss('a', "data")],
            }),
            ["Data Loss:", "  ams-critical", "  data"]
        );
    }

    #[test]
    fn confirming_listed_data_loss_resolves_display_names() {
        let observed = ObservedDataLoss {
            data_loss: vec![loss('a', "data"), loss('a', "logs")],
        };
        assert_eq!(
            confirm_listed_data_loss(&observed, &["logs".into(), "data".into()]).unwrap(),
            vec![loss('a', "logs"), loss('a', "data")]
        );
    }

    #[test]
    fn confirming_listed_data_loss_ignores_extra_names() {
        let observed = ObservedDataLoss {
            data_loss: vec![loss('a', "data")],
        };
        assert_eq!(
            confirm_listed_data_loss(&observed, &["data".into(), "gone".into()]).unwrap(),
            vec![loss('a', "data")]
        );
    }

    #[test]
    fn refusing_without_a_confirmation_states_which_names_to_pass() {
        let observed = ObservedDataLoss {
            data_loss: vec![loss('a', "data"), loss('a', "logs")],
        };
        let error = confirm_listed_data_loss(&observed, &[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Data Loss is not covered by the confirmation; pass the names as arguments: data logs"
        );
    }

    #[test]
    fn typed_names_that_match_more_than_one_listed_entry_are_refused() {
        let observed = ObservedDataLoss {
            data_loss: vec![loss('a', "data"), loss('b', "data")],
        };
        let error = confirm_listed_data_loss(&observed, &["data".into()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Data Loss name \"data\" matches more than one listed entry"
        );
    }

    #[test]
    fn a_machine_with_no_data_loss_needs_no_names() {
        let observed = ObservedDataLoss {
            data_loss: Vec::new(),
        };
        assert_eq!(
            confirm_listed_data_loss(&observed, &[]).unwrap(),
            Vec::<DataLoss>::new()
        );
    }

    #[test]
    fn execute_time_unconfirmed_data_loss_states_which_names_to_pass() {
        let missing = vec![loss('a', "logs")];
        let error = UnconfirmedDataLoss {
            missing: missing.clone(),
        }
        .into_rpc_error();
        assert_eq!(
            refusal_from_rpc(error).to_string(),
            pass_data_loss_names_message(&missing)
        );
    }

    #[test]
    fn service_warnings_are_silent_when_nothing_is_at_stake() {
        assert_eq!(
            service_warnings(&MachineName::parse("ams1").unwrap(), &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn service_warnings_name_services_on_the_machine() {
        assert_eq!(
            service_warnings(
                &MachineName::parse("ams1").unwrap(),
                &[
                    ServiceName::parse("api").unwrap(),
                    ServiceName::parse("web").unwrap(),
                ],
            ),
            vec!["WARNING: Machine ams1 is running Services: api, web".to_owned()]
        );
    }

    #[test]
    fn services_on_names_services_with_a_service_container_on_that_machine() {
        let live: LiveServices<RpcError> = derive_live_services(PartialResult {
            successes: vec![
                MachineSuccess {
                    machine_id: machine_id('a'),
                    value: vec![
                        observation('1', 'a', "api", ContainerKind::ServiceContainer, 'a'),
                        observation('2', 'a', "api", ContainerKind::ServiceContainer, 'a'),
                        observation('3', 'a', "hooked", ContainerKind::PreDeployHook, 'b'),
                    ],
                },
                MachineSuccess {
                    machine_id: machine_id('b'),
                    value: vec![observation(
                        '4',
                        'b',
                        "web",
                        ContainerKind::ServiceContainer,
                        'c',
                    )],
                },
            ],
            failures: Vec::new(),
            omissions: Vec::new(),
        });
        assert_eq!(
            services_on(&machine_id('a'), &live),
            [ServiceName::parse("api").unwrap()]
        );
    }

    fn loss(machine: char, name: &str) -> DataLoss {
        DataLoss::DockerVolume(DockerVolumeId {
            machine_id: machine_id(machine),
            name: DockerVolumeName::parse(name).unwrap(),
        })
    }

    fn observation(
        id: char,
        machine: char,
        name: &str,
        kind: ContainerKind,
        service: char,
    ) -> ContainerObservation {
        let service_id = ServiceId::parse(service.to_string().repeat(32)).unwrap();
        let service_name = ServiceName::parse(name).unwrap();
        ContainerObservation {
            container_id: ployz_core::ContainerId::parse(id.to_string().repeat(64)).unwrap(),
            display_name: name.into(),
            created_at_unix_nanos: 0,
            machine_id: machine_id(machine),
            service_id,
            service_name: service_name.clone(),
            kind,
            runtime: ContainerRuntimeObservation::Running {
                health: HealthObservation::Healthy,
            },
            effective_healthcheck: None,
            resolved_spec: serde_json::from_value(json!({
                "service_id": service_id,
                "name": service_name,
                "mode": { "mode": "replicated", "replicas": 1 },
                "container": { "image": "alpine:3.23.3", "pull_policy": "missing" }
            }))
            .unwrap(),
            address: None,
            labels: Default::default(),
        }
    }

    fn machine_id(value: char) -> MachineId {
        MachineId::parse(value.to_string().repeat(32)).unwrap()
    }

    #[test]
    fn reached_target_cleanup_rejections_are_not_unreachable_fallbacks() {
        assert!(
            ConnectError::Rpc(crate::connect::TransportError::from(
                tonic::Status::unavailable("route failed")
            ))
            .is_unreachable()
        );
        assert!(
            !ConnectError::Rpc(crate::connect::TransportError::from(
                tonic::Status::unimplemented("older daemon")
            ))
            .is_unreachable()
        );
        assert!(
            !ConnectError::Remote(RpcError {
                code: RpcErrorCode::Unavailable,
                message: "Docker is unavailable".into(),
                details: Value::Null,
            })
            .is_unreachable()
        );
    }

    #[test]
    fn deadline_exceeded_is_retryable_not_unreachable() {
        let error = ConnectError::Rpc(crate::connect::TransportError::from(
            tonic::Status::deadline_exceeded("timed out"),
        ));
        assert!(error.is_retryable());
        assert!(!error.is_unreachable());
    }
}
