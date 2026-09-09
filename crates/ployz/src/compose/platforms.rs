//! Which platforms a Deploy's Railpack Builds must produce, read from the
//! Machines each Service may be placed on.

use std::collections::BTreeMap;

use ployz_core::{
    Machine, MachineObservation, MembershipObservation, RequestedServiceSpec,
    ServicePlacementEligibility, ServicePlacementIneligibleReason,
};

use super::{CapturedBuild, ComposeError, invalid_build};
use crate::compose::CapturedCompose;

/// The only platforms the pinned Railpack frontend produces.
pub(super) const RAILPACK_PLATFORMS: [&str; 2] = ["linux/amd64", "linux/arm64"];

impl CapturedBuild {
    /// Fix each Railpack target's platforms to what the Machines its Service
    /// may be placed on run natively, from read-only observations. Authored
    /// `build.platforms` must already cover them; the execution host's default
    /// platform is replaced, since it describes the client and not the Cluster.
    /// Dockerfile targets keep one platform; Deploy's coverage check refuses a
    /// mismatch before any change.
    ///
    /// # Errors
    /// Names the Service whose explicit platforms miss a Machine's platform.
    pub fn cover_machines(
        &mut self,
        candidate: &CapturedCompose,
        machines: &[MachineObservation],
    ) -> Result<(), ComposeError> {
        for target in &mut self.targets {
            if !self
                .railpack
                .iter()
                .any(|recipe| recipe.name == target.name)
            {
                continue;
            }
            // A build-only dependency (profile-gated Service context) is never
            // placed, so nothing constrains its platform.
            let Some(spec) = candidate
                .intent()
                .target
                .iter()
                .find(|spec| spec.name.as_str() == target.name)
            else {
                continue;
            };
            let required = machine_platforms(&target.name, spec, machines)?;
            if required.is_empty() {
                // No placement is visible; the coverage check after the Build
                // decides, and nothing here can name a better platform.
                continue;
            }
            if !self.authored_platforms.contains(&target.name) {
                target.platforms = required.into_keys().collect();
            } else if let Some((platform, machines)) = required
                .iter()
                .find(|(platform, _)| !target.platforms.contains(platform))
            {
                return Err(invalid_build(&format!(
                    "service '{}' builds for {} but {} runs {platform}; add it to build.platforms",
                    target.name,
                    target.platforms.join(", "),
                    machines
                        .iter()
                        .map(|machine| named(machine))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(())
    }
}

/// Railpack platforms the Machines a Service may be placed on run natively,
/// each with the Machines that need it.
///
/// # Errors
/// Names a Machine the Service may land on that no Railpack platform runs:
/// no rerun can cover it, so the Build is refused before compilation.
fn machine_platforms<'observed>(
    service: &str,
    spec: &RequestedServiceSpec,
    machines: &'observed [MachineObservation],
) -> Result<BTreeMap<String, Vec<&'observed Machine>>, ComposeError> {
    let mut required = BTreeMap::<String, Vec<&Machine>>::new();
    for machine in machines
        .iter()
        .filter(|machine| machine.membership != MembershipObservation::Down)
        .filter(|machine| {
            !matches!(
                spec.placement_eligibility(&machine.machine, None),
                ServicePlacementEligibility::Ineligible(
                    ServicePlacementIneligibleReason::PlacementMismatch
                )
            )
        })
    {
        let architecture = &machine.machine.runtime.architecture;
        let Some(platform) = RAILPACK_PLATFORMS
            .iter()
            .find(|platform| crate::image::platform_compatible(platform, architecture))
        else {
            return Err(invalid_build(&format!(
                "service '{service}' may run on {}, which reports architecture {architecture:?}; Railpack builds only {}. Pin x-machines to Machines it builds for",
                named(&machine.machine),
                RAILPACK_PLATFORMS.join(" and ")
            )));
        };
        required
            .entry((*platform).to_owned())
            .or_default()
            .push(&machine.machine);
    }
    Ok(required)
}

fn named(machine: &Machine) -> String {
    format!("{} ({})", machine.name, machine.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{BuildOptions, capture_build, plan_build};

    #[test]
    fn placement_machines_fix_railpack_platforms_and_explicit_ones_must_cover_them() {
        use ployz_core::{Machine, MachineId, MachineName, WireGuardPublicKey};

        let root =
            std::env::temp_dir().join(format!("ployz-cover-machines-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Dockerfile"), "FROM scratch\n").unwrap();
        let observed = |seed: u8, architecture: &str, membership| {
            MachineObservation::new(
                Machine {
                    // Letters keep the ID a YAML string inside `x-machines`.
                    id: MachineId::parse(char::from(b'a' + seed).to_string().repeat(32)).unwrap(),
                    name: MachineName::parse(format!("machine-{seed}")).unwrap(),
                    subnet: format!("10.210.{seed}.0/24").parse().unwrap(),
                    public_key: WireGuardPublicKey([seed; 32]),
                    public_ip: None,
                    advertised_endpoints: Vec::new(),
                    runtime: ployz_core::MachineRuntime {
                        architecture: architecture.into(),
                        ..Default::default()
                    },
                },
                membership,
            )
        };
        let machines = [
            observed(1, "x86_64", MembershipObservation::Up),
            observed(2, "aarch64", MembershipObservation::Suspect),
            // A Down Machine is never a placement, so it derives nothing.
            observed(3, "aarch64", MembershipObservation::Down),
        ];
        let capture = |compose: &str| {
            let mut project = crate::compose::parse_normalized(compose, &root).unwrap();
            // The client's default platform describes this host, not the Cluster.
            project
                .environment
                .insert("DOCKER_DEFAULT_PLATFORM".into(), "linux/amd64".into());
            let options = BuildOptions::default();
            let plan = plan_build(&project, &options).unwrap();
            let captured = capture_build(&plan, &options, &mut project).unwrap();
            let candidate = project.capture(
                ployz_core::ProjectName::parse("example").unwrap(),
                ployz_core::PlanOptions::default(),
                Vec::new(),
                None,
                Vec::new(),
            );
            (captured, candidate)
        };

        let (mut captured, candidate) = capture(&format!(
            "services:\n  pinned:\n    image: registry.invalid/pinned:1\n    build: {{context: ., x-recipe: railpack}}\n    x-machines: [{}]\n  anywhere:\n    image: registry.invalid/anywhere:1\n    build: {{context: ., x-recipe: railpack}}\n  file:\n    image: registry.invalid/file:1\n    build: .\n",
            machines[0].machine.id
        ));
        // Before derivation every target carries the host default.
        assert!(
            captured
                .targets
                .iter()
                .all(|target| target.platforms == ["linux/amd64"])
        );
        captured.cover_machines(&candidate, &machines).unwrap();
        let platforms = |captured: &CapturedBuild, name: &str| {
            captured
                .targets
                .iter()
                .find(|target| target.name == name)
                .unwrap()
                .platforms
                .clone()
        };
        assert_eq!(platforms(&captured, "pinned"), ["linux/amd64"]);
        assert_eq!(
            platforms(&captured, "anywhere"),
            ["linux/amd64", "linux/arm64"]
        );
        // A Dockerfile keeps the host default; Deploy's coverage check decides.
        assert_eq!(platforms(&captured, "file"), ["linux/amd64"]);
        assert!(captured.cover_machines(&candidate, &[]).is_ok());
        assert_eq!(
            platforms(&captured, "anywhere"),
            ["linux/amd64", "linux/arm64"]
        );

        let (mut captured, candidate) = capture(
            "services:\n  explicit:\n    image: registry.invalid/explicit:1\n    build: {context: ., x-recipe: railpack, platforms: [linux/amd64]}\n",
        );
        let error = captured
            .cover_machines(&candidate, &machines)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("'explicit'")
                && error.contains("linux/arm64")
                && error.contains("machine-2")
                && !error.contains("machine-3"),
            "{error}"
        );
        assert!(captured.cover_machines(&candidate, &machines[..1]).is_ok());

        // A possible placement Railpack cannot build for is refused before
        // compilation: no rerun could cover it.
        let unbuildable = [
            machines[0].clone(),
            observed(4, "riscv64", MembershipObservation::Up),
        ];
        let error = captured
            .cover_machines(&candidate, &unbuildable)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("machine-4")
                && error.contains("riscv64")
                && error.contains("x-machines"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
