//! Cloud enroll HTTP: POST identity, consume `join`.

use std::{net::IpAddr, time::Duration};

use ployz_core::{AdvertisedEndpoint, CloudPairing, MachineName, Registered, WireGuardPublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_RETRY_AFTER: u64 = 2;

/// Failures talking to Cloud enroll.
#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("enroll HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("enroll initialize is not implemented")]
    Initialize,
    #[error("unexpected enroll response kind {0:?}")]
    UnexpectedKind(String),
}

/// Identity POSTed to `POST /api/enroll/<token>`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrollIdentity {
    pub name: MachineName,
    pub public_key: WireGuardPublicKey,
    pub advertised_endpoints: Vec<AdvertisedEndpoint>,
    pub public_ip: Option<IpAddr>,
}

/// Successful enroll `join` payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Join {
    pub pairing: CloudPairing,
    pub registration: Registered,
}

/// Enroll outcome this command understands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Response {
    Join(Box<Join>),
    NotYet { retry_after: Duration },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollWire {
    kind: String,
    pairing: Option<CloudPairing>,
    registration: Option<Registered>,
    #[serde(default, rename = "retryAfter")]
    retry_after: Option<u64>,
}

/// Enroll URL: host without a scheme is HTTPS.
#[must_use]
pub(crate) fn enroll_url(cloud_url: &str, token: &str) -> String {
    let host = cloud_url.trim().trim_end_matches('/');
    let origin = if host.contains("://") {
        host.to_owned()
    } else {
        format!("https://{host}")
    };
    format!("{origin}/api/enroll/{token}")
}

/// POST identity until Cloud returns `join` or a terminal failure.
///
/// # Errors
///
/// HTTP, unexpected JSON, or an `initialize` response (sibling ticket).
pub(crate) async fn enroll_join(url: &str, identity: &EnrollIdentity) -> Result<Join, Error> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    loop {
        match post_once(&http, url, identity).await? {
            Response::Join(join) => return Ok(*join),
            Response::NotYet { retry_after } => tokio::time::sleep(retry_after).await,
        }
    }
}

async fn post_once(
    http: &reqwest::Client,
    url: &str,
    identity: &EnrollIdentity,
) -> Result<Response, Error> {
    let sent = http.post(url).json(identity).send().await?;
    let status = sent.status();
    let bytes = sent.bytes().await?;
    if !status.is_success() {
        return Err(Error::Status {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    parse_enroll(&bytes)
}

fn parse_enroll(bytes: &[u8]) -> Result<Response, Error> {
    let wire: EnrollWire = serde_json::from_slice(bytes)?;
    match wire.kind.as_str() {
        "join" => {
            let Some(pairing) = wire.pairing else {
                return Err(Error::UnexpectedKind("join without pairing".into()));
            };
            let Some(registration) = wire.registration else {
                return Err(Error::UnexpectedKind("join without registration".into()));
            };
            Ok(Response::Join(Box::new(Join {
                pairing,
                registration,
            })))
        }
        "not_yet" => Ok(Response::NotYet {
            retry_after: Duration::from_secs(wire.retry_after.unwrap_or(DEFAULT_RETRY_AFTER)),
        }),
        "initialize" => Err(Error::Initialize),
        other => Err(Error::UnexpectedKind(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use ployz_core::{Machine, MachineId, MachineName, PairingCredential, Registered};
    use serde_json::json;

    use super::*;

    fn pairing() -> CloudPairing {
        CloudPairing::parse(
            "https://relay.example.invalid",
            PairingCredential::parse("pairing-secret").unwrap(),
        )
        .unwrap()
    }

    fn registration() -> Registered {
        Registered {
            assigned_machine: Machine {
                id: MachineId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
                name: MachineName::parse("joiner").unwrap(),
                subnet: "10.210.1.0/24".parse().unwrap(),
                management_address: ployz_core::ManagementAddress("fd00::1".parse().unwrap()),
                public_key: WireGuardPublicKey([1; 32]),
                public_ip: None,
                advertised_endpoints: Vec::new(),
                runtime: Default::default(),
            },
            visible_peers: Vec::new(),
            target_versions: Default::default(),
        }
    }

    #[test]
    fn host_without_scheme_is_https_under_ployz_dev() {
        assert_eq!(
            enroll_url("ployz.dev", "pmet_test"),
            "https://ployz.dev/api/enroll/pmet_test"
        );
        assert_eq!(
            enroll_url("http://127.0.0.1:9", "pmet_test"),
            "http://127.0.0.1:9/api/enroll/pmet_test"
        );
    }

    #[test]
    fn join_payload_is_pairing_plus_registration() {
        let value = json!({
            "kind": "join",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
            },
            "registration": registration(),
        });
        let Response::Join(join) =
            parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap()
        else {
            panic!("expected join");
        };
        assert_eq!(join.pairing, pairing());
        assert_eq!(join.registration, registration());
    }

    #[test]
    fn pairing_with_a_dial_field_is_rejected() {
        let value = json!({
            "kind": "join",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
                "dial": "dial-credential",
            },
            "registration": registration(),
        });
        let error = parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn top_level_dial_is_rejected() {
        let value = json!({
            "kind": "join",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
            },
            "registration": registration(),
            "dial": "dial-credential",
        });
        let error = parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn initialize_is_refused() {
        let value = json!({
            "kind": "initialize",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
            },
        });
        assert!(matches!(
            parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap_err(),
            Error::Initialize
        ));
    }

    #[test]
    fn not_yet_defaults_retry_after_to_two_seconds() {
        let Response::NotYet { retry_after } = parse_enroll(br#"{"kind":"not_yet"}"#).unwrap()
        else {
            panic!("expected not_yet");
        };
        assert_eq!(retry_after, Duration::from_secs(2));
    }
}
