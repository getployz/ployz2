//! Cloud Pairing and the management client key it admits, as one admitted mutation.

use ployz_core::{
    LocalMachinePhase, ManagementCapability, SetCloudPairingRequest, SetCloudPairingResponse,
};

use super::{Error, LocalMachine};

impl LocalMachine {
    /// Set or clear Cloud Pairing under mutation admission.
    ///
    /// `Set` persists the pairing, mints a fresh client key, persists only its public
    /// half and returns the full Management Capability. Because the client secret is
    /// never stored, a repeated `Set` (same pairing or not) mints a new key and cuts
    /// every holder of the previous one; the caller still receives a working capability.
    ///
    /// `Clear` persists pairing and accepted key cleared in one write. The record
    /// watch publishes that change, and the management transport closes every live
    /// connection of the old key in response.
    ///
    /// # Errors
    /// Returns [`Error::NotParticipating`] when this Machine is not participating,
    /// admission failures, [`Error::RecordOwner`] when the record owner has stopped,
    /// and [`Error::Store`] when the record cannot be written.
    pub async fn set_cloud_pairing(
        &self,
        request: SetCloudPairingRequest,
    ) -> Result<SetCloudPairingResponse, Error> {
        let local = self.clone();
        self.finish_mutation(async move {
            local
                .owner
                .mutate(move |store| {
                    if store.record().phase() != LocalMachinePhase::Participating {
                        return Err(Error::NotParticipating);
                    }
                    let capability = match request {
                        SetCloudPairingRequest::Set { pairing } => {
                            let client = iroh::SecretKey::generate();
                            store.persist_cloud_pairing(
                                Some(pairing),
                                Some(*client.public().as_bytes()),
                            )?;
                            Some(ManagementCapability::new(
                                store.record().management_secret().public_key(),
                                client.to_bytes(),
                            ))
                        }
                        SetCloudPairingRequest::Clear {} => {
                            store.persist_cloud_pairing(None, None)?;
                            None
                        }
                    };
                    Ok(SetCloudPairingResponse { capability })
                })
                .await?
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{LocalMachineStore, RecordOwner};
    use ployz_core::CloudPairing;

    async fn participating(dir: &std::path::Path) -> LocalMachine {
        let store = LocalMachineStore::open(dir).unwrap();
        let local = LocalMachine::new(RecordOwner::spawn(store).unwrap());
        local
            .initialize(ployz_core::InitializeRequest {
                initial_policy: Default::default(),
                name: ployz_core::MachineName::parse("first").unwrap(),
                cluster_network: "10.210.0.0/16".parse().unwrap(),
                public_ip: None,
                advertised_endpoints: vec![ployz_core::AdvertisedEndpoint(
                    "192.0.2.1:51820".parse().unwrap(),
                )],
                wireguard_mtu: None,
                cloud_pairing: None,
            })
            .await
            .unwrap();
        local
    }

    fn pairing(secret: &str) -> CloudPairing {
        CloudPairing::new(ployz_core::PairingCredential::parse(secret).unwrap())
    }

    #[tokio::test]
    async fn set_returns_a_capability_and_persists_only_the_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let local = participating(dir.path()).await;
        let response = local
            .set_cloud_pairing(SetCloudPairingRequest::Set {
                pairing: pairing("pairing"),
            })
            .await
            .unwrap();
        let capability = response.capability.unwrap();
        let record = local.record();
        assert_eq!(record.cloud_pairing, Some(pairing("pairing")));
        assert_eq!(
            *capability.machine(),
            record.management_secret().public_key()
        );
        let client_public = iroh::SecretKey::from_bytes(capability.client_secret()).public();
        assert_eq!(record.accepted_client, Some(*client_public.as_bytes()));
        let persisted = std::fs::read_to_string(dir.path().join("machine.json")).unwrap();
        assert!(!persisted.contains(&capability.to_secret_string()));
        assert!(!persisted.contains(&serde_json::to_string(capability.client_secret()).unwrap()));

        // A repeated Set mints a new key: the old capability no longer matches the record.
        let again = local
            .set_cloud_pairing(SetCloudPairingRequest::Set {
                pairing: pairing("pairing"),
            })
            .await
            .unwrap();
        assert_ne!(again.capability.unwrap(), capability);
        assert_ne!(
            local.record().accepted_client,
            Some(*client_public.as_bytes())
        );
    }

    #[tokio::test]
    async fn clear_removes_pairing_and_accepted_key_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let local = participating(dir.path()).await;
        local
            .set_cloud_pairing(SetCloudPairingRequest::Set {
                pairing: pairing("pairing"),
            })
            .await
            .unwrap();
        let mut records = local.owner().watch();
        records.mark_unchanged();
        let response = local
            .set_cloud_pairing(SetCloudPairingRequest::Clear {})
            .await
            .unwrap();
        assert_eq!(response, SetCloudPairingResponse { capability: None });
        assert!(records.has_changed().unwrap());
        let record = records.borrow_and_update().clone();
        assert_eq!(record.cloud_pairing, None);
        assert_eq!(record.accepted_client, None);
        // One publication means one write: pairing and key were not cleared separately.
        assert!(!records.has_changed().unwrap());
    }
}
