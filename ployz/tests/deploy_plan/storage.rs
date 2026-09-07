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
