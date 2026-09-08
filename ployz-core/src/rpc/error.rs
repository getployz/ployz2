//! The typed RPC failure every consumer branches on, and its wire shape.

use std::borrow::Cow;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use thiserror::Error;
use ts_rs::TS;

crate::value::open_string_enum!(RpcErrorCode, Unknown {
    InvalidArgument => "invalid_argument",
    NotFound => "not_found",
    Ambiguous => "ambiguous",
    Unsupported => "unsupported",
    Unavailable => "unavailable",
    Conflict => "conflict",
    Internal => "internal",
    Unauthenticated => "unauthenticated",
});

/// A failed RPC as every consumer sees it: `code` to branch on, `message` to
/// read, and `details` holding the objects a consumer acts on as data.
#[derive(Clone, Debug, Error, PartialEq, Deserialize, TS)]
#[error("{message}")]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

impl RpcError {
    /// Key under which an `Internal` error carries [`Self::REPORT_HINT`] in `details`.
    pub const REPORT_KEY: &'static str = "report";

    /// `details.report` of every `Internal` error on the wire: the failure is a
    /// Ployz bug, where to file it, and what to include.
    pub const REPORT_HINT: &'static str = "This is a Ployz bug. Report it at https://github.com/getployz/ployz2/issues and include the output of `ployz version`.";

    /// The bug-report hint an `Internal` error carries in `details.report`.
    /// Derived from `code`, never read back from `details`: a value decoded
    /// under the reserved key cannot pass itself off as the report path.
    #[must_use]
    pub fn report_hint(&self) -> Option<&'static str> {
        (self.code == RpcErrorCode::Internal).then_some(Self::REPORT_HINT)
    }
}

// The hint is derived from `code`, so it is added once here rather than by every
// producer. The wire shape is deliberately richer than the in-memory one: an
// `Internal` error with `Null` details does not round-trip to an equal value.
impl Serialize for RpcError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            code: &'a RpcErrorCode,
            message: &'a str,
            details: Cow<'a, Value>,
        }

        let details = match self.report_hint() {
            None => Cow::Borrowed(&self.details),
            Some(hint) => {
                // `report` belongs to this encoder, so the hint always wins. Typed
                // siblings keep their paths: only what cannot sit beside the hint —
                // a scalar or array `details`, or a producer value already under the
                // reserved key — moves one level down, and never over a field the
                // producer put there.
                let mut fields = self.details.as_object().cloned().unwrap_or_default();
                let displaced = if self.details.is_object() {
                    fields
                        .insert(Self::REPORT_KEY.to_owned(), hint.into())
                        .filter(|prior| prior.as_str() != Some(hint))
                } else {
                    fields.insert(Self::REPORT_KEY.to_owned(), hint.into());
                    Some(self.details.clone()).filter(|details| !details.is_null())
                };
                if let Some(displaced) = displaced {
                    // Displaced data collects under `details`. A producer that owns
                    // that key too keeps its value one level further down, so
                    // colliding keys cost depth rather than data.
                    let occupied = fields.remove("details");
                    fields.insert(
                        "details".to_owned(),
                        occupied.map_or_else(
                            || displaced.clone(),
                            |occupied| {
                                Value::Object(Map::from_iter([
                                    (Self::REPORT_KEY.to_owned(), displaced.clone()),
                                    ("details".to_owned(), occupied),
                                ]))
                            },
                        ),
                    );
                }
                Cow::Owned(Value::Object(fields))
            }
        };
        Wire {
            code: &self.code,
            message: &self.message,
            details,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod rpc_error_wire {
    use super::*;
    use serde_json::json;

    fn error(code: RpcErrorCode, details: Value) -> RpcError {
        RpcError {
            code,
            message: "boom".into(),
            details,
        }
    }

    #[test]
    fn internal_errors_carry_a_report_hint_on_the_wire() {
        let wire = serde_json::to_value(error(RpcErrorCode::Internal, Value::Null)).unwrap();
        let hint = wire
            .pointer("/details/report")
            .and_then(Value::as_str)
            .expect("report hint")
            .to_owned();
        assert!(hint.contains("bug"), "{hint}");
        assert!(hint.contains("ployz version"), "{hint}");
        assert_eq!(wire.get("message"), Some(&Value::from("boom")));
        assert_eq!(wire.get("code"), Some(&Value::from("internal")));

        let decoded: RpcError = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.report_hint(), Some(hint.as_str()));
    }

    #[test]
    fn internal_errors_keep_their_typed_details_beside_the_hint() {
        let wire = serde_json::to_value(error(
            RpcErrorCode::Internal,
            json!({ "reason": "start_failed" }),
        ))
        .unwrap();
        assert_eq!(
            wire.pointer("/details/reason"),
            Some(&Value::from("start_failed"))
        );
        assert!(
            wire.pointer("/details/report")
                .is_some_and(Value::is_string)
        );
    }

    #[test]
    fn a_conflicting_report_value_never_shadows_the_hint() {
        let wire = serde_json::to_value(error(
            RpcErrorCode::Internal,
            json!({ "report": "retry later", "reason": "start_failed" }),
        ))
        .unwrap();
        assert_eq!(
            wire.pointer("/details/report").and_then(Value::as_str),
            Some(RpcError::REPORT_HINT)
        );
        assert_eq!(
            wire.pointer("/details/details"),
            Some(&json!("retry later"))
        );
        assert_eq!(
            wire.pointer("/details/reason"),
            Some(&json!("start_failed")),
            "typed siblings keep their paths"
        );
    }

    #[test]
    fn a_displaced_report_survives_a_producer_owned_details_key() {
        let wire = serde_json::to_value(error(
            RpcErrorCode::Internal,
            json!({ "report": "producer text", "details": { "typed": 1 } }),
        ))
        .unwrap();
        assert_eq!(
            wire.pointer("/details/report").and_then(Value::as_str),
            Some(RpcError::REPORT_HINT)
        );
        assert_eq!(
            wire.pointer("/details/details/report"),
            Some(&json!("producer text"))
        );
        assert_eq!(
            wire.pointer("/details/details/details"),
            Some(&json!({ "typed": 1 }))
        );
    }

    #[test]
    fn internal_errors_with_unkeyed_details_keep_them_beside_the_hint() {
        let wire =
            serde_json::to_value(error(RpcErrorCode::Internal, json!(["start_failed"]))).unwrap();
        assert!(
            wire.pointer("/details/report")
                .is_some_and(Value::is_string),
            "{wire}"
        );
        assert_eq!(
            wire.pointer("/details/details"),
            Some(&json!(["start_failed"]))
        );
    }

    #[test]
    fn user_errors_carry_no_report_hint() {
        for code in [RpcErrorCode::NotFound, RpcErrorCode::InvalidArgument] {
            let wire = serde_json::to_value(error(code, Value::Null)).unwrap();
            assert!(wire.pointer("/details/report").is_none(), "{wire}");
            let decoded: RpcError = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded.report_hint(), None);
        }
    }
}
