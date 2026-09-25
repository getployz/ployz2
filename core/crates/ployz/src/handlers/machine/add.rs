use clap::ArgMatches;
use ployz_core::{
    InspectRequest, JoinRequest, LocalMachinePhase, MachineName, RegisterRequest, op,
};

use super::super::{connect_client, runtime};
use super::{ConnectionOptions, helpers, target};
use crate::handlers::{Error, leaf_matches};

pub(in crate::handlers) fn add(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let policy = super::enrollment_policy(matches)?;
    let options = ConnectionOptions::from_matches(root)?;
    let (config, context_name) = options.active_config()?;
    let destination = target(matches, "destination")?;
    let connection = destination.parse()?;
    let mut connection = helpers::configure_ssh_key(
        connection,
        matches.get_one::<String>("ssh-key").map(String::as_str),
    )?;
    let requested_name = matches
        .get_one::<String>("name")
        .map(MachineName::parse)
        .transpose()?;
    let token_request = helpers::token_request(matches)?;
    let wireguard_mtu = matches.get_one::<u32>("wg-mtu").copied();
    let yes = matches.get_flag("yes");
    let storage = crate::provisioning::resolve_storage(matches)?;
    let no_install = matches.get_flag("no-install");
    let runtime = runtime()?;
    let assigned = runtime.block_on(async {
        if !no_install {
            crate::provisioning::provision(matches, storage).await?;
        }
        let mut entry = connect_client(matches, options.context()).await?;
        let snapshot = crate::enrollment::observe_enrollment(&mut entry).await?;
        let mut target_client = if no_install {
            helpers::connect_direct(matches, &connection).await?
        } else {
            helpers::reconnect_direct(matches, &connection).await?
        };
        let mut token = target_client
            .call_repeatable::<op::MachineToken>(token_request.clone(), None)
            .await?;
        let details = target_client
            .call_repeatable::<op::Inspect>(
                InspectRequest {
                    advertised_endpoints: token.advertised_endpoints.clone(),
                    ..Default::default()
                },
                None,
            )
            .await?;
        let history = config.path().with_extension("enrollment");
        let resuming = crate::enrollment::local::has_assignment(&history, &snapshot, token.id)
            .map_err(|error| Error::usage(error.to_string()))?
            || snapshot
                .machines
                .iter()
                .any(|machine| machine.id == token.id && machine.public_key == token.public_key);
        if details.phase != LocalMachinePhase::Uninitialized && !resuming {
            helpers::confirm(yes, "Reset the Machine before adding it to this Cluster?")?;
            helpers::reset(&mut target_client).await?;
            target_client = helpers::reconnect_direct(matches, &connection).await?;
            token = target_client
                .call_repeatable::<op::MachineToken>(token_request, None)
                .await?;
        }
        let name = helpers::machine_name(requested_name, &token)?;

        let assignment = crate::enrollment::local::save_assignment(
            &history,
            &RegisterRequest {
                machine_id: token.id,
                assigned_subnet: None,
                initial_policy: policy,
                name,
                storage,
                public_key: token.public_key,
                public_ip: token.public_ip,
                advertised_endpoints: token.advertised_endpoints,
                runtime: token.runtime,
            },
            &snapshot,
        )
        .map_err(|error| Error::usage(error.to_string()))?;
        let assigned = assignment.machine.clone();
        let registration = crate::enrollment::publish_enrollment(&mut entry, &assignment).await?;
        helpers::join(
            &mut target_client,
            JoinRequest {
                registration,
                wireguard_mtu,
            },
        )
        .await?;

        Ok::<_, Error>(assigned)
    })?;

    connection = connection.with_machine_id(assigned.id);
    config.save_connection(&context_name, connection.clone())?;
    println!("Added Machine {} ({})", assigned.name, assigned.id);

    runtime.block_on(helpers::wait_direct_participating(
        matches,
        &connection,
        "added Machine did not become ready",
    ))?;

    let catch_up = runtime.block_on(async {
        let mut entry = super::super::reconnect_client(matches, options.context()).await?;
        Ok::<_, Error>(crate::global_catch_up::catch_up_globals(&mut entry, &assigned).await)
    })?;
    if let Err(error) = catch_up {
        let recovery =
            super::super::recovery_command(matches, &context_name, &["ingress", "deploy"]);
        return Err(Error::usage(format!(
            "{}\nFor ingress, continue with: {recovery}",
            crate::global_catch_up::joined_catch_up_error(error)
        )));
    }
    let dns_result = runtime.block_on(async {
        let mut entry = super::super::reconnect_client(matches, options.context()).await?;
        crate::dns::update_records_if_reserved(&mut entry).await?;
        Ok::<_, Error>(())
    });
    if let Err(error) = dns_result {
        let recovery =
            super::super::recovery_command(matches, &context_name, &["ingress", "deploy"]);
        eprintln!("Machine joined; DNS publication pending. Continue with: {recovery}");
        eprintln!(
            "{}",
            Error::warned("hosted DNS refresh failed after adding the Machine", error)
        );
    }
    Ok(())
}
