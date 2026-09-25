use super::support::*;
#[test]
fn global_volume_is_created_only_on_machines_that_lack_it() {
    let mut requested = requested(ServiceMode::Global);
    add_named_volume(&mut requested, "data");
    for (existing_on, created_on) in [
        (vec![], vec!['1', '2']),
        (vec!['1'], vec!['2']),
        (vec!['1', '2'], vec![]),
    ] {
        let plan = plan_deploy(
            [&requested],
            &DeploySnapshot {
                machines: vec![machine('1', "first"), machine('2', "second")],
                volume_snapshot: VolumeSnapshot::try_from_observations(
                    existing_on
                        .iter()
                        .map(|id| observed_volume(machine_id(*id), "data")),
                )
                .expect("valid Volume Snapshot fixture"),
                ..Default::default()
            },
            PlanOptions::default(),
        )
        .unwrap();

        assert!(
            matches!(
                operations(&plan).as_slice(),
                [
                    DeployOperation::RunContainer { machine_id: first, .. },
                    DeployOperation::RunContainer { machine_id: second, .. },
                ] if *first == machine_id('1') && *second == machine_id('2')
            ),
            "existing on {existing_on:?}"
        );
        assert_eq!(
            plan.volumes_to_create
                .iter()
                .map(|item| item.machine_id)
                .collect::<Vec<_>>(),
            created_on.into_iter().map(machine_id).collect::<Vec<_>>(),
            "existing on {existing_on:?}"
        );
    }
}

#[test]
fn compatible_named_volume_aliases_and_repeated_mounts_create_once() {
    let mut requested = requested(ServiceMode::Global);
    add_named_volume(&mut requested, "data");
    let mut volumes = requested.volume_graph().volumes().to_vec();
    let mut mounts = requested.volume_graph().mounts().to_vec();
    mounts.push(ServiceMount {
        target: ContainerPath::parse("/data-copy").unwrap(),
        ..mounts.first().unwrap().clone()
    });
    let source = volumes.first().unwrap().source.clone();
    let alias = ServiceVolumeReference::parse("data-alias").unwrap();
    volumes.push(ServiceVolume {
        reference: alias.clone(),
        source,
    });
    mounts.push(ServiceMount {
        volume: alias,
        target: ContainerPath::parse("/alias").unwrap(),
        read_only: false,
        no_copy: false,
        subpath: None,
    });
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

    assert!(matches!(
        operations(&plan).as_slice(),
        [DeployOperation::RunContainer { .. }]
    ));
    assert_eq!(plan.volumes_to_create.len(), 1);
}

#[test]
fn unused_volume_definition_does_not_create_a_docker_volume() {
    let mut requested = requested(ServiceMode::Global);
    add_named_volume(&mut requested, "data");
    let mut volumes = requested.volume_graph().volumes().to_vec();
    let mounts = requested.volume_graph().mounts().to_vec();
    volumes.push(ServiceVolume {
        reference: ServiceVolumeReference::parse("logs").unwrap(),
        source: ployz_core::RawVolumeSource::Ordinary {
            name: DockerVolumeName::parse("logs").unwrap(),
            driver: ployz_core::VolumeDriver::parse("local", Default::default()).unwrap(),
            labels: Default::default(),
        }
        .admit()
        .expect("valid volume declaration"),
    });
    requested
        .set_volume_graph(ployz_core::ServiceVolumeGraph::parse(volumes, mounts).unwrap())
        .unwrap();

    let plan = plan_deploy(
        [&requested],
        &DeploySnapshot {
            machines: vec![machine('1', "first")],
            volume_snapshot: VolumeSnapshot::try_from_observations(vec![owned_volume(
                machine_id('1'),
                "logs",
            )])
            .expect("valid Volume Snapshot fixture"),
            ..Default::default()
        },
        PlanOptions::default(),
    )
    .unwrap();

    assert!(matches!(
        operations(&plan).as_slice(),
        [DeployOperation::RunContainer { .. }]
    ));
    assert_eq!(plan.volumes_to_create.len(), 1);
    assert_eq!(
        plan.volumes_to_create
            .first()
            .expect("missing managed Volume is previewed")
            .name
            .as_str(),
        "app_data"
    );
    assert_eq!(
        plan.preserved_volumes,
        vec![ployz_core::PreservedVolume {
            id: owned_volume(machine_id('1'), "logs").id,
            machine_name: Some(MachineName::parse("first").unwrap()),
        }]
    );
}

#[test]
fn project_scoping_rejects_incompatible_physical_volume_aliases() {
    let mut requested = requested(ServiceMode::Global);
    add_named_volume(&mut requested, "data");
    let mut volumes = requested.volume_graph().volumes().to_vec();
    let mounts = requested.volume_graph().mounts().to_vec();
    volumes.push(ServiceVolume {
        reference: ServiceVolumeReference::parse("external").unwrap(),
        source: ployz_core::RawVolumeSource::External {
            name: DockerVolumeName::parse("app_data").unwrap(),
        }
        .admit()
        .expect("valid volume declaration"),
    });
    requested
        .set_volume_graph(ployz_core::ServiceVolumeGraph::parse(volumes, mounts).unwrap())
        .unwrap();

    assert_eq!(
        plan_deploy(
            [&requested],
            &DeploySnapshot {
                machines: vec![machine('1', "first")],
                ..Default::default()
            },
            PlanOptions::default(),
        ),
        Err(PlanError::ConflictingDockerVolumeDefinitions {
            name: DockerVolumeName::parse("app_data").unwrap(),
        })
    );
}
