//! Ambiguous-service failure through miette.

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum SpikeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("service name `{name}` is ambiguous")]
    #[diagnostic(
        code(ployz::cli::ambiguous_service),
        help("pass the service id, or qualify as project/name")
    )]
    AmbiguousService {
        name: &'static str,
        #[related]
        matches: Vec<ServiceMatch>,
    },

    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Error, Diagnostic, Serialize, Clone, Copy)]
#[error("{identity}  {service_id}")]
pub struct ServiceMatch {
    pub identity: &'static str,
    pub service_id: &'static str,
}

#[derive(Serialize)]
pub struct ErrorView {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: &'static str,
    pub matches: &'static [ServiceMatch],
}

const AMBIGUOUS_MATCHES: &[ServiceMatch] = &[
    ServiceMatch {
        identity: "web/api",
        service_id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
    },
    ServiceMatch {
        identity: "jobs/api",
        service_id: "d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1",
    },
];

#[must_use]
pub fn ambiguous_api() -> SpikeError {
    SpikeError::AmbiguousService {
        name: "api",
        matches: AMBIGUOUS_MATCHES.to_vec(),
    }
}

#[must_use]
pub fn error_view() -> ErrorView {
    ErrorView {
        kind: "ambiguous_service",
        name: "api",
        matches: AMBIGUOUS_MATCHES,
    }
}

impl SpikeError {
    pub fn message(error: impl ToString) -> Self {
        Self::Message(error.to_string())
    }
}
