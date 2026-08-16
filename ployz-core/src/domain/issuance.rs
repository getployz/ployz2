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

/// Shared backoff clock after a refusal or an authority failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IssuanceClock {
    pub failures: u32,
    pub next_attempt_at: SystemTime,
    pub last_failure: IssuanceFailure,
}

/// Whether this hostname already has material, or still needs a certificate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuanceSubject {
    HasMaterial,
    Missing { clock: Option<IssuanceClock> },
}

/// What the issuance loop should do for one wanted hostname.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuanceAction {
    Nothing,
    Refuse(IssuanceClock),
    Order,
}

/// Inputs to [`issuance_action`]. Rank delay and renewal are later tickets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IssuanceInput {
    pub subject: IssuanceSubject,
    pub verdict: ClusterDnsVerdict,
    pub now: SystemTime,
}

/// Decide whether to wait, refuse, or order. A resolve-verdict change drops resolve backoff.
#[must_use]
pub fn issuance_action(input: IssuanceInput) -> IssuanceAction {
    let IssuanceSubject::Missing { clock } = input.subject else {
        return IssuanceAction::Nothing;
    };
    let waiting = clock.is_some_and(|clock| clock.next_attempt_at > input.now);
    let last_resolve = clock.and_then(|clock| match clock.last_failure {
        IssuanceFailure::DoesNotResolve => Some(ClusterDnsVerdict::DoesNotResolve),
        IssuanceFailure::ResolvesElsewhere => Some(ClusterDnsVerdict::ResolvesElsewhere),
        IssuanceFailure::Authority => None,
    });
    let resolve_cleared = last_resolve.is_some_and(|last| last != input.verdict);
    if waiting && !resolve_cleared {
        return IssuanceAction::Nothing;
    }
    let last_failure = match input.verdict {
        ClusterDnsVerdict::PointsAtCluster => return IssuanceAction::Order,
        ClusterDnsVerdict::DoesNotResolve => IssuanceFailure::DoesNotResolve,
        ClusterDnsVerdict::ResolvesElsewhere => IssuanceFailure::ResolvesElsewhere,
    };
    IssuanceAction::Refuse(issuance_failure_clock(clock, last_failure, input.now))
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
    clock: Option<IssuanceClock>,
    new_failure: IssuanceFailure,
    now: SystemTime,
) -> IssuanceClock {
    let failures = if clock.is_some_and(|clock| clock.last_failure == new_failure) {
        clock
            .map(|clock| clock.failures)
            .unwrap_or(0)
            .saturating_add(1)
    } else {
        1
    };
    IssuanceClock {
        failures,
        next_attempt_at: now + issuance_backoff(failures),
        last_failure: new_failure,
    }
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
        ISSUANCE_BACKOFF_BASE, ISSUANCE_BACKOFF_CAP, IssuanceAction, IssuanceClock,
        IssuanceFailure, IssuanceInput, IssuanceSubject, issuance_action, issuance_backoff,
        issuance_failure_clock, issuance_refusal_reason,
    };
    use crate::{ClusterDnsVerdict, IngressHost};

    #[test]
    fn empty_row_orders_when_dns_points_at_the_cluster() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing { clock: None },
                ClusterDnsVerdict::PointsAtCluster
            )),
            IssuanceAction::Order
        );
    }

    #[test]
    fn empty_row_refuses_when_dns_misses_the_cluster() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing { clock: None },
                ClusterDnsVerdict::DoesNotResolve,
            )),
            refuse(IssuanceFailure::DoesNotResolve, 1)
        );
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing { clock: None },
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            refuse(IssuanceFailure::ResolvesElsewhere, 1)
        );
    }

    #[test]
    fn material_is_left_alone() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::HasMaterial,
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Nothing
        );
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::HasMaterial,
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn authority_backoff_is_served_out() {
        let clock = Some(clock(
            IssuanceFailure::Authority,
            1,
            now() + Duration::from_secs(3600),
        ));
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing { clock },
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Nothing
        );
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing { clock },
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn unchanged_resolve_backoff_is_served_out() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(
                        IssuanceFailure::DoesNotResolve,
                        1,
                        now() + Duration::from_secs(3600),
                    )),
                },
                ClusterDnsVerdict::DoesNotResolve,
            )),
            IssuanceAction::Nothing
        );
    }

    #[test]
    fn resolve_verdict_change_orders_without_waiting() {
        let later = now() + Duration::from_secs(6 * 60 * 60);
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(IssuanceFailure::DoesNotResolve, 1, later)),
                },
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(IssuanceFailure::ResolvesElsewhere, 1, later)),
                },
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
    }

    #[test]
    fn resolve_verdict_change_refuses_without_waiting() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(
                        IssuanceFailure::DoesNotResolve,
                        1,
                        now() + Duration::from_secs(6 * 60 * 60),
                    )),
                },
                ClusterDnsVerdict::ResolvesElsewhere,
            )),
            refuse(IssuanceFailure::ResolvesElsewhere, 1)
        );
    }

    #[test]
    fn expired_clock_retries() {
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(
                        IssuanceFailure::Authority,
                        1,
                        now() - Duration::from_secs(1),
                    )),
                },
                ClusterDnsVerdict::PointsAtCluster,
            )),
            IssuanceAction::Order
        );
        assert_eq!(
            issuance_action(input(
                IssuanceSubject::Missing {
                    clock: Some(clock(
                        IssuanceFailure::DoesNotResolve,
                        4,
                        now() - Duration::from_secs(1),
                    )),
                },
                ClusterDnsVerdict::DoesNotResolve,
            )),
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
        let resolve = clock(IssuanceFailure::DoesNotResolve, 4, now());
        let elsewhere = IssuanceFailure::ResolvesElsewhere;
        assert_eq!(
            issuance_failure_clock(None, IssuanceFailure::DoesNotResolve, now()),
            clock(
                IssuanceFailure::DoesNotResolve,
                1,
                now() + issuance_backoff(1)
            )
        );
        assert_eq!(
            issuance_failure_clock(Some(resolve), IssuanceFailure::DoesNotResolve, now()),
            clock(
                IssuanceFailure::DoesNotResolve,
                5,
                now() + issuance_backoff(5)
            )
        );
        assert_eq!(
            issuance_failure_clock(Some(resolve), elsewhere, now()),
            clock(elsewhere, 1, now() + issuance_backoff(1))
        );
        assert_eq!(
            issuance_failure_clock(Some(resolve), IssuanceFailure::Authority, now()),
            clock(IssuanceFailure::Authority, 1, now() + issuance_backoff(1))
        );
        assert_eq!(
            issuance_failure_clock(
                Some(clock(IssuanceFailure::Authority, 3, now())),
                IssuanceFailure::Authority,
                now()
            ),
            clock(IssuanceFailure::Authority, 4, now() + issuance_backoff(4))
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

    fn input(subject: IssuanceSubject, verdict: ClusterDnsVerdict) -> IssuanceInput {
        IssuanceInput {
            subject,
            verdict,
            now: now(),
        }
    }

    fn refuse(last_failure: IssuanceFailure, failures: u32) -> IssuanceAction {
        IssuanceAction::Refuse(clock(
            last_failure,
            failures,
            now() + issuance_backoff(failures),
        ))
    }

    fn clock(
        last_failure: IssuanceFailure,
        failures: u32,
        next_attempt_at: SystemTime,
    ) -> IssuanceClock {
        IssuanceClock {
            failures,
            next_attempt_at,
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
