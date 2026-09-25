//! Deploy planning tests for managed and external Docker Volume identity.

use super::support::*;

#[test]
fn external_volume_keeps_its_identity_without_a_create_preview() {
    let mut requested = requested(ServiceMode::Global);
    add_named_volume(&mut requested, "shared");
    let mut volumes = requested.volume_graph().volumes().to_vec();
    let mounts = requested.volume_graph().mounts().to_vec();
    let volume = volumes.first_mut().expect("named volume was added");
    volume.source = ployz_core::RawVolumeSource::External {
        name: DockerVolumeName::parse("shared").unwrap(),
    }
    .admit()
    .expect("valid volume declaration");
    requested
        .set_volume_graph(ployz_core::ServiceVolumeGraph::parse(volumes, mounts).unwrap())
        .unwrap();
    let plan = plan_deploy(
        [&requested],
        &DeploySnapshot {
            machines: vec![machine('1', "first")],
            ..Default::default()
        },
        PlanOptions::default(),
    )
    .unwrap();
    let plan_operations = operations(&plan);
    let [DeployOperation::RunContainer { spec, .. }] = plan_operations.as_slice() else {
        panic!("expected one run operation: {plan_operations:?}");
    };
    let operation_volume = spec
        .volume_graph()
        .volumes()
        .first()
        .expect("run operation mounts the external Volume");
    assert!(matches!(
        operation_volume.source.kind(),
        ployz_core::RawVolumeSource::External { name } if name.as_str() == "shared"
    ));
    assert!(plan.volumes_to_create.is_empty());
}
