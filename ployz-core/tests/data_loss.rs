//! Data Loss identity.

use ployz_core::{DataLoss, DockerVolumeId, DockerVolumeName, MachineId, ObservedDataLoss};
use serde_json::json;

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
fn uncovered_by_is_empty_when_confirmation_names_every_observed_loss() {
    let observed = ObservedDataLoss {
        data_loss: vec![volume('a', "data"), volume('a', "logs")],
    };
    assert_eq!(
        observed.uncovered_by(&[volume('a', "logs"), volume('a', "data")]),
        Vec::<DataLoss>::new()
    );
}

#[test]
fn uncovered_by_names_fresh_loss_the_confirmation_omitted() {
    let observed = ObservedDataLoss {
        data_loss: vec![volume('a', "data"), volume('a', "logs")],
    };
    assert_eq!(
        observed.uncovered_by(&[volume('a', "data")]),
        vec![volume('a', "logs")]
    );
}

#[test]
fn uncovered_by_ignores_confirmed_names_that_are_no_longer_observed() {
    let observed = ObservedDataLoss {
        data_loss: vec![volume('a', "data")],
    };
    assert_eq!(
        observed.uncovered_by(&[volume('a', "data"), volume('a', "gone")]),
        Vec::<DataLoss>::new()
    );
}

#[test]
fn uncovered_by_allows_an_empty_confirmation_when_there_is_no_data_loss() {
    let observed = ObservedDataLoss {
        data_loss: Vec::new(),
    };
    assert_eq!(observed.uncovered_by(&[]), Vec::<DataLoss>::new());
}

fn volume(machine: char, name: &str) -> DataLoss {
    DataLoss::DockerVolume(DockerVolumeId {
        machine_id: machine_id(machine),
        name: DockerVolumeName::parse(name).unwrap(),
    })
}

fn machine_id(value: char) -> MachineId {
    MachineId::parse(value.to_string().repeat(32)).unwrap()
}
