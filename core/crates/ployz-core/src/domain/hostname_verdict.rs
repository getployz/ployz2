//! Whether an Ingress Hostname reaches this Cluster, judged by fetching `/.ployz-verify` through it.

use std::net::IpAddr;

use crate::{IngressHost, MachineId};

/// What a `GET http://<hostname>/.ployz-verify` returned, without following redirects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerifyAnswer {
    /// No HTTP response: refused, reset, or timed out.
    NoAnswer,
    /// A 3xx response. `location` is empty when the header is missing.
    Redirect { location: String },
    /// Any other HTTP response.
    Response { success: bool, body: String },
}

/// How a hostname that reaches this Cluster gets here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterRoute {
    /// A resolved address is a Machine public address.
    Direct,
    /// A Machine answered, but no resolved address is a Machine public address.
    ViaProxy,
}

/// What one Machine saw fetching `/.ployz-verify` through an Ingress Hostname.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostnameVerdict {
    DoesNotResolve,
    Unreachable,
    RedirectsToHttps,
    ReachesElsewhere,
    ReachesCluster(ClusterRoute),
}

/// Judge the verify answer from each resolved address against this Cluster's Machines.
///
/// Any address that reaches this Cluster is enough; otherwise the first address decides.
/// No addresses is "does not resolve", including lookup failure.
#[must_use]
pub fn hostname_verdict(
    answers: &[(IpAddr, VerifyAnswer)],
    machine_ids: &[MachineId],
    cluster_addresses: &[IpAddr],
) -> HostnameVerdict {
    let verdicts = answers.iter().map(|(address, answer)| match answer {
        VerifyAnswer::NoAnswer => HostnameVerdict::Unreachable,
        VerifyAnswer::Redirect { location } if location.starts_with("https://") => {
            HostnameVerdict::RedirectsToHttps
        }
        VerifyAnswer::Response {
            success: true,
            body,
        } if MachineId::parse(body.trim()).is_ok_and(|id| machine_ids.contains(&id)) => {
            HostnameVerdict::ReachesCluster(if cluster_addresses.contains(address) {
                ClusterRoute::Direct
            } else {
                ClusterRoute::ViaProxy
            })
        }
        VerifyAnswer::Redirect { .. } | VerifyAnswer::Response { .. } => {
            HostnameVerdict::ReachesElsewhere
        }
    });
    let mut first = None;
    for verdict in verdicts {
        if matches!(verdict, HostnameVerdict::ReachesCluster(_)) {
            return verdict;
        }
        first.get_or_insert(verdict);
    }
    first.unwrap_or(HostnameVerdict::DoesNotResolve)
}

/// Why a hostname that does not reach this Cluster has no certificate, with one fix.
/// `None` when it reaches this Cluster.
#[must_use]
pub fn hostname_verdict_reason(
    hostname: &IngressHost,
    verdict: HostnameVerdict,
    cluster_addresses: &[IpAddr],
) -> Option<String> {
    Some(match verdict {
        HostnameVerdict::ReachesCluster(_) => return None,
        HostnameVerdict::DoesNotResolve => format!(
            "{hostname} does not resolve. Add a DNS record pointing at {}.",
            join_addresses(cluster_addresses)
        ),
        HostnameVerdict::Unreachable => {
            format!("{hostname} did not answer on port 80. Open port 80 to this Cluster.")
        }
        // TODO: write a docs page on custom domains behind a proxy (Cloudflare
        // "Always Use HTTPS", exempting the ACME path, or uploading an origin
        // certificate) and link it from this message and the dashboard row.
        HostnameVerdict::RedirectsToHttps => format!(
            "{hostname} redirects HTTP to HTTPS before reaching this Cluster. \
             Exempt /.well-known/acme-challenge/* from HTTPS redirects in your proxy."
        ),
        HostnameVerdict::ReachesElsewhere => format!(
            "{hostname} answers from another server. Point it at {}.",
            join_addresses(cluster_addresses)
        ),
    })
}

fn join_addresses(addresses: &[IpAddr]) -> String {
    if addresses.is_empty() {
        return "this Cluster's Machine addresses (none are published)".into();
    }
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" or ")
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{
        ClusterRoute, HostnameVerdict, VerifyAnswer, hostname_verdict, hostname_verdict_reason,
    };
    use crate::{IngressHost, MachineId};

    const OURS: &str = "0123456789abcdef0123456789abcdef";
    const THEIRS: &str = "fedcba9876543210fedcba9876543210";
    const PROXY: &str = "198.51.100.10";

    #[test]
    fn verdict_covers_every_answer() {
        let ids = [MachineId::parse(OURS).unwrap()];
        let judge = |answer: VerifyAnswer| {
            hostname_verdict(&[(ip(PROXY), answer)], &ids, &addrs(["192.0.2.1"]))
        };

        assert_eq!(
            hostname_verdict(&[], &ids, &addrs(["192.0.2.1"])),
            HostnameVerdict::DoesNotResolve
        );
        assert_eq!(judge(VerifyAnswer::NoAnswer), HostnameVerdict::Unreachable);
        assert_eq!(
            judge(redirect("https://app.example.com/.ployz-verify")),
            HostnameVerdict::RedirectsToHttps
        );
        assert_eq!(
            judge(redirect("http://elsewhere.example.com/")),
            HostnameVerdict::ReachesElsewhere
        );
        assert_eq!(judge(answered(THEIRS)), HostnameVerdict::ReachesElsewhere);
        assert_eq!(
            judge(VerifyAnswer::Response {
                success: false,
                body: OURS.into()
            }),
            HostnameVerdict::ReachesElsewhere
        );
        assert_eq!(
            judge(answered(&format!("{OURS}\n"))),
            HostnameVerdict::ReachesCluster(ClusterRoute::ViaProxy)
        );
    }

    #[test]
    fn any_address_reaching_the_cluster_wins_otherwise_the_first_decides() {
        let ids = [MachineId::parse(OURS).unwrap()];
        let cluster = addrs(["192.0.2.1"]);

        assert_eq!(
            hostname_verdict(
                &[
                    (ip(PROXY), VerifyAnswer::NoAnswer),
                    (ip("192.0.2.1"), answered(OURS)),
                ],
                &ids,
                &cluster
            ),
            HostnameVerdict::ReachesCluster(ClusterRoute::Direct)
        );
        assert_eq!(
            hostname_verdict(
                &[
                    (ip(PROXY), answered(THEIRS)),
                    (ip("192.0.2.1"), VerifyAnswer::NoAnswer),
                ],
                &ids,
                &cluster
            ),
            HostnameVerdict::ReachesElsewhere
        );
    }

    #[test]
    fn reason_names_one_fix_per_verdict() {
        let hostname = IngressHost::parse("app.example.com").unwrap();
        let cluster = addrs(["192.0.2.1", "192.0.2.2"]);
        let reason = |verdict| hostname_verdict_reason(&hostname, verdict, &cluster);

        assert_eq!(
            reason(HostnameVerdict::DoesNotResolve).as_deref(),
            Some(
                "app.example.com does not resolve. Add a DNS record pointing at 192.0.2.1 or 192.0.2.2."
            )
        );
        assert_eq!(
            reason(HostnameVerdict::Unreachable).as_deref(),
            Some("app.example.com did not answer on port 80. Open port 80 to this Cluster.")
        );
        assert_eq!(
            reason(HostnameVerdict::RedirectsToHttps).as_deref(),
            Some(
                "app.example.com redirects HTTP to HTTPS before reaching this Cluster. \
                 Exempt /.well-known/acme-challenge/* from HTTPS redirects in your proxy."
            )
        );
        assert_eq!(
            reason(HostnameVerdict::ReachesElsewhere).as_deref(),
            Some(
                "app.example.com answers from another server. Point it at 192.0.2.1 or 192.0.2.2."
            )
        );
        assert_eq!(
            reason(HostnameVerdict::ReachesCluster(ClusterRoute::ViaProxy)),
            None
        );
        assert_eq!(
            hostname_verdict_reason(&hostname, HostnameVerdict::ReachesElsewhere, &[]).as_deref(),
            Some(
                "app.example.com answers from another server. Point it at this Cluster's Machine addresses (none are published)."
            )
        );
    }

    fn answered(body: &str) -> VerifyAnswer {
        VerifyAnswer::Response {
            success: true,
            body: body.into(),
        }
    }

    fn redirect(location: &str) -> VerifyAnswer {
        VerifyAnswer::Redirect {
            location: location.into(),
        }
    }

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    fn addrs<const N: usize>(values: [&str; N]) -> Vec<IpAddr> {
        values
            .into_iter()
            .map(|value| value.parse().unwrap())
            .collect()
    }
}
