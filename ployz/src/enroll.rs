//! Cloud enroll HTTP: POST identity, parse `initialize` / `join` / `not_yet`.

use std::{net::IpAddr, time::Duration};

use ployz_core::{
    AdvertisedEndpoint, CloudEnrollToken, CloudPairing, MachineName, MachineToken,
    WireGuardPublicKey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default `--cloud-url` host. Scheme is added by [`enroll_url`].
pub const DEFAULT_CLOUD_HOST: &str = "ployz.dev";

/// Identity POSTed to enroll. CamelCase wire shape from the Cloud enroll ADR.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollIdentity {
    name: MachineName,
    public_key: WireGuardPublicKey,
    advertised_endpoints: Vec<AdvertisedEndpoint>,
    public_ip: Option<IpAddr>,
}

impl EnrollIdentity {
    /// Build the enroll POST body from the chosen Machine Name and token.
    #[must_use]
    pub fn new(name: MachineName, token: &MachineToken) -> Self {
        Self {
            name,
            public_key: token.public_key,
            advertised_endpoints: token.advertised_endpoints.clone(),
            public_ip: token.public_ip,
        }
    }
}

/// One enroll POST outcome. `join` and `not_yet` are sibling tickets.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrollResponse {
    Initialize {
        pairing: CloudPairing,
    },
    Join {},
    NotYet {
        #[serde(default, rename = "retryAfter")]
        retry_after: Option<u64>,
    },
}

/// Failures building the enroll URL or talking to Cloud enroll.
#[derive(Debug, Error)]
pub enum EnrollError {
    #[error("invalid Cloud enroll URL {0:?}")]
    Url(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Enroll URL: `https://<host>/api/enroll/<token>`. Host without a scheme is HTTPS.
///
/// # Errors
///
/// Returns [`EnrollError::Url`] when `cloud_url` is not a valid URL.
pub fn enroll_url(cloud_url: &str, token: &CloudEnrollToken) -> Result<reqwest::Url, EnrollError> {
    let formatted = match cloud_url.split_once("://") {
        Some(_) => cloud_url.trim_end_matches('/').to_owned(),
        None => format!("https://{}", cloud_url.trim_end_matches('/')),
    };
    let mut url =
        reqwest::Url::parse(&formatted).map_err(|_| EnrollError::Url(formatted.clone()))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| EnrollError::Url(formatted))?;
        path.pop_if_empty();
        path.extend(["api", "enroll", token.as_str()]);
    }
    Ok(url)
}

/// POST identity to enroll.
///
/// # Errors
///
/// Returns [`EnrollError::Http`] when the request fails or the body is not enroll JSON.
pub async fn post(
    url: reqwest::Url,
    identity: &EnrollIdentity,
) -> Result<EnrollResponse, EnrollError> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(url)
        .json(identity)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ployz_core::{
        AdvertisedEndpoint, CloudEnrollToken, CloudPairing, MachineName, MachineRuntime,
        MachineToken, PairingCredential, WireGuardPublicKey,
    };
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{EnrollIdentity, EnrollResponse, enroll_url, post};

    fn token() -> CloudEnrollToken {
        CloudEnrollToken::parse("pmet_test").unwrap()
    }

    fn identity() -> EnrollIdentity {
        EnrollIdentity::new(
            MachineName::parse("edge").unwrap(),
            &MachineToken {
                public_key: WireGuardPublicKey([7; 32]),
                public_ip: Some("203.0.113.1".parse::<IpAddr>().unwrap()),
                advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
                runtime: MachineRuntime::default(),
            },
        )
    }

    #[test]
    fn default_host_without_scheme_posts_https_ployz_dev() {
        assert_eq!(
            enroll_url(super::DEFAULT_CLOUD_HOST, &token())
                .unwrap()
                .as_str(),
            "https://ployz.dev/api/enroll/pmet_test"
        );
    }

    #[test]
    fn host_without_scheme_is_https() {
        assert_eq!(
            enroll_url("example.test", &token()).unwrap().as_str(),
            "https://example.test/api/enroll/pmet_test"
        );
    }

    #[test]
    fn scheme_is_kept_and_trailing_slash_is_trimmed() {
        assert_eq!(
            enroll_url("http://127.0.0.1:9/", &token())
                .unwrap()
                .as_str(),
            "http://127.0.0.1:9/api/enroll/pmet_test"
        );
    }

    #[test]
    fn identity_wire_shape_is_camel_case() {
        assert_eq!(
            serde_json::to_value(identity()).unwrap(),
            json!({
                "name": "edge",
                "publicKey": [7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7],
                "advertisedEndpoints": ["192.0.2.1:51820"],
                "publicIp": "203.0.113.1",
            })
        );
    }

    #[test]
    fn initialize_response_is_cloud_pairing_and_rejects_dial_on_pairing() {
        let pairing = CloudPairing::parse(
            "https://relay.ployz.dev",
            PairingCredential::parse("pairing-secret").unwrap(),
        )
        .unwrap();
        let value = json!({
            "kind": "initialize",
            "pairing": {
                "relayUrl": "https://relay.ployz.dev",
                "secret": "pairing-secret",
            },
        });
        assert_eq!(
            serde_json::from_value::<EnrollResponse>(value).unwrap(),
            EnrollResponse::Initialize { pairing }
        );
        let dial = serde_json::from_value::<EnrollResponse>(json!({
            "kind": "initialize",
            "pairing": {
                "relayUrl": "https://relay.ployz.dev",
                "secret": "pairing-secret",
                "dial": "dial-credential",
            },
        }));
        assert!(dial.unwrap_err().to_string().contains("unknown field"));
    }

    #[test]
    fn join_and_not_yet_are_distinct_kinds() {
        assert_eq!(
            serde_json::from_value::<EnrollResponse>(json!({
                "kind": "join",
                "pairing": { "relayUrl": "https://relay.ployz.dev", "secret": "pairing-secret" },
                "registration": {},
            }))
            .unwrap(),
            EnrollResponse::Join {}
        );
        assert_eq!(
            serde_json::from_value::<EnrollResponse>(json!({
                "kind": "not_yet",
                "retryAfter": 2,
            }))
            .unwrap(),
            EnrollResponse::NotYet {
                retry_after: Some(2)
            }
        );
    }

    #[tokio::test]
    async fn post_sends_identity_to_enroll_path() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = stream.read(&mut buf).await.unwrap();
            let request =
                String::from_utf8_lossy(buf.get(..n).expect("read count fits buffer")).into_owned();
            let payload = json!({
                "kind": "initialize",
                "pairing": {
                    "relayUrl": "http://relay.test",
                    "secret": "pairing-secret",
                },
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            request
        });
        let url = enroll_url(&format!("http://{addr}"), &token()).unwrap();
        let response = post(url, &identity()).await.unwrap();
        let request = server.await.unwrap();
        assert!(
            request.starts_with("POST /api/enroll/pmet_test "),
            "{request}"
        );
        assert!(request.contains("\"name\":\"edge\""), "{request}");
        let EnrollResponse::Initialize { pairing } = response else {
            panic!("expected initialize, got {response:?}");
        };
        assert_eq!(pairing.relay_url(), "http://relay.test");
        assert_eq!(pairing.secret().as_str(), "pairing-secret");
    }
}
