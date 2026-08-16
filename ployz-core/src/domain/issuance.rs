//! Whether to contact the certificate authority for one Ingress Hostname.

use std::{
    net::IpAddr,
    time::{Duration, SystemTime},
};

use super::ClusterDnsVerdict;
use crate::IngressHost;

/// Delay after the first refusal or authority failure.
pub const ISSUANCE_BACKOFF_BASE: Duration = Duration::from_secs(60);

/// Longest delay between attempts. 4/day stays far below 5 failed validations/hour.
pub const ISSUANCE_BACKOFF_CAP: Duration = Duration::from_secs(6 * 60 * 60);

/// Which failure earned the shared backoff clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuanceFailure {
    DoesNotResolve,
    ResolvesElsewhere,
    Authority,
}

/// What the issuance loop should do for one wanted hostname.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuanceAction {
    Nothing,
    Refuse {
        failures: u32,
        next_attempt_at: SystemTime,
        last_failure: IssuanceFailure,
    },
    Order,
}

/// Inputs to [`issuance_action`]. Rank delay and renewal are later tickets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IssuanceInput {
    pub has_material: bool,
    pub next_attempt_at: Option<SystemTime>,
    pub last_failure: Option<IssuanceFailure>,
    pub failures: u32,
    pub verdict: ClusterDnsVerdict,
    pub now: SystemTime,
}

/// Decide whether to wait, refuse, or order. A resolve-verdict change drops resolve backoff.
#[must_use]
pub fn issuance_action(input: IssuanceInput) -> IssuanceAction {
    if input.has_material {
        return IssuanceAction::Nothing;
    }
    let waiting = input
        .next_attempt_at
        .is_some_and(|deadline| deadline > input.now);
    let last_resolve = match input.last_failure {
        Some(IssuanceFailure::DoesNotResolve) => Some(ClusterDnsVerdict::DoesNotResolve),
        Some(IssuanceFailure::ResolvesElsewhere) => Some(ClusterDnsVerdict::ResolvesElsewhere),
        Some(IssuanceFailure::Authority) | None => None,
    };
    let resolve_cleared = last_resolve.is_some_and(|last| last != input.verdict);
    if waiting && !resolve_cleared {
        return IssuanceAction::Nothing;
    }
    let last_failure = match input.verdict {
        ClusterDnsVerdict::PointsAtCluster => return IssuanceAction::Order,
        ClusterDnsVerdict::DoesNotResolve => IssuanceFailure::DoesNotResolve,
        ClusterDnsVerdict::ResolvesElsewhere => IssuanceFailure::ResolvesElsewhere,
    };
    let (failures, next_attempt_at) =
        issuance_failure_clock(input.failures, input.last_failure, last_failure, input.now);
    IssuanceAction::Refuse {
        failures,
        next_attempt_at,
        last_failure,
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

/// Next shared clock after a refusal or an authority failure.
#[must_use]
pub fn issuance_failure_clock(
    failures: u32,
    last_failure: Option<IssuanceFailure>,
    new_failure: IssuanceFailure,
    now: SystemTime,
) -> (u32, SystemTime) {
    let failures = if last_failure == Some(new_failure) {
        failures.saturating_add(1)
    } else {
        1
    };
    (failures, now + issuance_backoff(failures))
}

/// Why a hostname that misses this Cluster has no certificate.
#[must_use]
pub fn issuance_refusal_reason(
    hostname: &IngressHost,
    last_failure: IssuanceFailure,
    resolved: &[IpAddr],
    cluster_addresses: &[IpAddr],
) -> String {
    match last_failure {
        IssuanceFailure::DoesNotResolve => format!(
            "Ingress Hostname {hostname} does not resolve; it should resolve to {}.",
            join_addresses(cluster_addresses)
        ),
        IssuanceFailure::ResolvesElsewhere => format!(
            "Ingress Hostname {hostname} resolves to {}; it should resolve to {}.",
            join_addresses(resolved),
            join_addresses(cluster_addresses)
        ),
        IssuanceFailure::Authority => {
            format!("certificate authority failed for Ingress Hostname {hostname}")
        }
    }
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
        ISSUANCE_BACKOFF_BASE, ISSUANCE_BACKOFF_CAP, IssuanceAction, IssuanceFailure,
        IssuanceInput, issuance_action, issuance_backoff, issuance_failure_clock,
        issuance_refusal_reason,
    };
    use crate::{ClusterDnsVerdict, IngressHost};

    #[test]
    fn empty_row_orders_when_dns_points_at_the_cluster() {
        assert_eq!(
            issuance_action(input(false, None, None, ClusterDnsVerdict::PointsAtCluster)),
            IssuanceAction::Order
        );
    }

    #[test]
    fn empty_row_refuses_when_dns_misses_the_cluster() {
        assert_eq!(
            issuance_action(input(false, None, None, ClusterDnsVerdict::DoesNotResolve)),
            refuse(IssuanceFailure::DoesNotResolve, 1)
        );
        assert_eq!(
            issuance_action(input(
                false,
                None,
                None,
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            refuse(IssuanceFailure::ResolvesElsewhere, 1)
        );
    }

    #[test]
    fn material_is_left_alone() {
        assert_eq!(
            issuance_action(input(true, None, None, ClusterDnsVerdict::PointsAtCluster)),
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
                Some(IssuanceFailure::Authority),
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Nothing
        );
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(3600)),
                Some(IssuanceFailure::Authority),
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
                Some(IssuanceFailure::DoesNotResolve),
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
                Some(IssuanceFailure::DoesNotResolve),
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() + Duration::from_secs(6 * 60 * 60)),
                Some(IssuanceFailure::ResolvesElsewhere),
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
                Some(IssuanceFailure::DoesNotResolve),
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            refuse(IssuanceFailure::ResolvesElsewhere, 1)
        );
    }

    #[test]
    fn expired_clock_retries() {
        assert_eq!(
            issuance_action(input(
                false,
                Some(now() - Duration::from_secs(1)),
                Some(IssuanceFailure::Authority),
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(IssuanceInput {
                failures: 4,
                ..input(
                    false,
                    Some(now() - Duration::from_secs(1)),
                    Some(IssuanceFailure::DoesNotResolve),
                    ClusterDnsVerdict::DoesNotResolve,
                )
            }),
            refuse(IssuanceFailure::DoesNotResolve, 5)
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
    fn failure_clock_resets_resolve_and_keeps_authority() {
        let resolve = IssuanceFailure::DoesNotResolve;
        let elsewhere = IssuanceFailure::ResolvesElsewhere;
        assert_eq!(
            issuance_failure_clock(0, None, resolve, now()),
            (1, now() + issuance_backoff(1))
        );
        assert_eq!(
            issuance_failure_clock(4, Some(resolve), resolve, now()),
            (5, now() + issuance_backoff(5))
        );
        assert_eq!(
            issuance_failure_clock(4, Some(resolve), elsewhere, now()),
            (1, now() + issuance_backoff(1))
        );
        assert_eq!(
            issuance_failure_clock(6, Some(resolve), IssuanceFailure::Authority, now()),
            (1, now() + issuance_backoff(1))
        );
        assert_eq!(
            issuance_failure_clock(
                3,
                Some(IssuanceFailure::Authority),
                IssuanceFailure::Authority,
                now()
            ),
            (4, now() + issuance_backoff(4))
        );
    }

    #[test]
    fn refusal_reason_names_the_hostname_and_addresses() {
        let hostname = IngressHost::parse("app.example.com").unwrap();
        let cluster = addrs(["192.0.2.1", "192.0.2.2"]);
        let elsewhere = addrs(["198.51.100.10"]);

        assert_eq!(
            issuance_refusal_reason(&hostname, IssuanceFailure::DoesNotResolve, &[], &cluster),
            "Ingress Hostname app.example.com does not resolve; it should resolve to 192.0.2.1, 192.0.2.2."
        );
        assert_eq!(
            issuance_refusal_reason(
                &hostname,
                IssuanceFailure::ResolvesElsewhere,
                &elsewhere,
                &cluster
            ),
            "Ingress Hostname app.example.com resolves to 198.51.100.10; it should resolve to 192.0.2.1, 192.0.2.2."
        );
        assert_eq!(
            issuance_refusal_reason(
                &hostname,
                IssuanceFailure::ResolvesElsewhere,
                &elsewhere,
                &[]
            ),
            "Ingress Hostname app.example.com resolves to 198.51.100.10; it should resolve to this Cluster's Machine addresses (none are published)."
        );
    }

    fn input(
        has_material: bool,
        next_attempt_at: Option<SystemTime>,
        last_failure: Option<IssuanceFailure>,
        verdict: ClusterDnsVerdict,
    ) -> IssuanceInput {
        IssuanceInput {
            has_material,
            next_attempt_at,
            last_failure,
            failures: 0,
            verdict,
            now: now(),
        }
    }

    fn refuse(last_failure: IssuanceFailure, failures: u32) -> IssuanceAction {
        IssuanceAction::Refuse {
            failures,
            next_attempt_at: now() + issuance_backoff(failures),
            last_failure,
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
