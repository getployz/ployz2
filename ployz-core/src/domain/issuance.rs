//! Whether to contact the certificate authority for one Ingress Hostname.

use std::{
    net::IpAddr,
    time::{Duration, SystemTime},
};

use super::{ClusterDnsVerdict, cluster_dns_verdict};
use crate::IngressHost;

/// Delay after the first refusal or authority failure.
pub const ISSUANCE_BACKOFF_BASE: Duration = Duration::from_secs(60);

/// Longest delay between attempts. 4/day stays far below 5 failed validations/hour.
pub const ISSUANCE_BACKOFF_CAP: Duration = Duration::from_secs(6 * 60 * 60);

/// What the issuance loop should do for one wanted hostname.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuanceAction {
    Nothing,
    Refuse,
    Order,
}

/// Inputs to [`issuance_action`]. Rank delay and renewal are later tickets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IssuanceInput {
    pub has_material: bool,
    pub next_attempt_at: Option<SystemTime>,
    pub last_resolve_verdict: Option<ClusterDnsVerdict>,
    pub verdict: ClusterDnsVerdict,
    pub now: SystemTime,
}

/// Decide whether to wait, refuse, or order. A resolve-verdict change drops resolve backoff.
#[must_use]
pub fn issuance_action(input: IssuanceInput) -> IssuanceAction {
    if input.has_material {
        return IssuanceAction::Nothing;
    }
    if waiting(input) && !resolve_backoff_cleared(input) {
        return IssuanceAction::Nothing;
    }
    match input.verdict {
        ClusterDnsVerdict::PointsAtCluster => IssuanceAction::Order,
        ClusterDnsVerdict::DoesNotResolve | ClusterDnsVerdict::ResolvesElsewhere => {
            IssuanceAction::Refuse
        }
    }
}

/// Delay after `failures` recorded attempts. `failures == 0` uses the base delay.
#[must_use]
pub fn issuance_backoff(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(31);
    let seconds = ISSUANCE_BACKOFF_BASE
        .as_secs()
        .saturating_mul(1_u64 << shift);
    Duration::from_secs(seconds.min(ISSUANCE_BACKOFF_CAP.as_secs()))
}

/// Why a hostname that misses this Cluster has no certificate.
#[must_use]
pub fn issuance_refusal_reason(
    hostname: &IngressHost,
    resolved: &[IpAddr],
    cluster_addresses: &[IpAddr],
) -> Option<String> {
    let should = join_addresses(cluster_addresses);
    match cluster_dns_verdict(resolved, cluster_addresses) {
        ClusterDnsVerdict::PointsAtCluster => None,
        ClusterDnsVerdict::DoesNotResolve => Some(format!(
            "Ingress Hostname {hostname} does not resolve; it should resolve to {should}."
        )),
        ClusterDnsVerdict::ResolvesElsewhere => Some(format!(
            "Ingress Hostname {hostname} resolves to {}; it should resolve to {should}.",
            join_addresses(resolved)
        )),
    }
}

fn waiting(input: IssuanceInput) -> bool {
    input
        .next_attempt_at
        .is_some_and(|deadline| deadline > input.now)
}

fn resolve_backoff_cleared(input: IssuanceInput) -> bool {
    input
        .last_resolve_verdict
        .is_some_and(|last| last != input.verdict)
}

fn join_addresses(addresses: &[IpAddr]) -> String {
    if addresses.is_empty() {
        return "this Cluster's Machine addresses (none are published)".into();
    }
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::{
        net::IpAddr,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        ISSUANCE_BACKOFF_BASE, ISSUANCE_BACKOFF_CAP, IssuanceAction, IssuanceInput,
        issuance_action, issuance_backoff, issuance_refusal_reason,
    };
    use crate::{ClusterDnsVerdict, IngressHost};

    #[test]
    fn empty_row_orders_when_dns_points_at_the_cluster() {
        assert_eq!(
            issuance_action(input(false, None, None, ClusterDnsVerdict::PointsAtCluster,)),
            IssuanceAction::Order
        );
    }

    #[test]
    fn empty_row_refuses_when_dns_misses_the_cluster() {
        assert_eq!(
            issuance_action(input(false, None, None, ClusterDnsVerdict::DoesNotResolve)),
            IssuanceAction::Refuse
        );
        assert_eq!(
            issuance_action(input(
                false,
                None,
                None,
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            IssuanceAction::Refuse
        );
    }

    #[test]
    fn material_is_left_alone() {
        assert_eq!(
            issuance_action(input(true, None, None, ClusterDnsVerdict::PointsAtCluster,)),
            IssuanceAction::Nothing
        );
        assert_eq!(
            issuance_action(input(true, None, None, ClusterDnsVerdict::DoesNotResolve)),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn authority_backoff_is_served_out() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(3600)),
                None,
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Nothing
        );
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(3600)),
                None,
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn unchanged_resolve_backoff_is_served_out() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(3600)),
                Some(ClusterDnsVerdict::DoesNotResolve),
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn resolve_verdict_change_orders_without_waiting() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(6 * 60 * 60)),
                Some(ClusterDnsVerdict::DoesNotResolve),
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(6 * 60 * 60)),
                Some(ClusterDnsVerdict::ResolvesElsewhere),
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
    }

    #[test]
    fn resolve_verdict_change_refuses_without_waiting() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(6 * 60 * 60)),
                Some(ClusterDnsVerdict::DoesNotResolve),
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            IssuanceAction::Refuse
        );
    }

    #[test]
    fn expired_clock_retries() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() - Duration::from_secs(1)),
                None,
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() - Duration::from_secs(1)),
                Some(ClusterDnsVerdict::DoesNotResolve),
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Refuse
        );
    }

    #[test]
    fn backoff_doubles_until_the_cap_and_never_stops() {
        assert_eq!(issuance_backoff(0), ISSUANCE_BACKOFF_BASE);
        assert_eq!(issuance_backoff(1), Duration::from_secs(60));
        assert_eq!(issuance_backoff(2), Duration::from_secs(120));
        assert_eq!(issuance_backoff(3), Duration::from_secs(240));
        assert_eq!(issuance_backoff(7), Duration::from_secs(3840));
        assert_eq!(issuance_backoff(9), Duration::from_secs(15360));
        assert_eq!(issuance_backoff(10), ISSUANCE_BACKOFF_CAP);
        assert_eq!(issuance_backoff(11), ISSUANCE_BACKOFF_CAP);
        assert_eq!(issuance_backoff(u32::MAX), ISSUANCE_BACKOFF_CAP);
        assert_eq!(ISSUANCE_BACKOFF_CAP, Duration::from_secs(21600));
    }

    #[test]
    fn refusal_reason_names_the_hostname_and_addresses() {
        let hostname = IngressHost::parse("app.example.com").unwrap();
        let cluster = addrs(["192.0.2.1", "192.0.2.2"]);
        let elsewhere = addrs(["198.51.100.10"]);

        assert_eq!(
            issuance_refusal_reason(&hostname, &[], &cluster).as_deref(),
            Some(
                "Ingress Hostname app.example.com does not resolve; it should resolve to 192.0.2.1, 192.0.2.2."
            )
        );
        assert_eq!(
            issuance_refusal_reason(&hostname, &elsewhere, &cluster).as_deref(),
            Some(
                "Ingress Hostname app.example.com resolves to 198.51.100.10; it should resolve to 192.0.2.1, 192.0.2.2."
            )
        );
        assert_eq!(
            issuance_refusal_reason(&hostname, &addrs(["192.0.2.1"]), &cluster),
            None
        );
        assert_eq!(
            issuance_refusal_reason(&hostname, &elsewhere, &[]).as_deref(),
            Some(
                "Ingress Hostname app.example.com resolves to 198.51.100.10; it should resolve to this Cluster's Machine addresses (none are published)."
            )
        );
    }

    fn input(
        has_material: bool,
        next_attempt_at: Option<SystemTime>,
        last_resolve_verdict: Option<ClusterDnsVerdict>,
        verdict: ClusterDnsVerdict,
    ) -> IssuanceInput {
        IssuanceInput {
            has_material,
            next_attempt_at,
            last_resolve_verdict,
            verdict,
            now: now(),
        }
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    fn addrs<const N: usize>(values: [&str; N]) -> Vec<IpAddr> {
        values
            .into_iter()
            .map(|value| value.parse().unwrap())
            .collect()
    }
}
