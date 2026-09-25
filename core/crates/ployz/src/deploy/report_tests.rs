use ployz_core::{
    ContainerId, DeployOperation, DockerVolumeId, DockerVolumeName, ExecutionError,
    FailedOperation, HealthFailure, HookFailure, MachineAction, MachineId, MachineName,
    OperationPhase, OperationRow, OperationStatus, QualifiedService, ReplacementCompensation,
    ReplacementOperation, RequestedServiceSpec, ResolvedServiceSpec, RestartAttempt, RpcError,
    RpcErrorCode, ServiceName, StopAttempt, UpdateOrder,
};

use super::*;

const PLAIN: Ink = Ink::plain();
const COLOR: Ink = Ink { color: true };

#[test]
fn progress_snapshot_prints_healthy_elapsed_removed_and_volume_rows() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let machine = Some(MachineName::parse("machine-dc3c").unwrap());
    let spec = resolved("excalidraw", "excalidraw/excalidraw:latest");
    let healthy = OperationRow {
        index: 0,
        machine_id,
        machine_name: machine.clone(),
        operation: DeployOperation::RunContainer {
            machine_id,
            spec,
            skip_health_monitor: false,
        },
        display_name: Some("excalidraw-0z12".into()),
        service_name: Some(ServiceName::parse("excalidraw").unwrap()),
        status: OperationStatus::Running {
            phase: OperationPhase::WaitingForHealth {
                container_id: ContainerId::parse("a".repeat(64)).unwrap(),
                health: None,
                elapsed_ms: 30_600,
                deadline_ms: 60_000,
            },
        },
    };
    let removed = OperationRow {
        index: 1,
        machine_id,
        machine_name: machine,
        operation: DeployOperation::RemoveContainer {
            machine_id,
            container_id: ContainerId::parse("f".repeat(64)).unwrap(),
        },
        display_name: Some("excalidraw/fde7ac7f11ad".into()),
        service_name: Some(ServiceName::parse("excalidraw").unwrap()),
        status: OperationStatus::Completed,
    };
    let volume = OperationRow::pending(
        2,
        DeployOperation::RemoveVolume {
            id: DockerVolumeId {
                machine_id,
                name: DockerVolumeName::parse("cashdash_data").unwrap(),
            },
        },
        Some(MachineName::parse("vultr").unwrap()),
        None,
        None,
    );
    let text = paint_live(
        "Deploying to default",
        2,
        3,
        &[healthy, removed, volume],
        &PLAIN,
    );
    assert!(text.contains("[+] Deploying to default 2/3\n"), "{text}");
    assert!(text.contains("Container excalidraw-0z12 on machine-dc3c"));
    assert!(text.contains("waiting for health"));
    assert!(!text.contains("Healthy"));
    assert!(text.contains("30.6s"));
    assert!(text.contains("Container excalidraw/fde7ac7f11ad on machine-dc3c"));
    assert!(text.contains("Removed"));
    assert!(text.contains("Volume cashdash_data on vultr"), "{text}");
    assert!(!text.contains("Container cashdash_data"), "{text}");
}

#[test]
fn failed_progress_row_cause_is_english_without_ids_or_debug() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let replace = |service: &str| {
        DeployOperation::ReplaceContainer(ReplacementOperation {
            machine_id,
            old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
            spec: resolved(service, "app:latest"),
            skip_health_monitor: false,
        })
    };
    let hook = DeployOperation::RunHook {
        machine_id,
        spec: resolved("migrate", "app:latest"),
        old_hook_containers: Vec::new(),
    };
    for (operation, service, machine, error, expected, forbidden) in [
        (
            replace("cashdash-frontend"),
            None,
            "machine-2",
            timed_out_create(),
            vec![
                "Container cashdash-frontend on machine-2",
                "create failed: target Machine RPC timed out",
            ],
            vec!["CreateContainer".to_owned(), "f".repeat(64), "d".repeat(32)],
        ),
        (
            replace("cashdash-frontend"),
            Some("cashdash-frontend"),
            "machine-2",
            health_timeout(ContainerId::parse("c".repeat(64)).unwrap()),
            vec!["health check timed out"],
            vec!["TimedOut".to_owned(), "c".repeat(64)],
        ),
        (
            hook,
            Some("migrate"),
            "edge",
            ExecutionError::Hook {
                container_id: ContainerId::parse("b".repeat(64)).unwrap(),
                failure: HookFailure::Exit { code: 1 },
            },
            vec!["pre-deploy hook exited 1"],
            vec!["Exit {".to_owned(), "b".repeat(64)],
        ),
    ] {
        let row = OperationRow {
            index: 0,
            machine_id,
            machine_name: Some(MachineName::parse(machine).unwrap()),
            operation,
            display_name: None,
            service_name: service.map(|name| ServiceName::parse(name).unwrap()),
            status: OperationStatus::Failed { error },
        };
        let text = paint_live("Deploying to default", 0, 1, &[row], &PLAIN);
        for needle in expected {
            assert!(text.contains(needle), "{needle}: {text}");
        }
        for needle in forbidden {
            assert!(!text.contains(&needle), "{needle}: {text}");
        }
    }
}

#[test]
fn failed_deploy_footer_names_the_service_and_error_without_a_hash_dump() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let failed = DeployOperation::ReplaceContainer(ReplacementOperation {
        machine_id,
        old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
        spec: resolved("cashdash-frontend", "app:latest"),
        skip_health_monitor: false,
    });
    let unexecuted = vec![
        DeployOperation::ReplaceContainer(ReplacementOperation {
            machine_id,
            old_container_id: ContainerId::parse("a".repeat(64)).unwrap(),
            spec: resolved("cashdash-horizon", "app:latest"),
            skip_health_monitor: false,
        }),
        DeployOperation::RunContainer {
            machine_id,
            spec: resolved("cashdash-web", "app:latest"),
            skip_health_monitor: false,
        },
    ];
    let outcome = DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation: failed,
            error: timed_out_create(),
        },
        unexecuted,
    };
    let text = paint_closing(&outcome, &[], false, &PLAIN);
    assert!(
        text.contains("• Container cashdash-horizon  Unexecuted"),
        "{text}"
    );
    assert!(
        text.contains("• Container cashdash-web  Unexecuted"),
        "{text}"
    );
    let footer = text
        .split_once("Failed:")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    assert!(
        footer.contains("replace cashdash-frontend\n  create failed: target Machine RPC timed out"),
        "{text}"
    );
    assert!(!footer.contains("cashdash-horizon"), "{text}");
    assert!(!footer.contains("cashdash-web"), "{text}");
    assert!(!text.contains("Completed"), "{text}");
    assert!(!text.contains(&"d".repeat(32)), "{text}");
    assert!(!text.contains("next: ployz service logs"), "{text}");
}

#[test]
fn failed_deploy_list_shows_completed_operations_before_the_create_footer() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let completed = DeployOperation::RunContainer {
        machine_id,
        spec: resolved("cashdash-reverb", "app:latest"),
        skip_health_monitor: true,
    };
    let failed = DeployOperation::RunContainer {
        machine_id,
        spec: resolved("cashdash-web", "app:latest"),
        skip_health_monitor: true,
    };
    let outcome = DeployOutcome::Failed {
        completed: vec![completed],
        failed: FailedOperation::Operation {
            operation: failed,
            error: timed_out_create(),
        },
        unexecuted: Vec::new(),
    };
    let text = paint_closing(&outcome, &[], false, &PLAIN);
    assert!(
        text.contains("✔ Container cashdash-reverb  Healthy"),
        "{text}"
    );
    assert!(
        text.contains("Failed: create cashdash-web\n  create failed: target Machine RPC timed out"),
        "{text}"
    );
}

#[test]
fn failed_footer_names_machine_from_live_rows() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let spec = resolved("cashdash-frontend", "app:latest");
    let failed = DeployOperation::ReplaceContainer(ReplacementOperation {
        machine_id,
        old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
        spec,
        skip_health_monitor: false,
    });
    let web = DeployOperation::RunContainer {
        machine_id,
        spec: resolved("cashdash-web", "app:latest"),
        skip_health_monitor: false,
    };
    let volume = DeployOperation::RemoveVolume {
        id: DockerVolumeId {
            machine_id,
            name: DockerVolumeName::parse("cashdash_data").unwrap(),
        },
    };
    let rows = vec![
        OperationRow {
            index: 0,
            machine_id,
            machine_name: Some(MachineName::parse("machine-2").unwrap()),
            operation: failed.clone(),
            display_name: None,
            service_name: Some(ServiceName::parse("cashdash-frontend").unwrap()),
            status: OperationStatus::Failed {
                error: timed_out_create(),
            },
        },
        OperationRow {
            index: 1,
            machine_id,
            machine_name: Some(MachineName::parse("vultr").unwrap()),
            operation: web.clone(),
            display_name: None,
            service_name: Some(ServiceName::parse("cashdash-web").unwrap()),
            status: OperationStatus::Unexecuted,
        },
        OperationRow {
            index: 2,
            machine_id,
            machine_name: Some(MachineName::parse("vultr").unwrap()),
            operation: volume.clone(),
            display_name: None,
            service_name: None,
            status: OperationStatus::Unexecuted,
        },
    ];
    let outcome = DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation: failed,
            error: timed_out_create(),
        },
        unexecuted: vec![web, volume],
    };
    let text = paint_closing(&outcome, &rows, true, &PLAIN);
    assert!(
        text.contains("Failed: replace cashdash-frontend on machine-2"),
        "{text}"
    );
    assert!(
        text.contains("create failed: target Machine RPC timed out"),
        "{text}"
    );
    assert!(!text.contains("cashdash-web"), "{text}");
    assert!(!text.contains("cashdash_data"), "{text}");
    assert!(!text.contains("Completed"), "{text}");
    assert!(!text.contains(&"d".repeat(32)), "{text}");
}

#[test]
fn failed_volume_footer_keeps_the_failing_machine_when_names_collide() {
    let alpha = MachineId::parse("a".repeat(32)).unwrap();
    let beta = MachineId::parse("b".repeat(32)).unwrap();
    let name = DockerVolumeName::parse("data").unwrap();
    let on_alpha = DeployOperation::RemoveVolume {
        id: DockerVolumeId {
            machine_id: alpha,
            name: name.clone(),
        },
    };
    let on_beta = DeployOperation::RemoveVolume {
        id: DockerVolumeId {
            machine_id: beta,
            name,
        },
    };
    let rows = vec![
        OperationRow {
            index: 0,
            machine_id: alpha,
            machine_name: Some(MachineName::parse("alpha").unwrap()),
            operation: on_alpha.clone(),
            display_name: None,
            service_name: None,
            status: OperationStatus::Completed,
        },
        OperationRow {
            index: 1,
            machine_id: beta,
            machine_name: Some(MachineName::parse("beta").unwrap()),
            operation: on_beta.clone(),
            display_name: None,
            service_name: None,
            status: OperationStatus::Running {
                phase: OperationPhase::RemovingVolume,
            },
        },
    ];
    let outcome = DeployOutcome::Failed {
        completed: vec![on_alpha],
        failed: FailedOperation::Operation {
            operation: on_beta,
            error: ExecutionError::Machine {
                action: MachineAction::RemoveVolume,
                error: RpcError {
                    code: RpcErrorCode::Unavailable,
                    message: "target Machine RPC timed out".into(),
                    details: serde_json::Value::Null,
                },
            },
        },
        unexecuted: Vec::new(),
    };
    let text = paint_closing(&outcome, &rows, true, &PLAIN);
    assert!(text.contains("Failed: remove data on beta"), "{text}");
    assert!(!text.contains("Failed: remove data on alpha"), "{text}");
}

#[test]
fn wait_healthy_footer_omits_the_dependent_machine() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let operation = DeployOperation::WaitHealthy {
        machine_id,
        dependent: QualifiedService::parse("app/web").unwrap(),
        dependency: QualifiedService::parse("app/db").unwrap(),
    };
    let error = || ExecutionError::DependencyHealth {
        dependency: QualifiedService::parse("app/db").unwrap(),
        failure: ployz_core::DependencyHealthFailure::NoContainers,
    };
    let row = OperationRow {
        index: 0,
        machine_id,
        machine_name: Some(MachineName::parse("edge").unwrap()),
        operation: operation.clone(),
        display_name: None,
        service_name: Some(ServiceName::parse("web").unwrap()),
        status: OperationStatus::Failed { error: error() },
    };
    let outcome = DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation,
            error: error(),
        },
        unexecuted: Vec::new(),
    };
    let text = paint_closing(&outcome, &[row], true, &PLAIN);
    assert!(text.contains("Failed: wait app/db\n"), "{text}");
    assert!(!text.contains(" on edge"), "{text}");
}

#[test]
fn replacement_failures_print_only_the_compensation_that_ran() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let operation = ReplacementOperation {
        machine_id,
        old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
        spec: resolved("cashdash-frontend", "app:latest"),
        skip_health_monitor: false,
    };
    let error = || health_timeout(ContainerId::parse("c".repeat(64)).unwrap());
    let health = |compensation| FailedOperation::ReplacementHealth {
        operation: operation.clone(),
        error: error(),
        compensation,
    };
    for (failed, expected, forbidden) in [
        (
            health(ReplacementCompensation::StartFirst {
                stop_new_container: StopAttempt::Stopped,
            }),
            vec!["stopped the new container", "health check timed out"],
            vec![],
        ),
        (
            health(ReplacementCompensation::StopFirst {
                stop_new_container: StopAttempt::Stopped,
                restart_old_container: RestartAttempt::NotAttempted,
            }),
            vec![
                "stopped the new container",
                "did not restart the old container",
            ],
            vec![],
        ),
        (
            FailedOperation::Operation {
                operation: DeployOperation::ReplaceContainer(operation.clone()),
                error: error(),
            },
            vec!["health check timed out"],
            vec![
                "stopped the new container",
                "restarted the old container",
                "did not restart",
                "compensation",
            ],
        ),
    ] {
        let outcome = DeployOutcome::Failed {
            completed: Vec::new(),
            failed,
            unexecuted: Vec::new(),
        };
        let text = paint_closing(&outcome, &[], false, &PLAIN);
        for needle in expected {
            assert!(text.contains(needle), "{needle}: {text}");
        }
        for needle in forbidden {
            assert!(!text.contains(needle), "{needle}: {text}");
        }
        assert!(!text.to_ascii_lowercase().contains("rolled back"), "{text}");
        assert!(!text.contains("reverted"), "{text}");
        assert!(!text.contains("restored"), "{text}");
    }
}

#[test]
fn health_failure_offers_logs_when_service_is_known() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let spec = resolved("cashdash-frontend", "app:latest");
    let operation = DeployOperation::ReplaceContainer(ReplacementOperation {
        machine_id,
        old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
        spec,
        skip_health_monitor: false,
    });
    let row = OperationRow {
        index: 0,
        machine_id,
        machine_name: Some(MachineName::parse("machine-2").unwrap()),
        operation: operation.clone(),
        display_name: None,
        service_name: Some(ServiceName::parse("cashdash-frontend").unwrap()),
        status: OperationStatus::Failed {
            error: health_timeout(ContainerId::parse("c".repeat(64)).unwrap()),
        },
    };
    let outcome = DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation,
            error: health_timeout(ContainerId::parse("c".repeat(64)).unwrap()),
        },
        unexecuted: Vec::new(),
    };
    let text = paint_closing(&outcome, &[row], true, &PLAIN);
    assert!(
        text.contains("next: ployz service logs cashdash-frontend"),
        "{text}"
    );
    assert!(!text.contains('/'), "{text}");
}

#[test]
fn color_marks_the_failure_but_not_the_service_name() {
    let machine_id = MachineId::parse("d".repeat(32)).unwrap();
    let row = OperationRow {
        index: 0,
        machine_id,
        machine_name: Some(MachineName::parse("machine-2").unwrap()),
        operation: DeployOperation::ReplaceContainer(ReplacementOperation {
            machine_id,
            old_container_id: ContainerId::parse("f".repeat(64)).unwrap(),
            spec: resolved("cashdash-frontend", "app:latest"),
            skip_health_monitor: false,
        }),
        display_name: None,
        service_name: None,
        status: OperationStatus::Failed {
            error: timed_out_create(),
        },
    };
    let rows = [row];
    let plain = paint_live("Deploying to default", 0, 1, &rows, &PLAIN);
    let color = paint_live("Deploying to default", 0, 1, &rows, &COLOR);
    assert!(!plain.contains('\u{1b}'), "{plain:?}");
    assert!(color.contains('\u{1b}'), "{color:?}");
    assert!(plain.contains("✖"), "{plain}");

    let outcome = DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation: rows[0].operation.clone(),
            error: timed_out_create(),
        },
        unexecuted: Vec::new(),
    };
    let footer = paint_closing(&outcome, &rows, true, &COLOR);
    assert!(
        footer.contains(&COLOR.paint(Role::Fail, "Failed:")),
        "{footer:?}"
    );
    assert!(
        !footer
            .contains(&COLOR.paint(Role::Fail, "Failed: replace cashdash-frontend on machine-2")),
        "{footer:?}"
    );
    assert!(footer.contains("cashdash-frontend"), "{footer:?}");
}

fn timed_out_create() -> ExecutionError {
    ExecutionError::Machine {
        action: MachineAction::CreateContainer,
        error: RpcError {
            code: RpcErrorCode::Unavailable,
            message: "target Machine RPC timed out".into(),
            details: serde_json::Value::Null,
        },
    }
}

fn health_timeout(container_id: ContainerId) -> ExecutionError {
    ExecutionError::Health {
        container_id,
        failure: HealthFailure::TimedOut,
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
