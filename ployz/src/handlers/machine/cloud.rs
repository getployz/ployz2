//! `ployz init --cloud`: local Initialize from an enroll `initialize` response.

use clap::ArgMatches;
use ployz_core::{
    CloudEnrollToken, InitializeRequest, InspectRequest, LocalMachinePhase, MachineName,
    MachineTokenRequest, ResetRequest, op,
};

use super::super::runtime;
use super::{ConnectionOptions, helpers};
use crate::{
    connect::DEFAULT_LOCAL_SOCKET,
    context::{Context, Transport},
    enroll::{self, EnrollIdentity, EnrollResponse},
    handlers::{Error, leaf_matches},
};

/// Initialise this Machine from Cloud enroll `initialize` + Cloud Pairing.
///
/// # Errors
///
/// Returns [`Error`] when `--cloud` is missing, `--connect` is SSH, enroll
/// fails or is not `initialize`, or local Initialize fails.
pub(in crate::handlers) fn init(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let Some(token) = matches.get_one::<String>("cloud") else {
        return Err(Error::usage(
            "local machine initialisation is not implemented; pass --cloud",
        ));
    };
    let token = CloudEnrollToken::parse(token.clone())?;
    let options = ConnectionOptions::from_matches(root)?;
    let mut config = options.load_or_empty_config()?;
    let context_name = matches
        .get_one::<String>("context")
        .cloned()
        .unwrap_or_else(|| "default".into());
    if config.contexts.contains_key(&context_name) {
        return Err(Error::usage(format!(
            "context {context_name:?} already exists"
        )));
    }
    let connection = match matches.get_one::<String>("connect") {
        Some(direct) => {
            let connection: crate::context::Connection = direct.parse()?;
            match connection.transport() {
                Transport::Tcp(_) | Transport::Unix(_) => connection,
                Transport::Ssh { .. } | Transport::Relay { .. } => {
                    return Err(Error::usage(
                        "init --cloud talks to local ployzd; do not use an SSH destination",
                    ));
                }
            }
        }
        None => crate::context::Connection::unix(DEFAULT_LOCAL_SOCKET)?,
    };
    let requested_name = matches
        .get_one::<String>("name")
        .map(MachineName::parse)
        .transpose()?;
    let cluster_network = matches
        .get_one::<String>("network")
        .expect("Cluster network has a default")
        .parse()
        .map_err(|error| Error::usage(format!("invalid Cluster network: {error}")))?;
    let wireguard_mtu = matches.get_one::<u32>("wg-mtu").copied();
    let yes = matches.get_flag("yes");
    let cloud_url = matches
        .get_one::<String>("cloud-url")
        .map(String::as_str)
        .unwrap_or(enroll::DEFAULT_CLOUD_HOST);
    let enroll_url = enroll::enroll_url(cloud_url, &token)?;

    let (machine, connection) = runtime()?.block_on(async {
        let mut target = helpers::connect_direct(&connection).await?;
        let mut token_response = target
            .call::<op::MachineToken>(MachineTokenRequest::default(), None)
            .await?;
        let details = target
            .call::<op::Inspect>(InspectRequest::default(), None)
            .await?;
        if details.phase != LocalMachinePhase::Uninitialized {
            helpers::confirm(yes, "Reset the Machine before initialising a new Cluster?")?;
            target.call::<op::Reset>(ResetRequest {}, None).await?;
            target = helpers::reconnect_direct(&connection).await?;
            token_response = target
                .call::<op::MachineToken>(MachineTokenRequest::default(), None)
                .await?;
        }
        let name = helpers::machine_name(requested_name, &token_response)?;
        let response = enroll::post(
            enroll_url,
            &EnrollIdentity::new(name.clone(), &token_response),
        )
        .await?;
        let pairing = match response {
            EnrollResponse::Initialize { pairing } => pairing,
            EnrollResponse::Join {} => {
                return Err(Error::usage("enroll join is not implemented"));
            }
            EnrollResponse::NotYet { .. } => {
                return Err(Error::usage("enroll is not ready"));
            }
        };
        let machine = target
            .call::<op::Initialize>(
                InitializeRequest {
                    name,
                    cluster_network,
                    public_ip: token_response.public_ip,
                    advertised_endpoints: token_response.advertised_endpoints,
                    wireguard_mtu,
                    cloud_pairing: Some(pairing),
                },
                None,
            )
            .await?
            .machine;
        let connection = connection.with_machine_id(machine.id);
        Ok::<_, Error>((machine, connection))
    })?;

    config.set_current_context(Some(context_name.clone()));
    config.contexts.insert(
        context_name,
        Context {
            connections: vec![connection.clone()],
        },
    );
    config.save()?;
    if !matches.get_flag("no-caddy") {
        runtime()?.block_on(async {
            let mut ready = helpers::wait_direct_participating(&connection).await?;
            let image = crate::caddy::latest_image().await?;
            let requested = crate::caddy::service_spec(image, Vec::new(), None);
            crate::deploy::apply_requested(&mut ready, &requested).await?;
            Ok::<_, Error>(())
        })?;
    }
    println!("Initialised Machine {} ({})", machine.name, machine.id);
    Ok(())
}
