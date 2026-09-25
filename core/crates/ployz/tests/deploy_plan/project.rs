use super::support::*;
use ployz::deploy::{VolumeFate, plan_project_removal};
use ployz_core::{PruneRefusal, QualifiedService};

fn project() -> ProjectName {
    ProjectName::parse("app").unwrap()
}

#[test]
fn project_removal_deletes_visible_services_and_preserves_volumes() {
    let spec = requested(ServiceMode::Global);
    let snapshot = DeploySnapshot {
        machines: vec![machine('1', "first")],
        containers: vec![container('b', '1', &spec, &service_id('a'))],
        volume_snapshot: VolumeSnapshot::try_from_observations(vec![
            owned_volume(machine_id('1'), "data"),
            unowned_volume(machine_id('1'), "orphan"),
        ])
        .expect("valid Volume Snapshot fixture"),
        ..Default::default()
    };
    let plan = plan_project_removal(&project(), &snapshot, VolumeFate::Preserve).unwrap();
    assert_eq!(plan.prune_refusal, None);
    assert_eq!(
        plan.would_remove,
        [QualifiedService::parse("app/api").unwrap()]
    );
    assert!(
        plan.operations
            .iter()
            .any(|row| matches!(row.operation, DeployOperation::RemoveContainer { .. }))
    );
    assert!(
        plan.operations
            .iter()
            .all(|row| !matches!(row.operation, DeployOperation::RemoveVolume { .. }))
    );
    assert_eq!(plan.preserved_volumes.len(), 1);
    assert_eq!(
        plan.preserved_volumes
            .first()
            .expect("owned volume is preserved")
            .id
            .name
            .as_str(),
        "app_data"
    );
}

#[test]
fn incomplete_snapshot_refuses_removal_and_does_not_prune() {
    let spec = requested(ServiceMode::Global);
    let snapshot = DeploySnapshot {
        machines: vec![machine('1', "first"), machine('2', "second")],
        containers: vec![container('b', '1', &spec, &service_id('a'))],
        volume_snapshot: VolumeSnapshot::try_from_parts(
            vec![owned_volume(machine_id('1'), "data")],
            Vec::new(),
            Vec::new(),
            vec![machine_id('2')],
        )
        .expect("valid Volume Snapshot fixture"),
        ..Default::default()
    };
    let plan = plan_project_removal(&project(), &snapshot, VolumeFate::Destroy).unwrap();
    assert_eq!(plan.prune_refusal, Some(PruneRefusal::IncompleteSnapshot));
    assert!(plan.operations.is_empty());
    assert_eq!(
        plan.would_remove,
        [QualifiedService::parse("app/api").unwrap()]
    );
    assert_eq!(plan.preserved_volumes.len(), 1);
}

#[test]
fn destroying_volumes_emits_remove_volume_only_when_complete() {
    let volume = owned_volume(machine_id('1'), "data");
    let snapshot = DeploySnapshot {
        machines: vec![machine('1', "first")],
        volume_snapshot: VolumeSnapshot::try_from_observations(vec![volume.clone()])
            .expect("valid Volume Snapshot fixture"),
        ..Default::default()
    };
    let plan = plan_project_removal(&project(), &snapshot, VolumeFate::Destroy).unwrap();
    assert_eq!(plan.prune_refusal, None);
    assert_eq!(
        operations(&plan),
        [DeployOperation::RemoveVolume { id: volume.id }]
    );
    assert!(plan.preserved_volumes.is_empty());
}

#[test]
fn other_project_resources_are_left_alone() {
    let spec = requested(ServiceMode::Global);
    let mut other = container('c', '1', &spec, &service_id('c'));
    other
        .try_update(|parts| {
            parts.project_name = ProjectName::parse("other").unwrap();
            parts.resolved_spec.name = ServiceName::parse("web").unwrap();
        })
        .unwrap();
    let mut other_volume = owned_volume(machine_id('1'), "data");
    other_volume
        .labels
        .insert(PROJECT_NAME_LABEL.to_owned(), "other".to_owned());
    other_volume.id.name = ProjectName::parse("other")
        .unwrap()
        .volume_name(&DockerVolumeName::parse("data").unwrap());
    let snapshot = DeploySnapshot {
        machines: vec![machine('1', "first")],
        containers: vec![container('b', '1', &spec, &service_id('a')), other],
        volume_snapshot: VolumeSnapshot::try_from_observations(vec![
            owned_volume(machine_id('1'), "keep"),
            other_volume,
        ])
        .expect("valid Volume Snapshot fixture"),
        ..Default::default()
    };
    let plan = plan_project_removal(&project(), &snapshot, VolumeFate::Destroy).unwrap();
    assert_eq!(
        plan.would_remove,
        [QualifiedService::parse("app/api").unwrap()]
    );
    assert!(plan.operations.iter().all(|row| match &row.operation {
        DeployOperation::RemoveContainer { container_id, .. } => {
            *container_id == super::support::container_id('b')
        }
        DeployOperation::RemoveVolume { id } => id.name.as_str() == "app_keep",
        other @ (DeployOperation::WaitHealthy { .. }
        | DeployOperation::RunContainer { .. }
        | DeployOperation::StopContainer { .. }
        | DeployOperation::ReplaceContainer(_)
        | DeployOperation::StopHook { .. }
        | DeployOperation::PrepareVolumes { .. }
        | DeployOperation::RunHook { .. }) => panic!("unexpected operation: {other:?}"),
    }));
}
