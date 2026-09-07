//! The typed RPC failure every consumer branches on, and its wire shape.

use std::borrow::Cow;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
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
    #[must_use]
    pub fn report_hint(&self) -> Option<&str> {
        self.details.get(Self::REPORT_KEY)?.as_str()
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

        // Only the two shapes producers emit today get the hint; anything else crosses untouched.
        let hint_missing = self.code == RpcErrorCode::Internal
            && self.report_hint().is_none()
            && (self.details.is_null() || self.details.is_object());
        let details = if hint_missing {
            let mut fields = self.details.as_object().cloned().unwrap_or_default();
            fields.insert(Self::REPORT_KEY.into(), Self::REPORT_HINT.into());
            Cow::Owned(Value::Object(fields))
        } else {
            Cow::Borrowed(&self.details)
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
    fn user_errors_carry_no_report_hint() {
        for code in [RpcErrorCode::NotFound, RpcErrorCode::InvalidArgument] {
            let wire = serde_json::to_value(error(code, Value::Null)).unwrap();
            assert!(wire.pointer("/details/report").is_none(), "{wire}");
            let decoded: RpcError = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded.report_hint(), None);
        }
    }
}
