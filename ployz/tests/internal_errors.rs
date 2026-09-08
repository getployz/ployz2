//! Internal (bug) failures are framed as bugs at the `Failure` seam; user
//! failures are not. Behavioral: the framing names the report step, the
//! original message survives, and the exact wording is free to change.

use ployz::{
    connect::ConnectError,
    failure::{Failure, partial_failures},
    image::PushError,
    operator::{LogError, LogFailure},
};
use ployz_core::{MachineFailure, MachineId, PartialResult, RpcError, RpcErrorCode};
use serde_json::Value;

fn rpc_error(code: RpcErrorCode) -> RpcError {
    RpcError {
        code,
        message: "widget exploded".into(),
        details: Value::Null,
    }
}

fn assert_framed_as_bug(failure: &Failure) {
    let display = failure.to_string();
    assert!(display.contains("widget exploded"), "{display}");
    assert!(display.contains("bug"), "{display}");
    assert!(display.contains("ployz version"), "{display}");
}

fn assert_not_framed_as_bug(failure: &Failure) {
    let display = failure.to_string();
    assert!(display.contains("widget exploded"), "{display}");
    assert!(!display.contains("bug"), "{display}");
    assert!(!display.contains("ployz version"), "{display}");
}

#[test]
fn internal_rpc_errors_are_framed_as_bugs_with_the_report_step() {
    assert_framed_as_bug(&Failure::from(rpc_error(RpcErrorCode::Internal)));
}

#[test]
fn internal_errors_inside_wrappers_are_still_framed() {
    assert_framed_as_bug(&Failure::from(ConnectError::Remote(rpc_error(
        RpcErrorCode::Internal,
    ))));
    assert_framed_as_bug(&Failure::from(tonic::Status::internal("widget exploded")));
    assert_framed_as_bug(&Failure::from(PushError::Unregistry(ConnectError::Remote(
        rpc_error(RpcErrorCode::Internal),
    ))));
}

#[test]
fn internal_errors_stringified_into_context_are_still_framed() {
    let remote = ConnectError::Remote(rpc_error(RpcErrorCode::Internal));
    assert_framed_as_bug(&Failure::context(
        format!("deployment incomplete: {remote}; rerun the command"),
        remote,
    ));
    assert_framed_as_bug(&Failure::from(LogFailure {
        message: "web: widget exploded".into(),
        source: Some(LogError::from(tonic::Status::internal("widget exploded"))),
    }));
}

fn fan_out(code: RpcErrorCode) -> PartialResult<(), RpcError> {
    PartialResult {
        successes: Vec::new(),
        failures: vec![MachineFailure {
            machine_id: MachineId::parse("a".repeat(32)).unwrap(),
            error: rpc_error(code),
        }],
        omissions: Vec::new(),
    }
}

fn summary(result: &PartialResult<(), RpcError>) -> Failure {
    partial_failures(result)
        .into_failure(|details| format!("one or more Machines failed: {details}"))
}

#[test]
fn internal_failures_inside_a_fan_out_are_framed_as_bugs() {
    assert_framed_as_bug(&summary(&fan_out(RpcErrorCode::Internal)));
    assert_not_framed_as_bug(&summary(&fan_out(RpcErrorCode::NotFound)));
}

#[test]
fn user_errors_are_not_framed_as_bugs() {
    for code in [RpcErrorCode::NotFound, RpcErrorCode::InvalidArgument] {
        assert_not_framed_as_bug(&Failure::from(rpc_error(code.clone())));
        assert_not_framed_as_bug(&Failure::from(ConnectError::Remote(rpc_error(code))));
    }
    assert_not_framed_as_bug(&Failure::from(tonic::Status::not_found("widget exploded")));
}
