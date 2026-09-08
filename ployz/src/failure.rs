use std::{borrow::Cow, error::Error, fmt, io, process::ExitCode};

use ployz_core::{
    CodecError, ContainerSelectorError, DataLoss, IngressLabelTooLong, MachineSelectorError,
    MachineUpdateError, PartialResult, RpcError, RpcErrorCode, ServiceSelectorError,
    StreamProtocolError, UnconfirmedDataLoss, ValueError,
};

use crate::{
    cloud_enroll,
    compose::ComposeError,
    connect::{ConnectError, TransportError},
    context::{ConfigError, ConnectionError, ContextError},
    deploy::{DeployError, PlanError},
    dns::{DomainRequired, Error as DnsError, NoReachableMachines},
    image::PushError,
    ingress::IngressImageError,
    operator::OperatorError,
    project::ProjectError,
    provisioning::ProvisionError,
    volume::AssignmentError,
};

/// CLI command outcome. `Display` is product stderr. `exit` is silent.
#[derive(Debug)]
pub struct Failure {
    inner: Inner,
    /// Whether this failure is a Ployz bug, decided while the typed error is
    /// still in hand. `Display` only prints the decision, so rendering a failure
    /// into a wider message cannot lose it.
    bug: bool,
}

#[derive(Debug)]
enum Inner {
    Command(Box<dyn Error + Send + Sync>),
    Exit(u8),
}

/// Whether `error` or anything in its source chain is an `Internal` RPC error:
/// a Ployz bug, printed with the report step. A wrapper that holds an
/// `RpcError` or `TransportError` must expose it with `#[source]`, or the bug
/// prints as if the user caused it.
fn is_internal_rpc(error: &(dyn Error + 'static)) -> bool {
    std::iter::successors(Some(error), |error| Error::source(*error)).any(|error| {
        error
            .downcast_ref::<RpcError>()
            .is_some_and(|error| error.code == RpcErrorCode::Internal)
            || error
                .downcast_ref::<TransportError>()
                .is_some_and(|error| error.rpc_code() == RpcErrorCode::Internal)
    })
}

// Skip marker for later capture_exception; not a library error.
#[derive(Debug)]
struct Usage {
    message: Cow<'static, str>,
    cause: Option<Box<dyn Error + Send + Sync>>,
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for Usage {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_ref()
            .map(|cause| &**cause as &(dyn Error + 'static))
    }
}

impl Failure {
    /// A typed error, classified on the way in. Preferred over rendering the
    /// error yourself: `Display` is identical and the code survives.
    pub(crate) fn command(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            bug: is_internal_rpc(&error),
            inner: Inner::Command(Box::new(error)),
        }
    }

    fn text(
        message: Cow<'static, str>,
        cause: Option<Box<dyn Error + Send + Sync>>,
        bug: bool,
    ) -> Self {
        Self {
            inner: Inner::Command(Box::new(Usage { message, cause })),
            bug,
        }
    }

    #[must_use]
    pub fn exit(code: u8) -> Self {
        Self {
            inner: Inner::Exit(code),
            bug: false,
        }
    }

    /// Product text the CLI wrote itself. Text that renders a typed error
    /// belongs in `context` or `Failures`, which keep the code.
    pub fn usage(message: impl Into<Cow<'static, str>>) -> Self {
        Self::text(message.into(), None, false)
    }

    /// Product text that already names `cause`. The classification comes from
    /// `cause`, so stringifying it into `message` cannot turn a bug into what
    /// looks like a user error.
    pub fn context(
        message: impl Into<Cow<'static, str>>,
        cause: impl Error + Send + Sync + 'static,
    ) -> Self {
        let bug = is_internal_rpc(&cause);
        Self::text(message.into(), Some(Box::new(cause)), bug)
    }

    /// Restate this failure inside wider product text. `sentence` sees the
    /// unframed message and the classification carries over, so a bug is still
    /// framed exactly once however many times it is restated.
    #[must_use]
    pub fn wrap(self, sentence: impl FnOnce(&str) -> String) -> Self {
        let message = match &self.inner {
            Inner::Command(error) => error.to_string(),
            Inner::Exit(code) => format!("exit {code}"),
        };
        let bug = self.bug;
        Self::text(sentence(&message).into(), Some(Box::new(self)), bug)
    }

    /// One product line for a follow-on failure. `terminate` prints it once.
    pub fn warned(context: impl fmt::Display, cause: impl Error + Send + Sync + 'static) -> Self {
        Self::context(format!("WARNING: {context}: {cause}."), cause)
    }
}

/// The failures behind a fan-out that did not finish everywhere.
///
/// Enumerating and framing happen together: entries turn into text only inside
/// [`Failures::into_failure`], which still holds every `RpcError` code, so an
/// aggregate cannot reach the user having forgotten that one of its parts was a
/// bug. There is deliberately no `Display`.
#[derive(Debug, Default)]
pub struct Failures {
    entries: Vec<String>,
    bug: bool,
}

impl Failures {
    /// Record what `scope` — a Machine, a Volume, a Global — reported.
    pub fn record(&mut self, scope: impl fmt::Display, error: &(impl Error + 'static)) {
        self.bug |= is_internal_rpc(error);
        self.entries.push(format!("{scope}: {error}"));
    }

    /// Record a failure with no `RpcError` behind it: a target that never
    /// answered, a reason the CLI knows locally.
    pub fn note(&mut self, scope: impl fmt::Display, reason: impl fmt::Display) {
        self.entries.push(format!("{scope}: {reason}"));
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Product text: `sentence` wraps the enumerated failures, framed as a bug
    /// when any of them was one.
    #[must_use]
    pub fn into_failure(self, sentence: impl FnOnce(&str) -> String) -> Failure {
        Failure::text(sentence(&self.entries.join("; ")).into(), None, self.bug)
    }
}

/// Every failure and unanswered target in a fan-out result.
#[must_use]
pub fn partial_failures<T>(result: &PartialResult<T, RpcError>) -> Failures {
    let mut failures = Failures::default();
    for failure in &result.failures {
        failures.record(failure.machine_id, &failure.error);
    }
    for machine_id in &result.omissions {
        failures.note(machine_id, "no terminal response");
    }
    failures
}

pub(crate) fn pass_data_loss_names_message(missing: &[DataLoss]) -> String {
    format!(
        "Additional volume loss is not covered by the confirmation: {}. Rerun to review the updated volume list.",
        missing
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn refusal_from_rpc(error: RpcError) -> Failure {
    match UnconfirmedDataLoss::from_rpc_error(&error) {
        Some(unconfirmed) => {
            Failure::context(pass_data_loss_names_message(&unconfirmed.missing), error)
        }
        None => error.into(),
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            Inner::Command(error) if self.bug => {
                write!(f, "internal error: {error}\n{}", RpcError::REPORT_HINT)
            }
            Inner::Command(error) => error.fmt(f),
            Inner::Exit(code) => write!(f, "exit {code}"),
        }
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.inner {
            Inner::Command(error) => Some(error.as_ref()),
            Inner::Exit(_) => None,
        }
    }
}

#[must_use]
pub fn terminate(result: Result<(), Failure>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure {
            inner: Inner::Exit(code),
            ..
        }) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

macro_rules! from_error {
    ($($t:ty),+ $(,)?) => {
        $(impl From<$t> for Failure {
            fn from(error: $t) -> Self {
                Self::command(error)
            }
        })+
    };
}

from_error!(
    ValueError,
    ContextError,
    ConnectionError,
    MachineSelectorError,
    ServiceSelectorError,
    ContainerSelectorError,
    PlanError,
    ComposeError,
    MachineUpdateError,
    DomainRequired,
    IngressLabelTooLong,
    NoReachableMachines,
    StreamProtocolError,
    ConfigError,
    io::Error,
    serde_json::Error,
    std::num::ParseIntError,
    shell_words::ParseError,
    PushError,
    TransportError,
    CodecError,
    AssignmentError,
    ProvisionError,
    IngressImageError,
    RpcError,
    ProjectError,
    cloud_enroll::Error,
    crate::operator::LogFailure,
);

impl From<ConnectError> for Failure {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "opaque Failure peels Display-changing wrappers; the rest keep the original error"
    )]
    fn from(error: ConnectError) -> Self {
        match error {
            ConnectError::Context(error) => error.into(),
            ConnectError::Value(error) => error.into(),
            error => Self::command(error),
        }
    }
}

impl From<OperatorError> for Failure {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "opaque Failure peels Display-changing wrappers; the rest keep the original error"
    )]
    fn from(error: OperatorError) -> Self {
        match error {
            OperatorError::Connect(error) => error.into(),
            OperatorError::Protocol(error) => error.into(),
            error => Self::command(error),
        }
    }
}

impl From<DeployError> for Failure {
    fn from(error: DeployError) -> Self {
        match error {
            DeployError::Connect(error) => error.into(),
            DeployError::Plan(error) => error.into(),
            DeployError::Project(error) => error.into(),
        }
    }
}

impl From<DnsError> for Failure {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "opaque Failure peels ConnectError; the rest keep the original error"
    )]
    fn from(error: DnsError) -> Self {
        match error {
            DnsError::Connect(error) => error.into(),
            error => Self::command(error),
        }
    }
}
impl From<tonic::Status> for Failure {
    fn from(status: tonic::Status) -> Self {
        TransportError::from(status).into()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;

    use ployz_core::{MachineName, RpcError, RpcErrorCode};
    use serde_json::Value;

    use super::*;

    fn source<E: StdError + 'static>(failure: &Failure) -> &E {
        StdError::source(failure)
            .and_then(|error| error.downcast_ref())
            .expect("command Failure should keep the original error")
    }

    #[test]
    fn missing_config_prints_the_no_config_string() {
        let failure = Failure::from(ContextError::NoConfig);
        assert_eq!(
            failure.to_string(),
            "no Ployz config or local daemon socket is available"
        );
        assert!(matches!(
            source::<ContextError>(&failure),
            ContextError::NoConfig
        ));
        assert_eq!(terminate(Err(failure)), ExitCode::FAILURE);
    }

    #[test]
    fn invalid_machine_name_display_is_stable() {
        let error = MachineName::parse("BAD NAME").unwrap_err();
        let failure = Failure::from(error);
        assert_eq!(
            failure.to_string(),
            "invalid Machine Name \"BAD NAME\": a 1-63 character lowercase DNS label"
        );
        assert_eq!(
            source::<ValueError>(&failure).to_string(),
            "invalid Machine Name \"BAD NAME\": a 1-63 character lowercase DNS label"
        );
        assert_eq!(terminate(Err(failure)), ExitCode::FAILURE);
    }

    #[test]
    fn connect_context_errors_unwrap_to_context() {
        let from_connect = Failure::from(ConnectError::Context(ContextError::NoConfig));
        let from_context = Failure::from(ContextError::NoConfig);
        assert_eq!(from_connect.to_string(), from_context.to_string());
        assert_eq!(
            from_connect.to_string(),
            "no Ployz config or local daemon socket is available"
        );
        assert!(matches!(
            source::<ContextError>(&from_connect),
            ContextError::NoConfig
        ));
        assert!(
            StdError::source(&from_connect)
                .unwrap()
                .downcast_ref::<ConnectError>()
                .is_none()
        );
        assert_eq!(terminate(Err(from_connect)), ExitCode::FAILURE);
    }

    #[test]
    fn exhausted_connections_print_how_many_were_tried() {
        let failure = Failure::from(ConnectError::AllFailed {
            source: crate::context::ConnectionSource::Context("prod".into()),
            attempts: 3,
            setup_retryable: true,
            last: Some(Box::new(ConnectError::Io(io::Error::from(
                io::ErrorKind::ConnectionRefused,
            )))),
        });
        let display = failure.to_string();
        assert!(display.contains("3"), "{display}");
        assert!(
            !display.contains("Os {") && !display.contains("code: 111"),
            "{display}"
        );
    }

    #[test]
    fn connect_keeps_non_peeled_connect_error() {
        let failure = Failure::from(ConnectError::MissingMachineDetails);
        assert_eq!(
            failure.to_string(),
            "connection attempt failed: inspect response omitted Machine details"
        );
        assert!(matches!(
            source::<ConnectError>(&failure),
            ConnectError::MissingMachineDetails
        ));
    }

    #[test]
    fn rpc_error_keeps_the_rpc_error() {
        let failure = Failure::from(RpcError {
            code: RpcErrorCode::Internal,
            message: "boom".into(),
            details: Value::Null,
        });
        assert!(failure.to_string().contains("boom"));
        assert_eq!(source::<RpcError>(&failure).message, "boom");
        assert_eq!(source::<RpcError>(&failure).code, RpcErrorCode::Internal);
    }

    #[test]
    fn usage_is_not_a_library_error() {
        let failure = Failure::usage("nope");
        assert_eq!(failure.to_string(), "nope");
        assert_eq!(source::<Usage>(&failure).to_string(), "nope");
        assert_eq!(terminate(Err(failure)), ExitCode::FAILURE);
    }

    #[test]
    fn warned_follow_on_is_one_line_and_fails() {
        let cause = RpcError {
            code: RpcErrorCode::Unavailable,
            message: "inspect Ingress Proxy Machine 905c7d04: Machine RPC returned: target Machine RPC timed out".into(),
            details: Value::Null,
        };
        let add = Failure::warned(
            "hosted DNS refresh failed after adding the Machine",
            cause.clone(),
        );
        let remove = Failure::warned(
            "hosted DNS refresh failed after removing the Machine",
            cause.clone(),
        );
        assert_eq!(
            add.to_string(),
            "WARNING: hosted DNS refresh failed after adding the Machine: inspect Ingress Proxy Machine 905c7d04: Machine RPC returned: target Machine RPC timed out."
        );
        assert_eq!(
            remove.to_string(),
            "WARNING: hosted DNS refresh failed after removing the Machine: inspect Ingress Proxy Machine 905c7d04: Machine RPC returned: target Machine RPC timed out."
        );
        assert_eq!(add.to_string().matches(&cause.message).count(), 1);
        assert_eq!(terminate(Err(remove)), ExitCode::FAILURE);
    }

    fn rpc(code: RpcErrorCode) -> Failure {
        Failure::from(RpcError {
            code,
            message: "boom".into(),
            details: Value::Null,
        })
    }

    #[test]
    fn internal_failures_are_framed_as_bugs_with_the_report_step() {
        let framed = rpc(RpcErrorCode::Internal).to_string();
        assert!(framed.contains("boom"), "{framed}");
        assert!(framed.contains("bug"), "{framed}");
        assert!(framed.contains("ployz version"), "{framed}");

        let restated = rpc(RpcErrorCode::Internal)
            .wrap(|details| format!("startup incomplete: {details}"))
            .to_string();
        assert!(restated.contains("startup incomplete"), "{restated}");
        assert_eq!(restated.matches("ployz version").count(), 1, "{restated}");

        let user = rpc(RpcErrorCode::NotFound).to_string();
        assert!(!user.contains("bug"), "{user}");
        assert!(!user.contains("ployz version"), "{user}");
    }

    /// The framing decision is data, so nothing may render a typed error into
    /// product text and hand the text to `usage`: `context`, `wrap` and
    /// `Failures` all keep the code. This scan is the enforcement, so a new
    /// stringifying site fails here instead of printing a bug as a user error.
    #[test]
    fn usage_never_swallows_a_typed_error() {
        const RENDERED: [&str; 5] = ["{error", "{err}", "{cause", ".to_string()", ".message"];

        let mut offenders = Vec::new();
        let mut pending = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                pending.extend(
                    std::fs::read_dir(&path)
                        .expect("readable source directory")
                        .map(|entry| entry.expect("readable entry").path()),
                );
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("readable source file");
            for (line, argument) in usage_arguments(&source) {
                // Text with no literal in it was rendered somewhere else, which is
                // where the code was lost; `text.trim()` reads as innocently as
                // `format!("{error}")` and costs the same framing.
                if RENDERED.iter().any(|rendered| argument.contains(rendered))
                    || !argument.contains('"')
                {
                    offenders.push(format!("{}:{line}", path.display()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "usage() renders a typed error at {offenders:?}: use context, wrap or Failures so the framing survives"
        );
    }

    /// Every `usage(` argument in `source`, with the line it starts on.
    fn usage_arguments(source: &str) -> Vec<(usize, String)> {
        let mut arguments = Vec::new();
        let mut rest = source;
        let mut consumed = 0;
        while let Some(start) = rest.find("::usage(") {
            let open = consumed + start + "::usage".len();
            let mut depth = 0_usize;
            let mut end = open;
            for (offset, character) in source[open..].char_indices() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let line = source[..open].lines().count();
            arguments.push((line, source[open..end].to_owned()));
            consumed = open + 1;
            rest = &source[consumed..];
        }
        arguments
    }

    #[test]
    fn exit_is_not_a_printed_command_failure() {
        assert!(StdError::source(&Failure::exit(3)).is_none());
        assert_eq!(terminate(Err(Failure::exit(3))), ExitCode::from(3));
        assert_eq!(terminate(Ok(())), ExitCode::SUCCESS);
        assert_eq!(terminate(Err(Failure::usage("nope"))), ExitCode::FAILURE);
    }
}
