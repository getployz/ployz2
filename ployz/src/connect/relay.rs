//! Cloud Relay Dial: wrap the opaque tunnel as a Channel.

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use hyper_util::rt::TokioIo;
use ployz_core::MachineId;
use ployz_relay::{
    AUTHORIZATION_METADATA, CloudRelayClient, DialCredential, MACHINE_ID_METADATA, TunnelIo,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request,
    metadata::MetadataValue,
    transport::{Channel, Endpoint},
};

use super::ConnectError;

pub(super) async fn connect_channel(
    url: &str,
    credential: &DialCredential,
    machine_id: &MachineId,
) -> Result<Channel, ConnectError> {
    let io = dial_tunnel(url, credential, machine_id).await?;
    let io = Arc::new(Mutex::new(Some(io)));
    Endpoint::from_static("http://[::]:50051")
        .connect_timeout(Duration::from_secs(5))
        .connect_with_connector(tower::service_fn(move |_| {
            let io = Arc::clone(&io);
            async move {
                io.lock()
                    .expect("relay tunnel mutex poisoned")
                    .take()
                    .ok_or_else(|| io::Error::other("relay tunnel already consumed"))
                    .map(TokioIo::new)
            }
        }))
        .await
        .map_err(ConnectError::from)
}

async fn dial_tunnel(
    url: &str,
    credential: &DialCredential,
    machine_id: &MachineId,
) -> Result<TunnelIo, ConnectError> {
    let channel = Endpoint::from_shared(url.to_owned())?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .map_err(ConnectError::from)?;
    let mut relay = CloudRelayClient::new(channel);
    let (tx, rx) = mpsc::channel(16);
    let mut request = Request::new(ReceiverStream::new(rx));
    insert_dial_metadata(request.metadata_mut(), credential, machine_id)?;
    let inbound = match relay.dial(request).await {
        Ok(response) => response.into_inner(),
        Err(status) => return Err(dial_status(status)),
    };
    Ok(TunnelIo::new(tx, inbound))
}

fn insert_dial_metadata(
    metadata: &mut tonic::metadata::MetadataMap,
    credential: &DialCredential,
    machine_id: &MachineId,
) -> Result<(), ConnectError> {
    let bearer = format!("Bearer {}", credential.as_str());
    metadata.insert(
        AUTHORIZATION_METADATA,
        MetadataValue::try_from(&bearer).map_err(|_| ConnectError::InvalidDialCredential)?,
    );
    metadata.insert(
        MACHINE_ID_METADATA,
        machine_id
            .as_str()
            .parse()
            .expect("Machine ID is ASCII metadata"),
    );
    Ok(())
}

fn dial_status(status: tonic::Status) -> ConnectError {
    if status.code() == tonic::Code::Unauthenticated {
        ConnectError::InvalidDialCredential
    } else if status.code() == tonic::Code::NotFound {
        ConnectError::UnknownMachine
    } else {
        ConnectError::from(status)
    }
}
