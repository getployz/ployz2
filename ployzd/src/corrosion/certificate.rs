//! Certificate row body held in the replicated store.

use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use ployz_core::{IssuanceClock, IssuanceFailure};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Error;

/// Certificate and private key held in cluster state for one Ingress Hostname.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CertificateMaterial {
    certificate: String,
    private_key: String,
}

impl CertificateMaterial {
    #[must_use]
    pub fn new(certificate: impl Into<String>, private_key: impl Into<String>) -> Option<Self> {
        let certificate = certificate.into();
        let private_key = private_key.into();
        (!certificate.is_empty() && !private_key.is_empty()).then_some(Self {
            certificate,
            private_key,
        })
    }

    #[must_use]
    pub fn certificate(&self) -> &str {
        &self.certificate
    }

    #[must_use]
    pub fn private_key(&self) -> &str {
        &self.private_key
    }
}

/// HTTP-01 token and key authorization held in cluster state for one Ingress Hostname.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateChallenge {
    token: String,
    response: String,
}

impl CertificateChallenge {
    #[must_use]
    pub fn new(token: impl Into<String>, response: impl Into<String>) -> Option<Self> {
        let token = token.into();
        let response = response.into();
        (!token.is_empty() && !response.is_empty()).then_some(Self { token, response })
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn response(&self) -> &str {
        &self.response
    }
}

/// Refusal reason plus the shared clock. Absent unless every field is present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateBackoff {
    last_error: String,
    clock: IssuanceClock,
}

impl CertificateBackoff {
    #[must_use]
    pub fn new(last_error: impl Into<String>, clock: IssuanceClock) -> Option<Self> {
        let last_error = last_error.into();
        (!last_error.is_empty()).then_some(Self { last_error, clock })
    }

    #[must_use]
    pub fn last_error(&self) -> &str {
        &self.last_error
    }

    #[must_use]
    pub fn clock(&self) -> IssuanceClock {
        self.clock
    }
}

/// Replicated certificate row for one Ingress Hostname.
#[derive(Clone, Default, Debug, Eq, PartialEq)]
pub struct CertificateRow {
    pub(crate) material: Option<CertificateMaterial>,
    pub(crate) challenge: Option<CertificateChallenge>,
    pub(crate) backoff: Option<CertificateBackoff>,
}

impl CertificateRow {
    /// Material and challenge without a refusal clock.
    #[must_use]
    pub fn from_parts(
        material: Option<CertificateMaterial>,
        challenge: Option<CertificateChallenge>,
    ) -> Self {
        Self {
            material,
            challenge,
            backoff: None,
        }
    }

    #[must_use]
    pub fn with_backoff(mut self, last_error: impl Into<String>, clock: IssuanceClock) -> Self {
        self.backoff = CertificateBackoff::new(last_error, clock);
        self
    }

    #[must_use]
    pub fn material(&self) -> Option<&CertificateMaterial> {
        self.material.as_ref()
    }

    #[must_use]
    pub fn challenge(&self) -> Option<&CertificateChallenge> {
        self.challenge.as_ref()
    }

    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.backoff.as_ref().map(CertificateBackoff::last_error)
    }

    #[must_use]
    pub fn clock(&self) -> Option<IssuanceClock> {
        self.backoff.as_ref().map(CertificateBackoff::clock)
    }

    pub(crate) fn decode(encoded: &str) -> Result<Self, Error> {
        if encoded.is_empty() {
            return Ok(Self::default());
        }
        let body: CertificateBody = serde_json::from_str(encoded)?;
        let backoff = decode_backoff(&body);
        Ok(Self {
            material: CertificateMaterial::new(body.certificate, body.private_key),
            challenge: CertificateChallenge::new(body.challenge_token, body.challenge_response),
            backoff,
        })
    }

    pub(crate) fn encode(&self) -> Result<String, Error> {
        Ok(serde_json::to_string(&json!({
            "certificate": self.material.as_ref().map(CertificateMaterial::certificate).unwrap_or(""),
            "private_key": self.material.as_ref().map(CertificateMaterial::private_key).unwrap_or(""),
            "challenge_token": self.challenge.as_ref().map(CertificateChallenge::token).unwrap_or(""),
            "challenge_response": self.challenge.as_ref().map(CertificateChallenge::response).unwrap_or(""),
            "last_error": self.backoff.as_ref().map(CertificateBackoff::last_error).unwrap_or(""),
            "next_attempt_at": self.backoff.as_ref().map(|backoff| encode_attempt(backoff.clock.next_attempt_at)).unwrap_or_default(),
            "failures": self.backoff.as_ref().map(|backoff| backoff.clock.failures).unwrap_or(0),
            "last_failure": encode_failure(self.backoff.as_ref().map(|backoff| backoff.clock.last_failure)),
        }))?)
    }
}

#[derive(Deserialize)]
struct CertificateBody {
    #[serde(default)]
    certificate: String,
    #[serde(default)]
    private_key: String,
    #[serde(default)]
    challenge_token: String,
    #[serde(default)]
    challenge_response: String,
    #[serde(default)]
    last_error: String,
    #[serde(default)]
    next_attempt_at: String,
    #[serde(default)]
    failures: u32,
    #[serde(default)]
    last_failure: String,
}

fn encode_attempt(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn decode_attempt(text: &str) -> Option<SystemTime> {
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| SystemTime::from(time.with_timezone(&Utc)))
}

fn decode_backoff(body: &CertificateBody) -> Option<CertificateBackoff> {
    let last_failure = decode_failure(&body.last_failure)?;
    let next_attempt_at = decode_attempt(&body.next_attempt_at)?;
    CertificateBackoff::new(
        body.last_error.clone(),
        IssuanceClock {
            failures: body.failures.max(1),
            next_attempt_at,
            last_failure,
        },
    )
}

fn encode_failure(failure: Option<IssuanceFailure>) -> &'static str {
    match failure {
        Some(IssuanceFailure::DoesNotResolve) => "does_not_resolve",
        Some(IssuanceFailure::ResolvesElsewhere) => "resolves_elsewhere",
        Some(IssuanceFailure::Authority) => "authority",
        None => "",
    }
}

fn decode_failure(text: &str) -> Option<IssuanceFailure> {
    match text {
        "does_not_resolve" => Some(IssuanceFailure::DoesNotResolve),
        "resolves_elsewhere" => Some(IssuanceFailure::ResolvesElsewhere),
        "authority" => Some(IssuanceFailure::Authority),
        _ => None,
    }
}

#[cfg(test)]
fn decode_certificate_body(encoded: &str) -> Result<Option<CertificateMaterial>, Error> {
    Ok(CertificateRow::decode(encoded)?.material)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use ployz_core::{IssuanceClock, IssuanceFailure};

    use super::{CertificateChallenge, CertificateMaterial, CertificateRow};

    #[test]
    fn invalid_certificate_body_is_an_error() {
        assert!(super::decode_certificate_body("{").is_err());
        assert!(super::decode_certificate_body("null").is_err());
    }

    #[test]
    fn empty_certificate_body_is_not_present() {
        assert_eq!(CertificateMaterial::new("", ""), None);
        assert_eq!(CertificateMaterial::new("CERT", ""), None);
        assert_eq!(super::decode_certificate_body("").unwrap(), None);
        assert_eq!(super::decode_certificate_body("{}").unwrap(), None);
        assert_eq!(
            super::decode_certificate_body(r#"{"certificate":"CERT","private_key":""}"#).unwrap(),
            None
        );
    }

    #[test]
    fn certificate_material_reads_known_fields_and_ignores_the_rest() {
        let material = super::decode_certificate_body(
            r#"{"certificate":"CERT","private_key":"KEY","last_error":"later"}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(material.certificate(), "CERT");
        assert_eq!(material.private_key(), "KEY");
    }

    #[test]
    fn certificate_challenge_reads_from_the_row_body() {
        let row = CertificateRow::decode(
            r#"{"certificate":"","private_key":"","challenge_token":"tok","challenge_response":"tok.thumb"}"#,
        )
        .unwrap();
        assert_eq!(row.material, None);
        let challenge = row.challenge.unwrap();
        assert_eq!(challenge.token(), "tok");
        assert_eq!(challenge.response(), "tok.thumb");
        assert_eq!(CertificateChallenge::new("", "tok.thumb"), None);
        assert_eq!(CertificateRow::decode("{}").unwrap().challenge, None);
    }

    #[test]
    fn certificate_row_round_trips_refusal_clock() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let clock = IssuanceClock {
            failures: 3,
            next_attempt_at: at,
            last_failure: IssuanceFailure::DoesNotResolve,
        };
        let row = CertificateRow::from_parts(None, None).with_backoff(
            "Ingress Hostname app.example.com does not resolve; it should resolve to 192.0.2.1.",
            clock,
        );
        let encoded = row.encode().unwrap();
        let decoded = CertificateRow::decode(&encoded).unwrap();
        assert_eq!(decoded.last_error(), row.last_error());
        assert_eq!(decoded.clock(), Some(clock));
        assert_eq!(
            CertificateRow::decode(
                r#"{"certificate":"","private_key":"","last_error":"later","next_attempt_at":"2023-11-14T22:13:20Z","failures":2,"last_failure":"resolves_elsewhere"}"#
            )
            .unwrap()
            .clock()
            .map(|clock| clock.last_failure),
            Some(IssuanceFailure::ResolvesElsewhere)
        );
        assert_eq!(
            CertificateRow::decode(r#"{"last_failure":"authority"}"#)
                .unwrap()
                .clock(),
            None
        );
    }
}
