//! Cluster DNS projection and verification.

use std::{collections::BTreeSet, net::IpAddr};

use crate::{DnsRecord, DnsRecordType, IngressHost, Machine, MachineObservation};

/// Machines eligible to contribute a public DNS address.
pub fn public_dns_candidates(
    observations: impl IntoIterator<Item = MachineObservation>,
) -> impl Iterator<Item = Machine> {
    observations.into_iter().filter_map(|observation| {
        (observation.membership.invites_rpc() && observation.machine.public_ip.is_some())
            .then_some(observation.machine)
    })
}

/// Build wildcard records from eligible, reachable Machines.
#[must_use]
pub fn public_dns_records(machines: impl IntoIterator<Item = Machine>) -> Vec<DnsRecord> {
    let mut ipv4 = BTreeSet::new();
    let mut ipv6 = BTreeSet::new();
    for machine in machines {
        match machine.public_ip {
            Some(IpAddr::V4(address)) => {
                ipv4.insert(address.to_string());
            }
            Some(IpAddr::V6(address)) => {
                ipv6.insert(address.to_string());
            }
            None => {}
        }
    }
    let mut records = Vec::new();
    if !ipv4.is_empty() {
        records.push(DnsRecord {
            name: "*".into(),
            record_type: DnsRecordType::A,
            values: ipv4.into_iter().collect(),
        });
    }
    if !ipv6.is_empty() {
        records.push(DnsRecord {
            name: "*".into(),
            record_type: DnsRecordType::Aaaa,
            values: ipv6.into_iter().collect(),
        });
    }
    records
}

/// Whether resolved addresses intersect this Cluster's Machine public addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterDnsVerdict {
    PointsAtCluster,
    ResolvesElsewhere,
    DoesNotResolve,
}

/// Intersect resolved addresses with Machine public addresses.
///
/// An empty `resolved` set is "does not resolve", including lookup failure.
#[must_use]
pub fn cluster_dns_verdict(resolved: &[IpAddr], cluster_addresses: &[IpAddr]) -> ClusterDnsVerdict {
    if resolved.is_empty() {
        return ClusterDnsVerdict::DoesNotResolve;
    }
    if resolved
        .iter()
        .any(|address| cluster_addresses.contains(address))
    {
        ClusterDnsVerdict::PointsAtCluster
    } else {
        ClusterDnsVerdict::ResolvesElsewhere
    }
}

impl IngressHost {
    /// Whether this hostname is the Cluster Domain or a name under it.
    #[must_use]
    pub fn under_cluster_domain(&self, cluster_domain: Option<&str>) -> bool {
        let Some(domain) = cluster_domain.filter(|domain| !domain.is_empty()) else {
            return false;
        };
        let hostname = self.as_str();
        hostname == domain || hostname.ends_with(&format!(".{domain}"))
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{ClusterDnsVerdict, cluster_dns_verdict, public_dns_candidates};
    use crate::{IngressHost, Machine, MachineObservation, MembershipObservation};

    #[test]
    fn public_dns_candidates_follow_all_membership_transitions() {
        let mut observation = MachineObservation {
            machine: serde_json::from_value::<Machine>(serde_json::json!({
                "id": "4".repeat(32),
                "name": "machine-4",
                "subnet": "10.210.4.0/24",
                "management_address": "fdcc::4",
                "public_key": vec![4; 32],
                "public_ip": "192.0.2.4"
            }))
            .unwrap(),
            membership: MembershipObservation::Unknown,
            selected_endpoint: None,
            rtt: None,
        };
        for (membership, published) in [
            (MembershipObservation::Unknown, false),
            (MembershipObservation::Suspect, true),
            (MembershipObservation::Down, false),
            (MembershipObservation::Up, true),
            (MembershipObservation::Unknown, false),
        ] {
            observation.membership = membership;
            assert_eq!(
                public_dns_candidates([observation.clone()])
                    .next()
                    .is_some(),
                published
            );
        }
    }

    #[test]
    fn verdict_covers_subset_none_empty_and_mix() {
        let cluster = addrs(["192.0.2.1", "192.0.2.2"]);
        let outside = addrs(["198.51.100.10"]);

        assert_eq!(
            cluster_dns_verdict(&addrs(["192.0.2.2"]), &cluster),
            ClusterDnsVerdict::PointsAtCluster
        );
        assert_eq!(
            cluster_dns_verdict(&outside, &cluster),
            ClusterDnsVerdict::ResolvesElsewhere
        );
        assert_eq!(
            cluster_dns_verdict(&[], &cluster),
            ClusterDnsVerdict::DoesNotResolve
        );
        assert_eq!(
            cluster_dns_verdict(&addrs(["198.51.100.10", "192.0.2.1"]), &cluster),
            ClusterDnsVerdict::PointsAtCluster
        );
    }

    #[test]
    fn under_cluster_domain_requires_a_label_boundary() {
        let domain = Some("opaque.uncloud.example");
        assert!(host("web.opaque.uncloud.example").under_cluster_domain(domain));
        assert!(host("opaque.uncloud.example").under_cluster_domain(domain));
        assert!(!host("app.example.com").under_cluster_domain(domain));
        assert!(!host("web.opaque.uncloud.example").under_cluster_domain(None));
        assert!(!host("web.opaque.uncloud.example").under_cluster_domain(Some("")));
        assert!(
            !host("evilopaque.uncloud.example").under_cluster_domain(domain),
            "a suffix without a label boundary is not under the Cluster Domain"
        );
    }

    fn host(hostname: &str) -> IngressHost {
        IngressHost::parse(hostname).unwrap()
    }

    fn addrs<const N: usize>(values: [&str; N]) -> Vec<IpAddr> {
        values
            .into_iter()
            .map(|value| value.parse().unwrap())
            .collect()
    }
}
