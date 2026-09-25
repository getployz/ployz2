//! Node contract coverage for preparation outcomes at the native SDK boundary.

use std::sync::{Arc, atomic::Ordering};

use ployz_build::{Stage, TargetEvidence, WorkEvidence, remote::Outcome};
use ployz_core::BUILD_CAPABILITY;

use super::{support, unix_session::UnixSession};

#[tokio::test]
async fn node_preparation_refusal_and_cancellation_preserve_work_evidence() {
    for kind in [
        "failed",
        "unknown",
        "cancel",
        "selection",
        "cancel-transfer",
    ] {
        let session = UnixSession::start().await;
        let marker_directory = tempfile::tempdir().unwrap();
        let marker = marker_directory.path().join("transfer-started");
        let mut description = support::test_description();
        description.machine_id = support::machine_id('a');
        if kind != "selection" {
            description
                .capabilities
                .insert(BUILD_CAPABILITY.parse().unwrap());
        }
        let work = WorkEvidence(std::collections::BTreeMap::from([(
            "api".into(),
            TargetEvidence::Unattempted,
        )]));
        let recorder = Arc::new(match kind {
            "failed" => support::BuildRecorder {
                admission_outcome: Some(Outcome::Failed {
                    stage: Stage::Admission,
                    message: "admission rejected".into(),
                    work,
                }),
                ..Default::default()
            },
            "unknown" => support::BuildRecorder {
                admission_outcome: Some(Outcome::Unknown {
                    stage: Stage::Admission,
                    message: "connection lost".into(),
                    work,
                }),
                ..Default::default()
            },
            "cancel" => support::BuildRecorder {
                quiet_until_cancel: true,
                ..Default::default()
            },
            "cancel-transfer" => support::BuildRecorder {
                retain_images: true,
                blocked_transfer_marker: Some(marker.clone()),
                ..Default::default()
            },
            _ => support::BuildRecorder::default(),
        });
        let mut service = support::DiscoveryService::new(description.clone());
        let mut machine = support::machine('a', "builder");
        machine.machine.runtime.architecture = "x86_64".into();
        machine.machine.accepts_builds = kind != "selection";
        service.machines = vec![machine];
        service.builds = Some(recorder.clone());
        let _machine = session.spawn_machine(description.machine_id, service).await;
        let mut environment = vec![("PLOYZ_PREPARATION_OUTCOME", kind)];
        if kind == "cancel-transfer" {
            environment.push(("PLOYZ_TRANSFER_STARTED", marker.to_str().unwrap()));
        }

        session
            .assert_sdk_script("node_prepare.js", description.machine_id, &environment)
            .await;

        let uploads = recorder.uploads.load(Ordering::SeqCst);
        let routes = recorder.routes.lock().unwrap().len();
        let created = recorder.created.load(Ordering::SeqCst);
        match kind {
            "failed" | "unknown" => {
                assert_eq!(uploads, 0, "{kind}: no upload after admission refusal");
                assert_eq!(routes, 1, "{kind}: a refused build is not replayed");
            }
            "cancel" => {
                assert!(recorder.cancelled.load(Ordering::SeqCst));
                assert_eq!(uploads, 1);
                assert!(!created);
            }
            "selection" => {
                assert_eq!(uploads, 0);
                assert_eq!(routes, 0);
            }
            _ => {
                assert_eq!(recorder.deliveries.lock().unwrap().len(), 1);
                assert!(!recorder.delivered.load(Ordering::SeqCst));
                assert!(!created);
            }
        }
    }
}

#[tokio::test]
async fn node_preparation_delivers_images_and_retains_them_through_confirmation() {
    for destination_id in ['a', 'b'] {
        let session = UnixSession::start().await;
        let mut description = support::test_description();
        description.machine_id = support::machine_id('a');
        description
            .capabilities
            .insert(BUILD_CAPABILITY.parse().unwrap());
        let recorder = Arc::new(support::BuildRecorder {
            retain_images: true,
            ..Default::default()
        });
        let mut service = support::DiscoveryService::new(description.clone());
        let mut machine = support::machine('a', "builder");
        machine.machine.runtime.architecture = "x86_64".into();
        machine.machine.accepts_services = destination_id == 'a';
        service.machines = vec![machine];
        if destination_id != 'a' {
            let mut destination = support::machine(destination_id, "runtime");
            destination.machine.accepts_builds = false;
            destination.machine.runtime.architecture = "x86_64".into();
            service.machines.push(destination);
        }
        service.inspect_container_result = Some(ployz_core::ContainerDetails {
            environment: None,
            container: super::listing_container(
                '1',
                'a',
                "api",
                ployz_core::ContainerKind::ServiceContainer,
                ployz_core::ContainerRuntimeObservation::Running {
                    health: ployz_core::HealthObservation::Healthy,
                },
            ),
        });
        service.builds = Some(recorder.clone());
        let _machine = session.spawn_machine(description.machine_id, service).await;
        session
            .assert_sdk_script(
                "node_prepare.js",
                description.machine_id,
                &[("PLOYZ_PREPARATION_OUTCOME", "success")],
            )
            .await;
        assert_eq!(recorder.uploads.load(Ordering::SeqCst), 1);
        assert!(recorder.delivered.load(Ordering::SeqCst));
        assert!(recorder.created.load(Ordering::SeqCst));
        let deliveries = recorder.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        let delivery = deliveries.first().unwrap();
        assert_eq!(
            delivery.0,
            ployz_core::RoutingRequest::One(ployz_core::MachineTarget::from(&support::machine_id(
                destination_id
            )))
        );
        assert_eq!(delivery.1.platform, "linux/amd64");
        assert!(
            delivery
                .1
                .pull
                .image()
                .ends_with(&format!("@sha256:{}", "1".repeat(64)))
        );
    }
}
