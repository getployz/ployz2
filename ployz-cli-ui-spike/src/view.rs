//! Product-shaped DTOs for the spike. Presentation crates stay in adapters.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ServiceRow {
    pub service_id: &'static str,
    pub service: &'static str,
    pub containers: u32,
    pub hooks: u32,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ImageRow {
    pub machine: &'static str,
    pub id: &'static str,
    pub repository: &'static str,
    pub created: &'static str,
    pub size: &'static str,
    pub platforms: &'static str,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PlanView {
    pub context: &'static str,
    pub project: &'static str,
    pub operations: &'static [PlanOp],
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PlanOp {
    pub kind: &'static str,
    pub service: &'static str,
    pub detail: &'static str,
    pub machine: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ProgressRow {
    pub status: &'static str,
    pub name: &'static str,
    pub phase: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct DeployView {
    pub plan: PlanView,
    pub progress: &'static [ProgressRow],
}

#[derive(Clone, Debug, Serialize)]
pub struct Catalog {
    pub services: &'static [ServiceRow],
    pub images: &'static [ImageRow],
    pub plan: PlanView,
}

#[must_use]
pub fn op_mark(kind: &str) -> &'static str {
    match kind {
        "replace" => "~",
        "create" => "+",
        _ => "·",
    }
}

#[must_use]
pub fn services() -> &'static [ServiceRow] {
    &[
        ServiceRow {
            service_id: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
            service: "web/api",
            containers: 2,
            hooks: 1,
        },
        ServiceRow {
            service_id: "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5",
            service: "web/web",
            containers: 1,
            hooks: 0,
        },
        ServiceRow {
            service_id: "c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6",
            service: "web/worker",
            containers: 3,
            hooks: 0,
        },
    ]
}

#[must_use]
pub fn images() -> &'static [ImageRow] {
    &[
        ImageRow {
            machine: "ord1",
            id: "sha256:c4717a8",
            repository: "traefik/whoami:latest",
            created: "2 days ago",
            size: "18.3MB",
            platforms: "linux/amd64",
        },
        ImageRow {
            machine: "ord2",
            id: "sha256:9f21ab0",
            repository: "ghcr.io/acme/api:1.4",
            created: "4 hours ago",
            size: "84.1MB",
            platforms: "linux/amd64,linux/arm64",
        },
    ]
}

#[must_use]
pub fn plan() -> PlanView {
    PlanView {
        context: "prod",
        project: "web",
        operations: &[
            PlanOp {
                kind: "replace",
                service: "api",
                detail: "replace container api-1",
                machine: "ord1",
            },
            PlanOp {
                kind: "create",
                service: "api",
                detail: "create container api-2",
                machine: "ord2",
            },
        ],
    }
}

#[must_use]
pub fn progress_at(tick: u32) -> &'static [ProgressRow] {
    match tick {
        0 => &[
            ProgressRow {
                status: "pending",
                name: "api-1",
                phase: "replace",
            },
            ProgressRow {
                status: "pending",
                name: "api-2",
                phase: "create",
            },
        ],
        1 => &[
            ProgressRow {
                status: "done",
                name: "api-1",
                phase: "replace",
            },
            ProgressRow {
                status: "running",
                name: "api-2",
                phase: "create",
            },
        ],
        _ => progress_done(),
    }
}

#[must_use]
pub fn progress_done() -> &'static [ProgressRow] {
    &[
        ProgressRow {
            status: "done",
            name: "api-1",
            phase: "replace",
        },
        ProgressRow {
            status: "done",
            name: "api-2",
            phase: "create",
        },
    ]
}

#[must_use]
pub fn deploy_view() -> DeployView {
    DeployView {
        plan: plan(),
        progress: progress_done(),
    }
}

#[must_use]
pub fn catalog() -> Catalog {
    Catalog {
        services: services(),
        images: images(),
        plan: plan(),
    }
}
