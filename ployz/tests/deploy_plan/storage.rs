//! Provisioned storage admission through the public planner and SDK error contract.
use super::support::*;
use ployz_core::{STORAGE_GIB, StorageBacking, StorageCapacity};

fn capacity(free_gib: u64) -> StorageCapacity {
    StorageCapacity {
        backing: StorageBacking::Unallocated {
            host_total_bytes: 80 * STORAGE_GIB,
            host_available_bytes: free_gib * STORAGE_GIB,
        },
        unmanaged_used_bytes: 0,
        volumes: BTreeMap::new(),
    }
}

fn intent() -> DeployIntent {
    let services =
        [("postgres", 10), ("redis", 4), ("data", 30), ("server", 8)].map(|(name, gib)| {
            let mut service = spec(name);
            add_named_volume(&mut service, name);
            make_provisioned(&mut service, name, gib * STORAGE_GIB);
            service
        });
    DeployIntent::apply_all(
        ProjectName::parse("app").unwrap(),
        services.iter(),
        PlanOptions::default(),
    )
}

#[test]
fn complete_plan_reports_all_four_volumes_before_any_deployment() {
    let mut target = machine('1', "machine1");
    target.storage = Some(ployz_core::MachineStorageObservation::Ready);
    let snapshot = DeploySnapshot {
        storage_capacity: BTreeMap::from([(target.machine.id, Ok(capacity(60)))]),
        machines: vec![target],
        ..Default::default()
    };
    let error = preview_deploy(&intent(), &snapshot, IngressContext::default())
        .unwrap_err()
        .into_rpc_error();
    assert_eq!(error.details.get("code").unwrap(), "insufficient_storage");
    assert_eq!(error.details.get("machine").unwrap(), "machine1");
    assert_eq!(
        error.details.get("required_growth_bytes").unwrap(),
        62_491_774_156u64
    );
    assert_eq!(
        error.details.get("shortfall_bytes").unwrap(),
        19_542_101_196u64
    );
    assert!(error.message.contains("18.20 GiB"));
}

#[test]
fn placement_uses_an_alternative_and_unknown_capacity_holds() {
    let mut first = machine('1', "first");
    let mut second = machine('2', "second");
    first.storage = Some(ployz_core::MachineStorageObservation::Ready);
    second.storage = first.storage;
    let mut snapshot = DeploySnapshot {
        storage_capacity: BTreeMap::from([
            (first.machine.id, Ok(capacity(20))),
            (second.machine.id, Ok(capacity(80))),
        ]),
        machines: vec![first, second],
        ..Default::default()
    };
    let preview = preview_deploy(&intent(), &snapshot, IngressContext::default()).unwrap();
    assert_eq!(preview.storage.len(), 1);
    assert_eq!(
        preview.storage.first().unwrap().machine_name.as_str(),
        "second"
    );
    snapshot.storage_capacity.clear();
    let error = preview_deploy(&intent(), &snapshot, IngressContext::default())
        .unwrap_err()
        .into_rpc_error();
    assert_eq!(
        error.details.get("code").unwrap(),
        "storage_capacity_unknown"
    );
}

#[test]
fn surviving_datasets_anchor_single_and_shared_services_without_docker_metadata() {
    for names in [vec!["api"], vec!["api", "worker"]] {
        let mut empty = machine('1', "empty");
        let mut owner = machine('2', "owner");
        empty.storage = Some(ployz_core::MachineStorageObservation::Ready);
        owner.storage = empty.storage;
        let mut existing = capacity(80);
        existing.backing = StorageBacking::Fixed {
            pool_size_bytes: 100 * STORAGE_GIB,
        };
        existing.volumes.insert(
            app_volume("data"),
            ProvisionedVolumeMaximumBytes::new(std::num::NonZeroU64::new(STORAGE_GIB).unwrap()),
        );
        let snapshot = DeploySnapshot {
            storage_capacity: BTreeMap::from([
                (empty.machine.id, Ok(capacity(80))),
                (owner.machine.id, Ok(existing)),
            ]),
            machines: vec![empty, owner],
            ..Default::default()
        };
        let mut services = names
            .iter()
            .map(|name| {
                let mut service = spec(name);
                add_named_volume(&mut service, "data");
                make_provisioned(&mut service, "data", STORAGE_GIB);
                service
            })
            .collect::<Vec<_>>();
        let intent = |services: &[RequestedServiceSpec]| {
            DeployIntent::apply_all(
                ProjectName::parse("app").unwrap(),
                services.iter(),
                PlanOptions::default(),
            )
        };
        let preview =
            preview_deploy(&intent(&services), &snapshot, IngressContext::default()).unwrap();
        assert!(
            preview
                .operations
                .iter()
                .all(|row| row.machine_name.as_ref().unwrap().as_str() == "owner"),
            "{preview:?}"
        );
        assert_eq!(
            preview
                .storage
                .first()
                .unwrap()
                .budget
                .additional_commitment_bytes,
            0
        );
        let mut conflicting = snapshot.clone();
        let owner = conflicting.machines.last().unwrap().machine.id;
        conflicting
            .storage_capacity
            .get_mut(&owner)
            .unwrap()
            .as_mut()
            .unwrap()
            .volumes
            .insert(
                app_volume("data"),
                ProvisionedVolumeMaximumBytes::new(
                    std::num::NonZeroU64::new(2 * STORAGE_GIB).unwrap(),
                ),
            );
        let error = preview_deploy(&intent(&services), &conflicting, IngressContext::default())
            .unwrap_err()
            .into_rpc_error();
        assert_eq!(error.details.get("code").unwrap(), "volume_size_conflict");
        for service in &mut services {
            service.placement.machines = vec![MachineTarget::parse("empty").unwrap()];
        }
        assert!(
            preview_deploy(&intent(&services), &snapshot, IngressContext::default()).is_err(),
            "must not create empty data on a different Machine"
        );
    }
}
