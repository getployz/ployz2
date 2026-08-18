//! Deploy Intent and Plan Options shared by the CLI planner and `@ployz/sdk`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::RequestedServiceSpec;
use crate::ServiceName;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanOptions {
    pub force_recreate: bool,
    pub skip_health_monitor: bool,
    /// Caller-supplied entropy keeps the planner pure while varying equal-priority placement.
    pub placement_seed: u64,
}

/// One Service Name this Deploy will apply from the target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceAttempt {
    pub name: ServiceName,
}

/// Complete desired Services plus which of those Services this command applies.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeployIntent {
    pub target: Vec<RequestedServiceSpec>,
    pub apply: Vec<ServiceAttempt>,
    pub options: PlanOptions,
    #[serde(default, skip)]
    dependencies: BTreeMap<ServiceName, Vec<ServiceName>>,
}

impl DeployIntent {
    /// Complete desired Services, the Service Attempts this command applies, and planning options.
    ///
    /// Empty `apply` means apply nothing. Names in `apply` that are absent from
    /// `target` are not planned; they are not a prune.
    #[must_use]
    pub fn new(
        target: Vec<RequestedServiceSpec>,
        apply: Vec<ServiceAttempt>,
        options: PlanOptions,
    ) -> Self {
        Self {
            target,
            apply,
            options,
            dependencies: BTreeMap::new(),
        }
    }

    /// Target and apply set from every spec, in that order.
    #[must_use]
    pub fn apply_all<'a>(
        specs: impl IntoIterator<Item = &'a RequestedServiceSpec>,
        options: PlanOptions,
    ) -> Self {
        let target: Vec<_> = specs.into_iter().cloned().collect();
        let apply = target
            .iter()
            .map(|spec| ServiceAttempt {
                name: spec.name.clone(),
            })
            .collect();
        Self::new(target, apply, options)
    }

    /// One-spec target with apply set to that name.
    #[must_use]
    pub fn apply_one(spec: RequestedServiceSpec, options: PlanOptions) -> Self {
        let apply = vec![ServiceAttempt {
            name: spec.name.clone(),
        }];
        Self::new(vec![spec], apply, options)
    }

    /// Target from every loaded spec; `apply` is this command's Service Attempts.
    #[must_use]
    pub fn from_named_specs(
        services: &BTreeMap<String, RequestedServiceSpec>,
        dependencies: &BTreeMap<String, Vec<String>>,
        apply: Vec<ServiceAttempt>,
        options: PlanOptions,
    ) -> Self {
        let dependencies = dependencies
            .iter()
            .filter_map(|(name, deps)| {
                Some((
                    services.get(name)?.name.clone(),
                    deps.iter()
                        .filter_map(|dep| services.get(dep).map(|spec| spec.name.clone()))
                        .collect(),
                ))
            })
            .collect();
        Self::new(services.values().cloned().collect(), apply, options)
            .with_dependencies(dependencies)
    }

    /// `depends_on` edges used to expand and order `apply` inside the planner.
    #[must_use]
    pub fn with_dependencies(
        mut self,
        dependencies: BTreeMap<ServiceName, Vec<ServiceName>>,
    ) -> Self {
        self.dependencies = dependencies;
        self
    }

    /// Planner `depends_on` edges used to expand and order `apply`.
    #[must_use]
    pub fn dependencies(&self) -> &BTreeMap<ServiceName, Vec<ServiceName>> {
        &self.dependencies
    }
}
