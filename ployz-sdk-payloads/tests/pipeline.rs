use std::collections::BTreeMap;

use ployz_core::{
    CERTIFICATE_POLICY_CAPABILITY, ContainerRuntimeObservation, ContractDescription,
    DESCRIBE_CONTRACT_CAPABILITY, DockerVolume, HealthObservation, MembershipObservation, RpcError,
    RpcErrorCode,
};
use ployz_sdk_payloads::{
    PACKAGE_NAME, decode_fixture, drift, fixtures, sdk_package_root, write_generated,
};
use serde_json::Value;

fn fixture<'a>(fixtures: &'a BTreeMap<String, Value>, name: &str) -> &'a Value {
    fixtures
        .get(name)
        .unwrap_or_else(|| panic!("missing fixture {name}"))
}

fn pkg_field<'a>(pkg: &'a Value, name: &str) -> &'a Value {
    pkg.get(name)
        .unwrap_or_else(|| panic!("missing package.json field {name}"))
}

fn nested_object<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .get(name)
        .unwrap_or_else(|| panic!("missing nested object {name}"))
}

fn type_block<'a>(dts: &'a str, type_name: &str) -> &'a str {
    let marker = format!("export type {type_name}");
    let mut from = 0;
    while let Some(relative) = dts.get(from..).and_then(|slice| slice.find(&marker)) {
        let start = from + relative;
        from = start + marker.len();
        let next = dts.as_bytes().get(from).copied().unwrap_or(0);
        if next == b'_' || next.is_ascii_alphanumeric() {
            continue;
        }
        let rest = dts.get(start..).expect("TypeScript type is in range");
        return rest
            .split_once("\nexport ")
            .map_or(rest, |(block, _)| block);
    }
    panic!("missing TypeScript type {type_name}");
}

fn assert_ts_has_object_keys(dts: &str, type_name: &str, value: &Value) {
    let block = type_block(dts, type_name);
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{type_name} fixture is not an object"));
    for key in object.keys() {
        assert!(
            block.contains(key.as_str()),
            "{type_name} TypeScript is missing field {key}"
        );
    }
}

#[test]
fn npm_package_identity_matches_the_napi_crate() {
    let pkg: Value = serde_json::from_str(include_str!("../../ployz-sdk/package.json")).unwrap();
    assert_eq!(pkg_field(&pkg, "name"), PACKAGE_NAME);
    assert_eq!(pkg_field(&pkg, "name"), "@ployz/sdk");
    assert_eq!(pkg_field(&pkg, "version"), env!("CARGO_PKG_VERSION"));
    assert_eq!(pkg_field(&pkg, "main"), "index.js");
    assert_eq!(pkg_field(&pkg, "types"), "index.d.ts");
}

#[test]
fn workspace_forbids_unsafe_outside_the_napi_crate() {
    let workspace = include_str!("../../Cargo.toml");
    assert!(workspace.contains("unsafe_code = \"forbid\""));
    assert!(!workspace.contains("unsafe_code = \"deny\""));
    assert!(!workspace.contains("unsafe_code = \"allow\""));
    let sdk_manifest = include_str!("../../ployz-sdk/Cargo.toml");
    assert!(sdk_manifest.contains("unsafe_code = \"allow\""));
    assert!(
        !sdk_manifest
            .lines()
            .any(|line| line.trim() == "workspace = true"),
        "ployz-sdk must not inherit workspace lints"
    );
    let payloads = include_str!("../src/lib.rs");
    assert!(!payloads.contains("allow(unsafe_code)"));
}

#[test]
fn generated_artifacts_match_checked_in_files() {
    if let Some(drift) = drift(&sdk_package_root()) {
        panic!("{drift}");
    }
}

#[test]
fn json_fixtures_round_trip_through_rust_types() {
    let fixtures = fixtures();
    let description: ContractDescription =
        decode_fixture(fixture(&fixtures, "contract_description"));
    assert!(description.supports(DESCRIBE_CONTRACT_CAPABILITY));
    assert_eq!(
        serde_json::to_value(&description).unwrap(),
        *fixture(&fixtures, "contract_description")
    );

    let error: RpcError = decode_fixture(fixture(&fixtures, "rpc_error"));
    assert_eq!(error.code.as_str(), "unsupported");
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        *fixture(&fixtures, "rpc_error")
    );

    let volume: DockerVolume = decode_fixture(fixture(&fixtures, "docker_volume"));
    assert_eq!(volume.driver, "local");
    assert_eq!(
        serde_json::to_value(&volume).unwrap(),
        *fixture(&fixtures, "docker_volume")
    );

    let encoded = fixture(&fixtures, "partial_result");
    let successes = encoded
        .get("successes")
        .and_then(Value::as_array)
        .expect("partial_result.successes");
    let failures = encoded
        .get("failures")
        .and_then(Value::as_array)
        .expect("partial_result.failures");
    let omissions = encoded
        .get("omissions")
        .and_then(Value::as_array)
        .expect("partial_result.omissions");
    assert_eq!(successes.len(), 1);
    assert_eq!(failures.len(), 1);
    assert_eq!(omissions.len(), 1);
    let volume: DockerVolume = decode_fixture(
        successes
            .first()
            .and_then(|row| row.get("value"))
            .expect("success value"),
    );
    assert_eq!(volume.driver, "local");
    let error: RpcError = decode_fixture(
        failures
            .first()
            .and_then(|row| row.get("error"))
            .expect("failure error"),
    );
    assert_eq!(error.code.as_str(), "unsupported");
}

#[test]
fn unknown_fields_are_accepted_on_public_payloads() {
    let fixtures = fixtures();
    let description: ContractDescription =
        decode_fixture(fixture(&fixtures, "contract_description_unknown_fields"));
    assert!(description.supports(DESCRIBE_CONTRACT_CAPABILITY));

    let volume: DockerVolume = decode_fixture(fixture(&fixtures, "docker_volume_unknown_fields"));
    assert_eq!(
        serde_json::to_value(&volume).unwrap(),
        *fixture(&fixtures, "docker_volume")
    );
}

#[test]
fn observation_enums_keep_an_unknown_case() {
    let fixtures = fixtures();
    let membership: MembershipObservation =
        decode_fixture(fixture(&fixtures, "membership_observation_unknown"));
    assert_eq!(membership.as_str(), "future_membership");
    assert_eq!(
        serde_json::to_value(&membership).unwrap(),
        *fixture(&fixtures, "membership_observation_unknown")
    );

    let health: HealthObservation =
        decode_fixture(fixture(&fixtures, "health_observation_unknown"));
    assert_eq!(health, HealthObservation::Unrecognized("degraded".into()));

    let unknown_json = fixture(&fixtures, "container_runtime_unknown");
    let unknown: ContainerRuntimeObservation = decode_fixture(unknown_json);
    assert_eq!(
        unknown,
        ContainerRuntimeObservation::Unknown {
            raw: unknown_json.clone()
        }
    );
    assert_eq!(serde_json::to_value(&unknown).unwrap(), *unknown_json);

    let known: ContainerRuntimeObservation =
        decode_fixture(fixture(&fixtures, "container_runtime_known_unknown_fields"));
    assert_eq!(
        known,
        ContainerRuntimeObservation::Running {
            health: HealthObservation::Healthy
        }
    );
}

#[test]
fn generated_typescript_encodes_additive_evolution_rules() {
    let dts = ployz_sdk_payloads::artifacts().index_dts;
    assert!(dts.contains("export type Additive<T extends object> = T & JsonObject;"));
    assert!(dts.contains("export type MembershipObservation ="));
    assert!(dts.contains("| (string & {});"));
    assert!(dts.contains("export type ContainerRuntimeObservation ="));
    assert!(dts.contains("state?: string"));
    assert!(dts.contains("export type ContractDescription = Additive<{"));
    assert!(dts.contains("export type DeployIntent = Additive<{"));
    assert!(dts.contains("export type DeployOutcome<E = RpcError> ="));
    assert!(dts.contains("unexecuted: JsonValue[]"));
    assert!(!dts.contains("{ Success:"));
    assert!(dts.contains("export const DESCRIBE_CONTRACT_CAPABILITY: CapabilityName"));
    assert!(dts.contains(DESCRIBE_CONTRACT_CAPABILITY));
    assert!(dts.contains(CERTIFICATE_POLICY_CAPABILITY));
    assert!(dts.contains("export declare function packageName(): \"@ployz/sdk\";"));
    for wire in MembershipObservation::known_wires() {
        assert!(
            dts.contains(&format!("\"{wire}\"")),
            "MembershipObservation TypeScript is missing {wire}"
        );
    }
    for wire in HealthObservation::known_wires() {
        assert!(
            dts.contains(&format!("\"{wire}\"")),
            "HealthObservation TypeScript is missing {wire}"
        );
    }
    for wire in RpcErrorCode::known_wires() {
        assert!(
            dts.contains(&format!("\"{wire}\"")),
            "RpcErrorCode TypeScript is missing {wire}"
        );
    }
}

#[test]
fn generated_typescript_declares_rust_fixture_keys() {
    let dts = ployz_sdk_payloads::artifacts().index_dts;
    let fixtures = fixtures();
    assert_ts_has_object_keys(
        &dts,
        "ContractDescription",
        fixture(&fixtures, "contract_description"),
    );
    assert_ts_has_object_keys(&dts, "RpcError", fixture(&fixtures, "rpc_error"));
    assert_ts_has_object_keys(&dts, "DockerVolume", fixture(&fixtures, "docker_volume"));
    assert_ts_has_object_keys(
        &dts,
        "DockerVolumeId",
        nested_object(fixture(&fixtures, "docker_volume"), "id"),
    );
    assert_ts_has_object_keys(&dts, "PartialResult", fixture(&fixtures, "partial_result"));
    assert_ts_has_object_keys(&dts, "DeployIntent", fixture(&fixtures, "deploy_intent"));
    assert_ts_has_object_keys(
        &dts,
        "PlanOptions",
        nested_object(fixture(&fixtures, "deploy_intent"), "options"),
    );
}

#[test]
fn capability_fixture_matches_the_rpc_catalog() {
    let fixtures = fixtures();
    let names = fixture(&fixtures, "capabilities")
        .as_array()
        .expect("capabilities array")
        .iter()
        .map(|value| value.as_str().expect("capability string"))
        .collect::<Vec<_>>();
    assert!(names.contains(&DESCRIBE_CONTRACT_CAPABILITY));
    assert!(names.contains(&CERTIFICATE_POLICY_CAPABILITY));
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn no_nats_compatibility_package_is_introduced() {
    let root = sdk_package_root();
    let workspace = root.parent().expect("workspace root");
    assert!(!workspace.join("packages/ployz-sdk").exists());
    assert!(!workspace.join("ployz-nats-sdk").exists());
}

#[test]
fn write_generated_fails_when_the_package_root_is_a_file() {
    let path = std::env::temp_dir().join(format!(
        "ployz-sdk-write-generated-{}.tmp",
        std::process::id()
    ));
    std::fs::write(&path, b"not a directory").unwrap();
    assert!(write_generated(&path).is_err());
    std::fs::remove_file(&path).unwrap();
}
