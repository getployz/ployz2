//! Where a Build runs. The requested location comes from the CLI and Compose;
//! the automatic choice is a pure function of the Machines this client observed.
//! The selected Machine still admits or refuses the Build itself.

use std::collections::BTreeSet;

use ployz_core::{MachineId, MachineName, MachineTarget};
use thiserror::Error;

/// `--remote` given without a value. Shared with the clap definition in
/// `cli`, whose `default_missing_value` must stay this exact string for plain
/// `--remote` to mean automatic selection rather than a pin.
pub(crate) const AUTOMATIC: &str = "";

use crate::image::platform_compatible;

/// A requested build location that names no Machine. The origin is part of the
/// message: the same text can come from a flag or from Compose.
#[derive(Debug, Error)]
#[error("{origin} {value:?} is not a Machine name or ID, `auto`, or `local`")]
pub(crate) struct InvalidLocation {
    origin: &'static str,
    value: String,
}

/// Execution location a user asked for, before any Machine is observed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Location {
    /// Build in the invoking client's own Docker.
    Local,
    /// Build on a Machine, once one is resolved.
    Remote(Selection),
}

/// How the Build Machine is chosen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Selection {
    /// Choose one compatible Machine from what this client can observe.
    Automatic,
    /// Build on exactly this Machine.
    Pinned(MachineTarget),
}

impl Location {
    /// Resolve the requested location. An explicit flag always beats Compose,
    /// and Compose beats the local default.
    ///
    /// `remote` is the `--remote` value: absent when the flag was not given,
    /// empty for plain `--remote`, which always asks for a fresh automatic
    /// choice rather than the Compose pin. `preferred` is the Compose
    /// `x-build-machine` value. `auto` and `local` name modes wherever they
    /// appear, so a Machine called either is pinned by its ID instead.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLocation`] when a pinned target is not a Machine
    /// identity, naming the flag or Compose key it was written in.
    pub(crate) fn requested(
        remote: Option<&str>,
        local: bool,
        preferred: Option<&str>,
    ) -> Result<Self, InvalidLocation> {
        if local {
            return Ok(Self::Local);
        }
        let (origin, value) = match (remote, preferred) {
            (Some(value), _) => ("--remote", value),
            (None, Some(value)) => ("Compose x-build-machine", value),
            (None, None) => return Ok(Self::Local),
        };
        match value {
            "local" => Ok(Self::Local),
            AUTOMATIC | "auto" => Ok(Self::Remote(Selection::Automatic)),
            target => MachineTarget::parse(target)
                .map(|target| Self::Remote(Selection::Pinned(target)))
                .map_err(|_| InvalidLocation {
                    origin,
                    value: target.to_owned(),
                }),
        }
    }
}

/// What one visible Machine offers a Build, as observed by this client.
#[derive(Debug)]
pub(crate) struct Candidate {
    pub id: MachineId,
    pub name: MachineName,
    pub evidence: Evidence,
}

/// Build capability of one Machine. Unanswered evidence stays visible: it is
/// never read as either support or refusal.
#[derive(Debug)]
pub(crate) enum Evidence {
    /// The Machine advertises remote Builds. `architecture` is the kernel
    /// architecture from the Cluster's discovery record, not part of the
    /// Machine's answer and not Docker's own platform, so it can be stale or
    /// absent.
    Builds { architecture: String },
    /// The Machine answered and does not advertise remote Builds.
    Refuses,
    /// The Machine's membership structurally forbids an RPC, so it was never
    /// asked. That is a known ineligibility, not a failed probe.
    Ineligible(String),
    /// The Machine could not be asked, with the reason it could not.
    Unanswered(String),
}

/// The Machine an automatic Build selected, and what it cannot prove about it.
#[derive(Debug)]
pub(crate) struct Choice<'list> {
    pub machine: &'list Candidate,
    /// Required platforms this client could not confirm the Machine runs
    /// natively. The Machine confirms or refuses them at admission.
    pub unconfirmed: Vec<String>,
    /// Visible Machines that never answered, so a better candidate may have
    /// been passed over. One line of evidence each.
    pub unanswered: Vec<String>,
}

/// Why no Machine could be chosen. The observed evidence belongs here; the
/// command that could fix it belongs to the CLI.
#[derive(Debug, Error)]
pub(crate) enum NoBuildMachine {
    #[error("no Machine is visible, so no Build Machine can be selected")]
    Invisible,
    #[error("no visible Machine answered, so none could be selected. Observed {}", .unanswered.join("; "))]
    Silent { unanswered: Vec<String> },
    #[error("no visible Machine can run this Build. Observed {}", .observed.join("; "))]
    Incapable { observed: Vec<String> },
}

/// Pick one Machine for an automatic Build.
///
/// Machines that never invite an RPC, do not answer, or answer without the
/// Build capability are not candidates. The Machine that natively runs the most required platforms wins,
/// and Machine ID breaks every remaining tie, so the choice does not depend on
/// the order the Machines were observed.
///
/// # Errors
///
/// Returns [`NoBuildMachine`] when no observed Machine can build, carrying every
/// Machine and the evidence that excluded it.
pub(crate) fn choose<'list>(
    candidates: &'list [Candidate],
    required: &BTreeSet<String>,
) -> Result<Choice<'list>, NoBuildMachine> {
    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    let mut unanswered = Vec::new();
    for candidate in candidates {
        let name = &candidate.name;
        let id = &candidate.id;
        match &candidate.evidence {
            Evidence::Builds { architecture } => {
                eligible.push((candidate, unconfirmed(architecture, required)));
            }
            Evidence::Refuses => {
                excluded.push(format!("{name} ({id}) does not run remote Builds"));
            }
            Evidence::Ineligible(reason) => excluded.push(format!("{name} ({id}) {reason}")),
            Evidence::Unanswered(reason) => {
                unanswered.push(format!("{name} ({id}) capability unknown: {reason}"));
            }
        }
    }
    let chosen = eligible
        .into_iter()
        .min_by(|(left, left_open), (right, right_open)| {
            left_open
                .len()
                .cmp(&right_open.len())
                .then_with(|| left.id.as_str().cmp(right.id.as_str()))
        });
    let Some((machine, unconfirmed)) = chosen else {
        // Nothing answered is a reachability problem, not a capability one;
        // telling that user to upgrade a Machine would be wrong advice.
        return Err(match (excluded.is_empty(), unanswered.is_empty()) {
            (true, true) => NoBuildMachine::Invisible,
            (true, false) => NoBuildMachine::Silent { unanswered },
            _ => NoBuildMachine::Incapable {
                observed: excluded.into_iter().chain(unanswered).collect(),
            },
        });
    };
    Ok(Choice {
        machine,
        unconfirmed,
        unanswered,
    })
}

/// Required platforms this client cannot confirm the Machine runs natively. An
/// unreported or unrecognised architecture proves nothing, so every required
/// platform stays unconfirmed rather than being called emulated.
pub(crate) fn unconfirmed(architecture: &str, required: &BTreeSet<String>) -> Vec<String> {
    required
        .iter()
        .filter(|platform| !platform_compatible(platform, architecture))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(hex: char, name: &str, evidence: Evidence) -> Candidate {
        Candidate {
            id: MachineId::parse(hex.to_string().repeat(32)).unwrap(),
            name: MachineName::parse(name).unwrap(),
            evidence,
        }
    }

    fn builds(architecture: &str) -> Evidence {
        Evidence::Builds {
            architecture: architecture.into(),
        }
    }

    fn platforms<const N: usize>(values: [&str; N]) -> BTreeSet<String> {
        values.into_iter().map(ToOwned::to_owned).collect()
    }

    #[test]
    fn explicit_flags_override_compose_and_compose_overrides_the_local_default() {
        let requested = Location::requested;
        let pinned = |name: &str| Location::Remote(Selection::Pinned(name.parse().unwrap()));
        assert_eq!(requested(None, false, None).unwrap(), Location::Local);
        assert_eq!(
            requested(None, false, Some("local")).unwrap(),
            Location::Local
        );
        assert_eq!(
            requested(None, false, Some("auto")).unwrap(),
            Location::Remote(Selection::Automatic)
        );
        assert_eq!(
            requested(None, false, Some("tower")).unwrap(),
            pinned("tower")
        );
        // Plain --remote asks for a fresh choice, never the configured pin.
        assert_eq!(
            requested(Some(""), false, Some("tower")).unwrap(),
            Location::Remote(Selection::Automatic)
        );
        assert_eq!(
            requested(Some("edge"), false, Some("tower")).unwrap(),
            pinned("edge")
        );
        assert_eq!(
            requested(None, true, Some("tower")).unwrap(),
            Location::Local
        );
        assert_eq!(
            requested(None, true, Some("auto")).unwrap(),
            Location::Local
        );
        // The two mode words mean the same on the flag as in Compose.
        assert_eq!(
            requested(Some("auto"), false, Some("tower")).unwrap(),
            Location::Remote(Selection::Automatic)
        );
        assert_eq!(
            requested(Some("local"), false, Some("tower")).unwrap(),
            Location::Local
        );
        assert!(requested(Some("*"), false, None).is_err());
        assert!(requested(None, false, Some("*")).is_err());
    }

    #[test]
    fn automatic_selection_prefers_native_platforms_and_ignores_observation_order() {
        let observed = |order: [char; 3]| {
            order
                .into_iter()
                .map(|hex| match hex {
                    'a' => candidate('a', "amd", builds("x86_64")),
                    'b' => candidate('b', "arm", builds("aarch64")),
                    _ => candidate('c', "old", builds("")),
                })
                .collect::<Vec<_>>()
        };
        for order in [['a', 'b', 'c'], ['c', 'b', 'a'], ['b', 'a', 'c']] {
            let machines = observed(order);
            let native = choose(&machines, &platforms(["linux/arm64"])).unwrap();
            assert_eq!(native.machine.name.as_str(), "arm");
            assert!(native.unconfirmed.is_empty() && native.unanswered.is_empty());
            // No native candidate: the lowest Machine ID wins and the platforms
            // it cannot prove stay visible.
            let emulating = choose(&machines, &platforms(["linux/riscv64"])).unwrap();
            assert_eq!(emulating.machine.name.as_str(), "amd");
            assert_eq!(emulating.unconfirmed, ["linux/riscv64"]);
            // Nothing required: stable Machine-ID order alone decides.
            assert_eq!(
                choose(&machines, &BTreeSet::new())
                    .unwrap()
                    .machine
                    .name
                    .as_str(),
                "amd"
            );
        }
    }

    #[test]
    fn incapable_and_unanswered_machines_are_not_candidates_but_stay_in_the_refusal() {
        let machines = [
            candidate('a', "old", Evidence::Refuses),
            candidate('b', "gone", Evidence::Unanswered("did not answer".into())),
        ];
        let refusal = choose(&machines, &BTreeSet::new()).unwrap_err().to_string();
        assert!(refusal.contains("old"), "{refusal}");
        assert!(refusal.contains("does not run remote Builds"), "{refusal}");
        assert!(refusal.contains("gone"), "{refusal}");
        assert!(refusal.contains("did not answer"), "{refusal}");
        assert!(
            matches!(
                choose(&[], &BTreeSet::new()).unwrap_err(),
                NoBuildMachine::Invisible
            ),
            "an empty Cluster is not a capability refusal"
        );
        // Nothing answered at all is a reachability refusal, not a capability
        // one. A Machine that was never eligible to be asked is neither.
        let silent = [candidate(
            'b',
            "gone",
            Evidence::Unanswered("did not answer".into()),
        )];
        assert!(matches!(
            choose(&silent, &BTreeSet::new()).unwrap_err(),
            NoBuildMachine::Silent { .. }
        ));
        let draining = [candidate(
            'b',
            "gone",
            Evidence::Ineligible("is draining".into()),
        )];
        assert!(
            matches!(
                choose(&draining, &BTreeSet::new()).unwrap_err(),
                NoBuildMachine::Incapable { .. }
            ),
            "a Machine that cannot be asked is not a connection problem"
        );
        // One capable Machine among them is selected, and the Machine that
        // never answered stays visible on that success.
        let mut machines = Vec::from_iter(machines);
        machines.push(candidate('c', "tower", builds("x86_64")));
        let choice = choose(&machines, &platforms(["linux/amd64"])).unwrap();
        assert_eq!(choice.machine.name.as_str(), "tower");
        assert!(choice.unconfirmed.is_empty());
        assert_eq!(choice.unanswered.len(), 1, "{choice:?}");
        assert!(choice.unanswered.concat().contains("gone"), "{choice:?}");
    }

    #[test]
    fn a_multi_platform_request_prefers_the_machine_that_emulates_least() {
        let machines = [
            // Reports no architecture: it would emulate both.
            candidate('a', "old", builds("")),
            candidate('b', "arm", builds("aarch64")),
        ];
        let choice = choose(&machines, &platforms(["linux/amd64", "linux/arm64"])).unwrap();
        assert_eq!(choice.machine.name.as_str(), "arm");
        assert_eq!(choice.unconfirmed, ["linux/amd64"]);
        // A variant suffix names the same execution platform.
        assert!(
            choose(&machines, &platforms(["linux/arm64/v8"]))
                .unwrap()
                .unconfirmed
                .is_empty()
        );
    }
}
