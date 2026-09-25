use std::num::NonZeroU64;

use ployz_core::{
    ContainerId, DeployOperation, DockerVolumeId, DockerVolumeName, MachineId, MachineName,
    OperationRow, PreservedVolume, ProjectName, ProvisionedVolumeMaximumBytes, PruneRefusal,
    QualifiedService, ReplacementOperation, RequestedServiceSpec, ResolvedServiceSpec, ServiceName,
    UpdateOrder, VolumeToCreate,
};

use super::*;

#[test]
fn empty_preview_prints_no_changes_without_a_prompt_body() {
    let preview = DeployPreview::new(Vec::new(), Vec::new(), ProjectName::parse("app").unwrap());
    assert_eq!(plan_text(&preview, "default"), "No changes.\n");
    assert_eq!(
        confirm_prompt("default"),
        "Proceed with deployment to default? [y/N] "
    );
}

#[test]
fn removal_plan_lists_container_and_volume_removes() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let container_id = ContainerId::parse("f".repeat(64)).unwrap();
    let volume_id = DockerVolumeId {
        machine_id,
        name: DockerVolumeName::parse("shop_data").unwrap(),
    };
    let rows = vec![
        OperationRow::pending(
            0,
            DeployOperation::RemoveContainer {
                machine_id,
                container_id,
            },
            Some(MachineName::parse("edge").unwrap()),
            Some("web-1".into()),
            Some(ServiceName::parse("web").unwrap()),
        ),
        OperationRow::pending(
            1,
            DeployOperation::RemoveVolume {
                id: volume_id.clone(),
            },
            Some(MachineName::parse("edge").unwrap()),
            None,
            None,
        ),
    ];
    let mut preview = DeployPreview::new(rows, Vec::new(), ProjectName::parse("shop").unwrap());
    preview.would_remove = vec![QualifiedService::parse("shop/web").unwrap()];
    let text = removal_plan_text(&preview, "default");
    assert!(text.starts_with("Removal plan\n"), "{text}");
    assert!(text.contains("- remove container web-1 on edge"), "{text}");
    assert!(text.contains("- remove volume shop_data on edge"), "{text}");
    preview.operations.clear();
    preview.preserved_volumes = vec![PreservedVolume {
        id: volume_id,
        machine_name: Some(MachineName::parse("edge").unwrap()),
    }];
    let preserved = removal_plan_text(&preview, "default");
    assert!(
        preserved.contains("would preserve volume shop_data on edge"),
        "{preserved}"
    );
}

#[test]
fn service_tree_keeps_volume_and_service_ordering() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let machine = Some(MachineName::parse("edge").unwrap());
    let container = |index, service: &str, display: &str, id: char| {
        OperationRow::pending(
            index,
            DeployOperation::StopContainer {
                machine_id,
                container_id: ContainerId::parse(id.to_string().repeat(64)).unwrap(),
                purpose: ployz_core::StopContainerPurpose::Lifecycle,
            },
            machine.clone(),
            Some(display.into()),
            Some(ServiceName::parse(service).unwrap()),
        )
    };
    let volume = |index, name: &str| {
        OperationRow::pending(
            index,
            DeployOperation::RemoveVolume {
                id: DockerVolumeId {
                    machine_id,
                    name: DockerVolumeName::parse(name).unwrap(),
                },
            },
            machine.clone(),
            None,
            None,
        )
    };
    let rows = vec![
        container(0, "web", "web-old", '1'),
        volume(1, "z"),
        container(2, "api", "api-old", '2'),
        volume(3, "a"),
        container(4, "web", "web-new", '3'),
    ];
    let preview = DeployPreview::new(rows, Vec::new(), ProjectName::parse("app").unwrap());

    assert_eq!(
        service_trees(&preview),
        concat!(
            "- remove volume z on edge\n",
            "- remove volume a on edge\n",
            "~ update service api\n",
            "  ╰── - stop container api-old on edge\n",
            "~ update service web\n",
            "  ├── - stop container web-old on edge\n",
            "  ╰── - stop container web-new on edge\n",
        )
    );
}

#[test]
fn plan_identifies_a_provisioned_volume_and_its_bound() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let mut preview =
        DeployPreview::new(Vec::new(), Vec::new(), ProjectName::parse("shop").unwrap());
    preview.volumes_to_create = vec![VolumeToCreate {
        machine_id,
        machine_name: Some(MachineName::parse("edge").unwrap()),
        name: DockerVolumeName::parse("data").unwrap(),
        maximum_bytes: Some(ProvisionedVolumeMaximumBytes::new(
            NonZeroU64::new(1_073_741_824).unwrap(),
        )),
    }];

    let text = plan_text(&preview, "default");

    assert!(text.contains("Volumes to create\n"), "{text}");
    assert!(
        text.contains("+ provisioned volume data (maximum 1073741824 bytes) on edge"),
        "{text}"
    );
}

#[test]
fn replace_plan_matches_tree_shape() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let old = ContainerId::parse("f".repeat(64)).unwrap();
    let spec = resolved("excalidraw", "excalidraw/excalidraw:latest");
    let row = OperationRow::pending(
        0,
        DeployOperation::ReplaceContainer(ReplacementOperation {
            machine_id,
            old_container_id: old,
            spec,
            skip_health_monitor: false,
        }),
        Some(MachineName::parse("machine-dc3c").unwrap()),
        Some("excalidraw/fde7ac7f11ad".into()),
        Some(ServiceName::parse("excalidraw").unwrap()),
    );
    let preview = DeployPreview::new(vec![row], Vec::new(), ProjectName::parse("app").unwrap());
    let text = plan_text(&preview, "default");
    assert!(text.contains("Deployment plan\ncontext: default\nproject: app\n"));
    assert!(text.contains("~ update service excalidraw\n"));
    assert!(text.contains("  │   image: excalidraw/excalidraw:latest\n"));
    assert!(text.contains("  ╰── +/- replace container excalidraw/fde7ac7f11ad on machine-dc3c\n"));
    assert!(text.contains("1 replace (start-first) · across 1 machine\n"));
}

#[test]
fn plan_shows_dependency_health_wait() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let row = OperationRow::pending(
        0,
        DeployOperation::WaitHealthy {
            machine_id,
            dependent: QualifiedService::parse("app/web").unwrap(),
            dependency: QualifiedService::parse("app/db").unwrap(),
        },
        Some(MachineName::parse("edge").unwrap()),
        None,
        Some(ServiceName::parse("web").unwrap()),
    );
    let preview = DeployPreview::new(vec![row], Vec::new(), ProjectName::parse("app").unwrap());

    assert!(
        plan_text(&preview, "default").contains("~ wait for app/db to be healthy before app/web")
    );
}

#[test]
fn plan_lists_would_remove_with_observer_relative_refusal() {
    let mut preview =
        DeployPreview::new(Vec::new(), Vec::new(), ProjectName::parse("shop").unwrap());
    preview.would_remove = vec![QualifiedService::parse("shop/debug").unwrap()];
    preview.prune_refusal = Some(PruneRefusal::IncompleteSnapshot);
    let text = plan_text(&preview, "default");
    assert!(text.contains("project: shop\n"), "{text}");
    assert!(text.contains("would remove shop/debug"), "{text}");
    assert!(
        text.contains("incomplete relative to this Machine's current visible fan-out"),
        "{text}"
    );
    assert!(!text.contains("authoritative"));
    assert!(!text.contains("No changes."));
}

#[test]
fn removed_containers_print_under_a_service_marker_only_when_the_service_is_pruned() {
    for (service, would_remove, marker, absent) in [
        (
            "debug",
            true,
            "- remove service debug\n",
            "~ update service debug",
        ),
        (
            "web",
            false,
            "~ update service web\n",
            "- remove service web",
        ),
    ] {
        let row = OperationRow::pending(
            0,
            DeployOperation::RemoveContainer {
                machine_id: MachineId::parse("d".repeat(32)).unwrap(),
                container_id: ContainerId::parse("f".repeat(64)).unwrap(),
            },
            Some(MachineName::parse("machine-dc3c").unwrap()),
            Some(format!("{service}/fde7ac7f11ad")),
            Some(ServiceName::parse(service).unwrap()),
        );
        let mut preview =
            DeployPreview::new(vec![row], Vec::new(), ProjectName::parse("shop").unwrap());
        if would_remove {
            preview.would_remove =
                vec![QualifiedService::parse(format!("shop/{service}")).unwrap()];
        }
        let text = plan_text(&preview, "default");
        assert!(text.contains(marker), "{text}");
        assert!(!text.contains(absent), "{text}");
        assert!(
            text.contains(&format!(
                "- remove container {service}/fde7ac7f11ad on machine-dc3c"
            )),
            "{text}"
        );
        assert!(text.contains("1 remove · across 1 machine"), "{text}");
        assert!(!text.contains("would remove"), "{text}");
        assert!(!text.contains("will not remove"), "{text}");
    }
}

fn resolved(name: &str, image: &str) -> ResolvedServiceSpec {
    let requested: RequestedServiceSpec = serde_json::from_value(serde_json::json!({
        "name": name,
        "mode": { "mode": "replicated", "replicas": 1 },
        "container": { "image": image, "pull_policy": "missing" }
    }))
    .unwrap();
    requested
        .to_resolved(
            ployz_core::ServiceId::parse("a".repeat(32)).unwrap(),
            ployz_core::ResolvedUpdateConfig {
                order: UpdateOrder::StartFirst,
                monitor_millis: None,
            },
        )
        .expect("volume graph is scoped")
}
