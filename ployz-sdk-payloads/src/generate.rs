//! Deterministic TypeScript and JSON fixtures for `@ployz/sdk` public payloads.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use ployz_core::{
    CapabilityName, ContainerId, ContractDescription, DESCRIBE_CONTRACT_CAPABILITY, DeployIntent,
    DeployOperation, DeployOutcome, DockerVolume, DockerVolumeId, DockerVolumeName,
    FailedOperation, HealthObservation, MachineFailure, MachineId, MachineSuccess,
    MembershipObservation, PROTOCOL_MAJOR, PartialResult, PlanOptions, RpcError, RpcErrorCode,
    ServiceAttempt, ServiceName,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// npm package name for this crate's Node artifact.
pub const PACKAGE_NAME: &str = "@ployz/sdk";

const MACHINE_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
const OTHER_MACHINE_ID_HEX: &str = "fedcba9876543210fedcba9876543210";

enum Shape {
    Alias(&'static str),
    OpenString(&'static [&'static str]),
    Additive(&'static [(&'static str, &'static str)]),
    Raw(&'static str),
}

const PAYLOADS: &[(&str, Shape)] = &[
    (
        "JsonValue",
        Shape::Raw(
            "export type JsonValue =\n  | null\n  | boolean\n  | number\n  | string\n  | JsonValue[]\n  | JsonObject;\n",
        ),
    ),
    (
        "JsonObject",
        Shape::Raw("export type JsonObject = { readonly [key: string]: JsonValue | undefined };\n"),
    ),
    (
        "Additive",
        Shape::Raw("export type Additive<T extends object> = T & JsonObject;\n"),
    ),
    ("MachineId", Shape::Alias("string")),
    ("ContainerId", Shape::Alias("string")),
    ("ServiceName", Shape::Alias("string")),
    ("DockerVolumeName", Shape::Alias("string")),
    ("CapabilityName", Shape::Alias("string")),
    ("RequestedServiceSpec", Shape::Alias("JsonValue")),
    ("ResolvedServiceSpec", Shape::Alias("JsonValue")),
    ("ServiceVolume", Shape::Alias("JsonValue")),
    (
        "MembershipObservation",
        Shape::OpenString(MembershipObservation::known_wires()),
    ),
    (
        "HealthObservation",
        Shape::OpenString(HealthObservation::known_wires()),
    ),
    (
        "RpcErrorCode",
        Shape::OpenString(RpcErrorCode::known_wires()),
    ),
    (
        "DockerVolumeId",
        Shape::Additive(&[("machine_id", "MachineId"), ("name", "DockerVolumeName")]),
    ),
    (
        "DockerVolume",
        Shape::Additive(&[
            ("id", "DockerVolumeId"),
            ("driver", "string"),
            ("options", "{ readonly [key: string]: string }"),
            ("labels", "{ readonly [key: string]: string }"),
        ]),
    ),
    (
        "ContractDescription",
        Shape::Additive(&[
            ("machine_id", "MachineId"),
            ("protocol_major", "number"),
            ("daemon_version", "string"),
            ("capabilities", "CapabilityName[]"),
        ]),
    ),
    (
        "RpcError",
        Shape::Raw(
            "export type RpcError = Additive<{\n  code: RpcErrorCode;\n  message: string;\n  details?: JsonValue;\n}>;\n",
        ),
    ),
    (
        "MachineSuccess",
        Shape::Raw(
            "export type MachineSuccess<T> = Additive<{\n  machine_id: MachineId;\n  value: T;\n}>;\n",
        ),
    ),
    (
        "MachineFailure",
        Shape::Raw(
            "export type MachineFailure<E> = Additive<{\n  machine_id: MachineId;\n  error: E;\n}>;\n",
        ),
    ),
    (
        "PartialResult",
        Shape::Raw(
            "export type PartialResult<T, E> = Additive<{\n  successes: Array<MachineSuccess<T>>;\n  failures: Array<MachineFailure<E>>;\n  omissions: MachineId[];\n}>;\n",
        ),
    ),
    (
        "ContainerRuntimeObservation",
        Shape::Raw(
            "export type ContainerRuntimeObservation =\n  | Additive<{ state: \"created\" }>\n  | Additive<{ state: \"running\"; health: HealthObservation }>\n  | Additive<{ state: \"paused\" }>\n  | Additive<{ state: \"restarting\" }>\n  | Additive<{ state: \"exited\"; code: number }>\n  | Additive<{ state: \"removing\" }>\n  | Additive<{ state: \"dead\" }>\n  | Additive<{ state?: string }>;\n",
        ),
    ),
    (
        "PlanOptions",
        Shape::Additive(&[
            ("force_recreate", "boolean"),
            ("skip_health_monitor", "boolean"),
            ("placement_seed", "number"),
        ]),
    ),
    (
        "ServiceAttempt",
        Shape::Additive(&[("name", "ServiceName")]),
    ),
    (
        "DeployIntent",
        Shape::Additive(&[
            ("target", "RequestedServiceSpec[]"),
            ("apply", "ServiceAttempt[]"),
            ("options", "PlanOptions"),
        ]),
    ),
    (
        "SerdeResult",
        Shape::Raw(
            "export type SerdeResult<T, E> =\n  | Additive<{ Ok: T }>\n  | Additive<{ Err: E }>;\n",
        ),
    ),
    (
        "ReplacementOperation",
        Shape::Raw(
            "export type ReplacementOperation = Additive<{\n  machine_id: MachineId;\n  old_container_id: ContainerId;\n  spec: ResolvedServiceSpec;\n  skip_health_monitor: boolean;\n}>;\n",
        ),
    ),
    (
        "DeployOperation",
        Shape::Raw(
            "export type DeployOperation =\n  | Additive<{ CreateVolume: { machine_id: MachineId; volume: ServiceVolume } }>\n  | Additive<{\n      RunContainer: {\n        machine_id: MachineId;\n        spec: ResolvedServiceSpec;\n        skip_health_monitor: boolean;\n      };\n    }>\n  | Additive<{ StopContainer: { machine_id: MachineId; container_id: ContainerId } }>\n  | Additive<{ RemoveContainer: { machine_id: MachineId; container_id: ContainerId } }>\n  | Additive<{ ReplaceContainer: ReplacementOperation }>\n  | Additive<{ StopHook: { machine_id: MachineId; container_id: ContainerId } }>\n  | Additive<{\n      RunHook: {\n        machine_id: MachineId;\n        spec: ResolvedServiceSpec;\n        old_hook_containers: Array<[MachineId, ContainerId]>;\n      };\n    }>;\n",
        ),
    ),
    (
        "RestartAttempt",
        Shape::Raw(
            "export type RestartAttempt<E = RpcError> =\n  | \"NotAttempted\"\n  | Additive<{ Attempted: SerdeResult<null, E> }>;\n",
        ),
    ),
    (
        "ReplacementCompensation",
        Shape::Raw(
            "export type ReplacementCompensation<E = RpcError> =\n  | Additive<{ StartFirst: { stop_new_container: SerdeResult<null, E> } }>\n  | Additive<{\n      StopFirst: {\n        stop_new_container: SerdeResult<null, E>;\n        restart_old_container: RestartAttempt<E>;\n      };\n    }>;\n",
        ),
    ),
    (
        "FailedOperation",
        Shape::Raw(
            "export type FailedOperation<E = RpcError> =\n  | Additive<{ Operation: { operation: DeployOperation; error: E } }>\n  | Additive<{\n      ReplacementHealth: {\n        operation: ReplacementOperation;\n        error: E;\n        compensation: ReplacementCompensation<E>;\n      };\n    }>;\n",
        ),
    ),
    (
        "DeployOutcome",
        Shape::Raw(
            "export type DeployOutcome<E = RpcError> =\n  | Additive<{ Success: { completed: DeployOperation[] } }>\n  | Additive<{\n      Failed: {\n        completed: DeployOperation[];\n        failed: FailedOperation<E>;\n        unexecuted: DeployOperation[];\n      };\n    }>;\n",
        ),
    ),
];

/// Generated TypeScript declarations and JSON fixtures.
pub struct Artifacts {
    pub index_dts: String,
    pub fixtures_json: String,
}

/// Build the checked-in `@ployz/sdk` artifacts from Rust types.
#[must_use]
pub fn artifacts() -> Artifacts {
    check_additive_fields_match_rust();
    Artifacts {
        index_dts: typescript(),
        fixtures_json: pretty_json(&Value::Object(fixtures().into_iter().collect())),
    }
}

/// JSON fixtures keyed by stable names, produced from Rust values.
#[must_use]
pub fn fixtures() -> BTreeMap<String, Value> {
    let mut fixtures = BTreeMap::new();
    fixtures.insert(
        "contract_description".into(),
        to_value(&contract_description()),
    );
    fixtures.insert(
        "contract_description_unknown_fields".into(),
        with_unknown_field(
            to_value(&contract_description()),
            "build_revision",
            json!("future"),
        ),
    );
    fixtures.insert("rpc_error".into(), to_value(&rpc_error()));
    fixtures.insert("docker_volume".into(), to_value(&docker_volume()));
    fixtures.insert(
        "docker_volume_unknown_fields".into(),
        with_unknown_field(to_value(&docker_volume()), "quota_bytes", json!(1)),
    );
    fixtures.insert("partial_result".into(), to_value(&partial_result()));
    fixtures.insert(
        "membership_observation_unknown".into(),
        json!("future_membership"),
    );
    fixtures.insert("health_observation_unknown".into(), json!("degraded"));
    fixtures.insert(
        "container_runtime_unknown".into(),
        json!({
            "state": "hibernating",
            "wake_at": "tomorrow",
            "vendor": { "reason": 7 }
        }),
    );
    fixtures.insert(
        "container_runtime_known_unknown_fields".into(),
        json!({
            "state": "running",
            "health": "healthy",
            "engine_detail": "accepted and ignored"
        }),
    );
    fixtures.insert(
        "capabilities".into(),
        Value::Array(
            capability_rows()
                .into_iter()
                .map(|(_, name)| Value::String(name.to_owned()))
                .collect(),
        ),
    );
    fixtures.insert("service_attempt".into(), to_value(&service_attempt()));
    fixtures.insert("deploy_intent".into(), to_value(&deploy_intent()));
    fixtures.insert("deploy_outcome".into(), to_value(&deploy_outcome()));
    fixtures.insert(
        "deploy_outcome_unknown_fields".into(),
        with_unknown_field_in(
            to_value(&deploy_outcome()),
            "Success",
            "future_note",
            json!("ok"),
        ),
    );
    fixtures.insert(
        "deploy_outcome_failed".into(),
        to_value(&deploy_outcome_failed()),
    );
    fixtures
}

/// Write generated files under the napi package root.
///
/// # Errors
///
/// Returns filesystem errors from creating `generated/` or writing files.
pub fn write_generated(root: &Path) -> std::io::Result<()> {
    let artifacts = artifacts();
    fs::create_dir_all(root.join("generated"))?;
    fs::write(root.join("index.d.ts"), artifacts.index_dts)?;
    fs::write(
        root.join("generated/fixtures.json"),
        artifacts.fixtures_json,
    )?;
    Ok(())
}

/// Compare generated output to the files checked in under `root`.
#[must_use]
pub fn drift(root: &Path) -> Option<String> {
    let expected = artifacts();
    let mut problems = Vec::new();
    match fs::read_to_string(root.join("index.d.ts")) {
        Ok(on_disk) if on_disk == expected.index_dts => {}
        Ok(_) => problems.push("index.d.ts is stale".to_owned()),
        Err(error) => problems.push(format!("index.d.ts: {error}")),
    }
    match fs::read_to_string(root.join("generated/fixtures.json")) {
        Ok(on_disk) if on_disk == expected.fixtures_json => {}
        Ok(_) => problems.push("generated/fixtures.json is stale".to_owned()),
        Err(error) => problems.push(format!("generated/fixtures.json: {error}")),
    }
    if problems.is_empty() {
        None
    } else {
        problems.push(
            "regenerate with `cargo run -p ployz-sdk-payloads --bin generate-sdk-types`".to_owned(),
        );
        Some(problems.join("\n"))
    }
}

fn typescript() -> String {
    let mut out = String::from(
        "// Generated by `cargo run -p ployz-sdk-payloads --bin generate-sdk-types`. Do not edit.\n\n",
    );
    for (name, shape) in PAYLOADS {
        out.push_str(&emit_shape(name, shape));
        out.push('\n');
    }
    out.push_str(&capability_typescript());
    out.push_str("export declare function packageName(): \"");
    out.push_str(PACKAGE_NAME);
    out.push_str("\";\n");
    out
}

fn emit_shape(name: &str, shape: &Shape) -> String {
    match shape {
        Shape::Alias(ts) => format!("export type {name} = {ts};\n"),
        Shape::OpenString(known) => {
            let variants = known
                .iter()
                .map(|value| format!("\"{value}\""))
                .collect::<Vec<_>>()
                .join(" | ");
            format!("export type {name} = {variants} | (string & {{}});\n")
        }
        Shape::Additive(fields) => {
            let mut body = format!("export type {name} = Additive<{{\n");
            for (field, ts) in *fields {
                body.push_str("  ");
                body.push_str(field);
                body.push_str(": ");
                body.push_str(ts);
                body.push_str(";\n");
            }
            body.push_str("}>;\n");
            body
        }
        Shape::Raw(body) => (*body).to_owned(),
    }
}

fn capability_typescript() -> String {
    let rows = capability_rows();
    let mut out = String::new();
    for (rust_name, wire) in &rows {
        out.push_str("export const ");
        out.push_str(rust_name);
        out.push_str(": CapabilityName = \"");
        out.push_str(wire);
        out.push_str("\";\n");
    }
    out.push('\n');
    out.push_str("export const CATALOGUED_CAPABILITIES = [\n");
    for (_, wire) in &rows {
        out.push_str("  \"");
        out.push_str(wire);
        out.push_str("\",\n");
    }
    out.push_str("] as const;\n\n");
    out.push_str(
        "export type CataloguedCapability = (typeof CATALOGUED_CAPABILITIES)[number];\n\n",
    );
    out
}

fn capability_rows() -> Vec<(&'static str, &'static str)> {
    let mut rows = ployz_core::CATALOGUED_CAPABILITY_BINDINGS.to_vec();
    rows.sort_by_key(|(_, wire)| *wire);
    rows
}

fn check_additive_fields_match_rust() {
    let examples = [
        ("ContractDescription", to_value(&contract_description())),
        ("DockerVolume", to_value(&docker_volume())),
        ("DockerVolumeId", to_value(&docker_volume().id)),
        ("DeployIntent", to_value(&deploy_intent())),
        ("PlanOptions", to_value(&PlanOptions::default())),
        ("ServiceAttempt", to_value(&service_attempt())),
    ];
    for (name, shape) in PAYLOADS {
        let Shape::Additive(_) = shape else {
            continue;
        };
        assert!(
            examples.iter().any(|(example, _)| *example == *name),
            "{name} Additive shape has no serde example"
        );
    }
    for (name, value) in examples {
        let fields = additive_fields(name)
            .unwrap_or_else(|| panic!("{name} is missing from Additive payload shapes"));
        let object = value
            .as_object()
            .unwrap_or_else(|| panic!("{name} serde value is not an object"));
        for key in object.keys() {
            assert!(
                fields.iter().any(|(field, _)| field == key),
                "{name} TypeScript fields missing serde key {key}"
            );
        }
        for (field, _) in fields {
            assert!(
                object.contains_key(*field),
                "{name} serde value missing field {field}"
            );
        }
    }
}

fn additive_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    PAYLOADS.iter().find_map(|(payload, shape)| {
        let Shape::Additive(fields) = shape else {
            return None;
        };
        (*payload == name).then_some(*fields)
    })
}

fn contract_description() -> ContractDescription {
    ContractDescription {
        machine_id: machine_id(MACHINE_ID_HEX),
        protocol_major: PROTOCOL_MAJOR,
        daemon_version: "0.1.0".into(),
        capabilities: [CapabilityName::parse(DESCRIBE_CONTRACT_CAPABILITY)
            .expect("catalogued capability names are valid")]
        .into(),
    }
}

fn rpc_error() -> RpcError {
    RpcError {
        code: RpcErrorCode::Unsupported,
        message: "watch is not advertised".into(),
        details: Value::Null,
    }
}

fn docker_volume() -> DockerVolume {
    DockerVolume {
        id: DockerVolumeId {
            machine_id: machine_id(MACHINE_ID_HEX),
            name: DockerVolumeName::parse("data").expect("fixture volume name is valid"),
        },
        driver: "local".into(),
        options: BTreeMap::from([("type".into(), "none".into())]),
        labels: BTreeMap::from([("ployz.managed".into(), "false".into())]),
    }
}

fn service_attempt() -> ServiceAttempt {
    ServiceAttempt {
        name: ServiceName::parse("web").expect("fixture Service Name is valid"),
    }
}

fn deploy_intent() -> DeployIntent {
    DeployIntent::new(Vec::new(), Vec::new(), PlanOptions::default())
}

fn deploy_outcome() -> DeployOutcome<RpcError> {
    DeployOutcome::Success {
        completed: vec![DeployOperation::StopContainer {
            machine_id: machine_id(MACHINE_ID_HEX),
            container_id: container_id(),
        }],
    }
}

fn deploy_outcome_failed() -> DeployOutcome<RpcError> {
    DeployOutcome::Failed {
        completed: Vec::new(),
        failed: FailedOperation::Operation {
            operation: DeployOperation::StopContainer {
                machine_id: machine_id(MACHINE_ID_HEX),
                container_id: container_id(),
            },
            error: rpc_error(),
        },
        unexecuted: Vec::new(),
    }
}

const CONTAINER_ID_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn container_id() -> ContainerId {
    ContainerId::parse(CONTAINER_ID_HEX).expect("fixture Container ID is valid")
}

fn partial_result() -> PartialResult<DockerVolume, RpcError> {
    PartialResult {
        successes: vec![MachineSuccess {
            machine_id: machine_id(MACHINE_ID_HEX),
            value: docker_volume(),
        }],
        failures: vec![MachineFailure {
            machine_id: machine_id(OTHER_MACHINE_ID_HEX),
            error: rpc_error(),
        }],
        omissions: vec![machine_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")],
    }
}

fn machine_id(value: &str) -> MachineId {
    MachineId::parse(value).expect("fixture Machine ID is valid")
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("SDK fixtures serialize")
}

fn with_unknown_field(mut value: Value, key: &str, extra: Value) -> Value {
    value
        .as_object_mut()
        .expect("unknown-field fixtures wrap objects")
        .insert(key.to_owned(), extra);
    value
}

fn with_unknown_field_in(mut value: Value, variant: &str, key: &str, extra: Value) -> Value {
    value
        .get_mut(variant)
        .and_then(Value::as_object_mut)
        .expect("unknown-field fixtures wrap a serde variant object")
        .insert(key.to_owned(), extra);
    value
}

fn pretty_json(value: &Value) -> String {
    let mut encoded = serde_json::to_string_pretty(value).expect("fixtures are valid JSON");
    encoded.push('\n');
    encoded
}

/// Package directory for the `@ployz/sdk` napi artifact.
#[must_use]
pub fn sdk_package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("ployz-sdk")
}

/// Decode a fixture as `T`.
#[must_use]
pub fn decode_fixture<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).expect("fixture matches the Rust type")
}
