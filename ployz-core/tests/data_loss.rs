//! Data Loss identity.

use ployz_core::{
    DataLoss, DockerVolume, DockerVolumeId, DockerVolumeName, MachineId, ObservedDataLoss,
};
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
fn from_volumes_names_each_docker_volume_and_none_when_empty() {
    let loaded = machine_id('a');
    let observed =
        ObservedDataLoss::from_volumes(&[volume(loaded, "data"), volume(loaded, "logs")]);
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
    assert_eq!(
        ObservedDataLoss::from_volumes(&[]).data_loss,
        Vec::<DataLoss>::new()
    );
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
