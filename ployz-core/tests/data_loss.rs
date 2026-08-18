//! Data Loss identity and observer-relative listing.

use ployz_core::{
    DataLoss, DockerVolume, DockerVolumeId, DockerVolumeName, MachineFailure, MachineId,
    MachineSuccess, ObservedDataLoss, PartialResult, RpcError, RpcErrorCode,
};
use serde_json::{Value, json};

#[test]
fn docker_volume_data_loss_carries_machine_and_name() {
    let loss = DataLoss::DockerVolume(DockerVolumeId {
        machine_id: machine_id('a'),
        name: DockerVolumeName::parse("data").unwrap(),
    });
    let encoded = serde_json::to_value(&loss).unwrap();
    assert_eq!(
        encoded,
        json!({
            "DockerVolume": {
                "machine_id": machine_id('a').as_str(),
                "name": "data"
            }
        })
    );
    assert!(encoded.get("kind").is_none());
    assert!(encoded.get("scope").is_none());
    assert_eq!(serde_json::from_value::<DataLoss>(encoded).unwrap(), loss);
}

#[test]
fn a_kind_cannot_carry_an_identity_that_does_not_belong_to_it() {
    let illegal = json!({
        "kind": "docker_volume",
        "name": "data",
        "scope": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    serde_json::from_value::<DataLoss>(illegal).unwrap_err();
    serde_json::from_value::<DataLoss>(json!({
        "DockerVolume": { "name": "data" }
    }))
    .unwrap_err();
}

#[test]
fn volume_listing_with_volumes_and_without_are_live_observation() {
    let loaded = machine_id('a');
    let empty = machine_id('c');
    let listed = PartialResult {
        successes: vec![
            MachineSuccess {
                machine_id: loaded,
                value: vec![volume(loaded, "data"), volume(loaded, "logs")],
            },
            MachineSuccess {
                machine_id: empty,
                value: vec![],
            },
        ],
        failures: Vec::new(),
        omissions: Vec::new(),
    };

    let observed = ObservedDataLoss::from_volume_listing(&loaded, &listed).unwrap();
    assert_eq!(
        observed.data_loss,
        [
            DataLoss::DockerVolume(DockerVolumeId {
                machine_id: loaded,
                name: DockerVolumeName::parse("data").unwrap(),
            }),
            DataLoss::DockerVolume(DockerVolumeId {
                machine_id: loaded,
                name: DockerVolumeName::parse("logs").unwrap(),
            }),
        ]
    );

    let none = ObservedDataLoss::from_volume_listing(&empty, &listed).unwrap();
    assert_eq!(none.data_loss, Vec::<DataLoss>::new());
}

#[test]
fn omitted_or_failed_listings_are_not_an_empty_data_loss_list() {
    let omitted = machine_id('d');
    let failed = machine_id('b');
    let missing = machine_id('e');
    let listed = PartialResult {
        successes: Vec::new(),
        failures: vec![MachineFailure {
            machine_id: failed,
            error: RpcError {
                code: RpcErrorCode::Unavailable,
                message: "target unavailable".into(),
                details: Value::Null,
            },
        }],
        omissions: vec![omitted],
    };

    let omitted_error = ObservedDataLoss::from_volume_listing(&omitted, &listed).unwrap_err();
    assert_eq!(omitted_error.code, RpcErrorCode::Unavailable);
    assert!(omitted_error.message.contains("this observer"));

    let failed_error = ObservedDataLoss::from_volume_listing(&failed, &listed).unwrap_err();
    assert_eq!(failed_error.code, RpcErrorCode::Unavailable);
    assert_eq!(failed_error.message, "target unavailable");

    let missing_error = ObservedDataLoss::from_volume_listing(&missing, &listed).unwrap_err();
    assert_eq!(missing_error.code, RpcErrorCode::NotFound);
}

fn volume(machine_id: MachineId, name: &str) -> DockerVolume {
    DockerVolume {
        id: DockerVolumeId {
            machine_id,
            name: DockerVolumeName::parse(name).unwrap(),
        },
        driver: "local".into(),
        options: Default::default(),
        labels: Default::default(),
    }
}

fn machine_id(value: char) -> MachineId {
    MachineId::parse(value.to_string().repeat(32)).unwrap()
}
