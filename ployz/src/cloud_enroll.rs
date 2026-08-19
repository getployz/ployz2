//! Cloud enroll HTTP: POST identity, consume `initialize` / `join`.

use std::{net::IpAddr, time::Duration};

use ployz_core::{
    AdvertisedEndpoint, CloudEnrollToken, CloudPairing, MachineName, MachineToken, Registered,
    WireGuardPublicKey,
};
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
}

/// Identity POSTed to `POST /api/enroll/<token>`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrollIdentity {
    name: MachineName,
    public_key: WireGuardPublicKey,
    advertised_endpoints: Vec<AdvertisedEndpoint>,
    public_ip: Option<IpAddr>,
}

impl EnrollIdentity {
    pub(crate) fn from_machine_token(name: MachineName, token: &MachineToken) -> Self {
        Self {
            name,
            public_key: token.public_key,
            advertised_endpoints: token.advertised_endpoints.clone(),
            public_ip: token.public_ip,
        }
    }
}

/// Successful enroll `join` payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Join {
    pub pairing: CloudPairing,
    pub registration: Registered,
}

/// Enroll outcome after `not_yet` retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Join(Box<Join>),
    Initialize { pairing: CloudPairing },
}

/// One enroll POST body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Response {
    Join(Box<Join>),
    Initialize { pairing: CloudPairing },
    NotYet { retry_after: Duration },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EnrollWire {
    Join {
        pairing: CloudPairing,
        registration: Box<Registered>,
    },
    NotYet {
        #[serde(default, rename = "retryAfter")]
        retry_after: Option<u64>,
    },
    Initialize {
        pairing: CloudPairing,
    },
}

/// Enroll URL: host without a scheme is HTTPS.
#[must_use]
pub(crate) fn enroll_url(cloud_url: &str, token: &CloudEnrollToken) -> String {
    format!("{}/api/enroll/{}", cloud_origin(cloud_url), token.as_str())
}

/// Hosted DNS API on the same Cloud host as enroll. Not `dns.uncloud.run`.
#[must_use]
pub(crate) fn dns_endpoint(cloud_url: &str) -> String {
    format!("{}/api/dns/v1", cloud_origin(cloud_url))
}

fn cloud_origin(cloud_url: &str) -> String {
    let host = cloud_url.trim().trim_end_matches('/');
    if host.contains("://") {
        host.to_owned()
    } else {
        format!("https://{host}")
    }
}

/// POST identity until Cloud returns `initialize` or `join`.
///
/// # Errors
///
/// HTTP or unexpected JSON.
pub(crate) async fn enroll(url: &str, identity: &EnrollIdentity) -> Result<Outcome, Error> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    loop {
        match post_once(&http, url, identity).await? {
            Response::Join(join) => return Ok(Outcome::Join(join)),
            Response::Initialize { pairing } => return Ok(Outcome::Initialize { pairing }),
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
    match serde_json::from_slice::<EnrollWire>(bytes)? {
        EnrollWire::Join {
            pairing,
            registration,
        } => Ok(Response::Join(Box::new(Join {
            pairing,
            registration: *registration,
        }))),
        EnrollWire::NotYet { retry_after } => Ok(Response::NotYet {
            retry_after: Duration::from_secs(retry_after.unwrap_or(DEFAULT_RETRY_AFTER)),
        }),
        EnrollWire::Initialize { pairing } => Ok(Response::Initialize { pairing }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use ployz_core::{
        AdvertisedEndpoint, Machine, MachineId, MachineName, MachineToken, PairingCredential,
        Registered,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

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

    fn token() -> CloudEnrollToken {
        CloudEnrollToken::parse("pmet_test").unwrap()
    }

    #[test]
    fn host_without_scheme_is_https_under_ployz_dev() {
        assert_eq!(
            enroll_url("ployz.dev", &token()),
            "https://ployz.dev/api/enroll/pmet_test"
        );
        assert_eq!(
            enroll_url("http://127.0.0.1:9", &token()),
            "http://127.0.0.1:9/api/enroll/pmet_test"
        );
    }

    #[test]
    fn hosted_dns_is_on_the_same_cloud_host_not_uncloud_run() {
        assert_eq!(dns_endpoint("ployz.dev"), "https://ployz.dev/api/dns/v1");
        assert_eq!(
            dns_endpoint("http://127.0.0.1:9"),
            "http://127.0.0.1:9/api/dns/v1"
        );
        assert!(!dns_endpoint("ployz.dev").contains("dns.uncloud.run"));
    }

    #[test]
    fn join_payload_is_pairing_plus_registration() {
        let value = serde_json::json!({
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
        let value = serde_json::json!({
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
        let value = serde_json::json!({
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
    fn initialize_payload_is_cloud_pairing() {
        let value = serde_json::json!({
            "kind": "initialize",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
            },
        });
        let Response::Initialize { pairing: got } =
            parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap()
        else {
            panic!("expected initialize");
        };
        assert_eq!(got, pairing());
    }

    #[test]
    fn initialize_pairing_with_a_dial_field_is_rejected() {
        let value = serde_json::json!({
            "kind": "initialize",
            "pairing": {
                "relayUrl": "https://relay.example.invalid",
                "secret": "pairing-secret",
                "dial": "dial-credential",
            },
        });
        let error = parse_enroll(serde_json::to_vec(&value).unwrap().as_slice()).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn not_yet_defaults_retry_after_to_two_seconds() {
        let Response::NotYet { retry_after } = parse_enroll(br#"{"kind":"not_yet"}"#).unwrap()
        else {
            panic!("expected not_yet");
        };
        assert_eq!(retry_after, Duration::from_secs(2));
    }

    #[test]
    fn not_yet_honors_retry_after_seconds() {
        let Response::NotYet { retry_after } =
            parse_enroll(br#"{"kind":"not_yet","retryAfter":5}"#).unwrap()
        else {
            panic!("expected not_yet");
        };
        assert_eq!(retry_after, Duration::from_secs(5));
    }

    fn waiter_identity() -> EnrollIdentity {
        EnrollIdentity::from_machine_token(
            MachineName::parse("waiter").unwrap(),
            &MachineToken {
                public_key: WireGuardPublicKey([7; 32]),
                public_ip: Some("192.0.2.7".parse().unwrap()),
                advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.7:51820".parse().unwrap())],
                runtime: Default::default(),
            },
        )
    }

    fn join_body() -> serde_json::Value {
        serde_json::json!({
            "kind": "join",
            "pairing": pairing(),
            "registration": registration(),
        })
    }

    fn initialize_body() -> serde_json::Value {
        serde_json::json!({
            "kind": "initialize",
            "pairing": pairing(),
        })
    }

    #[tokio::test]
    async fn not_yet_retries_until_join_with_the_same_identity() {
        let script = EnrollScript::start([
            serde_json::json!({"kind": "not_yet", "retryAfter": 0}),
            join_body(),
        ])
        .await;
        let identity = waiter_identity();
        let Outcome::Join(join) = enroll(&script.url, &identity).await.unwrap() else {
            panic!("expected join");
        };
        assert_eq!(join.registration, registration());
        assert_same_identity(&script.posts(), &identity);
    }

    #[tokio::test]
    async fn not_yet_retries_until_initialize_with_the_same_identity() {
        let script = EnrollScript::start([
            serde_json::json!({"kind": "not_yet", "retryAfter": 0}),
            initialize_body(),
        ])
        .await;
        let identity = waiter_identity();
        let Outcome::Initialize { pairing: got } = enroll(&script.url, &identity).await.unwrap()
        else {
            panic!("expected initialize");
        };
        assert_eq!(got, pairing());
        assert_same_identity(&script.posts(), &identity);
    }

    #[tokio::test]
    async fn same_public_key_can_receive_initialize_again() {
        let script = EnrollScript::start([initialize_body()]).await;
        let identity = waiter_identity();
        let first = enroll(&script.url, &identity).await.unwrap();
        let second = enroll(&script.url, &identity).await.unwrap();
        assert_eq!(first, Outcome::Initialize { pairing: pairing() });
        assert_eq!(first, second);
        assert_same_identity(&script.posts(), &identity);
    }

    fn assert_same_identity(posts: &[serde_json::Value], identity: &EnrollIdentity) {
        let expected = serde_json::to_value(identity).unwrap();
        assert_eq!(posts, &[expected.clone(), expected]);
    }

    struct EnrollScript {
        url: String,
        posts: Arc<Mutex<Vec<serde_json::Value>>>,
        _server: tokio::task::JoinHandle<()>,
    }

    impl EnrollScript {
        async fn start(bodies: impl IntoIterator<Item = serde_json::Value>) -> Self {
            let queue: VecDeque<Vec<u8>> = bodies
                .into_iter()
                .map(|body| serde_json::to_vec(&body).unwrap())
                .collect();
            assert!(!queue.is_empty());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let posts = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&posts);
            let remaining = Arc::new(Mutex::new(queue));
            let server = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let mut buf = vec![0; 8192];
                    let n = stream.read(&mut buf).await.unwrap();
                    let raw = buf.get(..n).expect("read count is in bounds");
                    recorded.lock().unwrap().push(json_body(raw));
                    let body = {
                        let mut remaining = remaining.lock().unwrap();
                        let body = remaining.pop_front().expect("scripted enroll has a body");
                        if remaining.is_empty() {
                            remaining.push_back(body.clone());
                        }
                        body
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(header.as_bytes()).await.unwrap();
                    stream.write_all(&body).await.unwrap();
                }
            });
            Self {
                url: format!("http://{address}"),
                posts,
                _server: server,
            }
        }

        fn posts(&self) -> Vec<serde_json::Value> {
            self.posts.lock().unwrap().clone()
        }
    }

    fn json_body(raw: &[u8]) -> serde_json::Value {
        let sep = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("HTTP request has a header separator");
        serde_json::from_slice(raw.get(sep + 4..).expect("body follows headers")).unwrap()
    }
}
