//! Deterministic Envoy file-based xDS rendering and live apply.

use ployz_core::{
    ContainerId, ContainerObservation, HttpProtocol, INGRESS_VERIFY_PATH, IngressHost,
    IngressProxyBackend, Machine, ingress_proxy_backend,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{corrosion::ReplicatedStore, docker::LocalDocker, filesystem::atomic_write};

mod apply;

use super::{
    IngressProjection, IngressSite, WatchApply, newest_local_ingress, watch as watch_inputs,
};

pub(crate) const CONFIG_FILE: &str = "bootstrap.yaml";
const LDS_FILE: &str = "lds.yaml";
const RDS_FILE: &str = "rds.yaml";
const CDS_FILE: &str = "cds.yaml";
const HTTP_CONTAINER_PORT: u16 = 8080;
const ROUTE_TIMEOUT: &str = "60s";
const ROUTE_IDLE_TIMEOUT: &str = "75s";
const CONNECT_TIMEOUT: &str = "5s";
/// Envoy rejects `port_value` 0; `127.0.0.1:1` fails closed as HTTP 503.
const EMPTY_UPSTREAM_PORT: u16 = 1;

/// Static bootstrap: file-watched xDS, admin omitted.
pub(crate) const BOOTSTRAP: &str = "\
node:
  id: ingress
  cluster: ployz
dynamic_resources:
  lds_config:
    path_config_source:
      path: /config/lds.yaml
      watched_directory:
        path: /config
    resource_api_version: V3
  cds_config:
    path_config_source:
      path: /config/cds.yaml
      watched_directory:
        path: /config
    resource_api_version: V3
";

const LISTENER_TYPE: &str = "type.googleapis.com/envoy.config.listener.v3.Listener";
const ROUTE_TYPE: &str = "type.googleapis.com/envoy.config.route.v3.RouteConfiguration";
const CLUSTER_TYPE: &str = "type.googleapis.com/envoy.config.cluster.v3.Cluster";
const HCM_TYPE: &str = "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager";
const ROUTER_TYPE: &str = "type.googleapis.com/envoy.extensions.filters.http.router.v3.Router";

#[derive(Clone, Debug, Eq, PartialEq)]
struct IngressProcess {
    container_id: ContainerId,
    image: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WatchInput {
    projection: IngressProjection,
    process: Option<IngressProcess>,
}

impl WatchInput {
    fn derive(
        machine: &Machine,
        observations: &[ContainerObservation],
        certificates: &BTreeMap<IngressHost, crate::corrosion::CertificateRow>,
    ) -> Result<Self, ployz_core::IngressProxyServiceSpecError> {
        let process = newest_local_ingress(machine, observations)
            .map(|container| {
                let observation = container.as_observation();
                if ingress_proxy_backend(&observation.resolved_spec)? != IngressProxyBackend::Envoy
                {
                    return Err(ployz_core::IngressProxyServiceSpecError);
                }
                Ok(IngressProcess {
                    container_id: observation.container_id,
                    image: observation.resolved_spec.container.image.clone(),
                })
            })
            .transpose()?;
        Ok(Self {
            projection: IngressProjection::derive(machine, observations, certificates),
            process,
        })
    }
}

/// Watch the common projection and apply it through file-watched Envoy xDS.
pub(crate) async fn watch(
    machine: Machine,
    replicated: ReplicatedStore,
    config_file: PathBuf,
    docker: LocalDocker,
    shutdown: CancellationToken,
) -> io::Result<()> {
    replicated
        .require_ingress_proxy_backend(IngressProxyBackend::Envoy)
        .await
        .map_err(io::Error::other)?;
    watch_inputs(
        machine,
        replicated,
        shutdown,
        |machine, observations, certificates| {
            WatchInput::derive(machine, observations, certificates).map_err(io::Error::other)
        },
        async move |input| {
            let Some(process) = input.process.as_ref() else {
                return WatchApply::WaitForChange(io::Error::other(
                    "local Envoy Ingress Proxy container is missing",
                ));
            };
            classify_apply(
                apply::apply(&input.projection, &config_file, &process.image, &docker).await,
            )
        },
    )
    .await
}

fn classify_apply(result: Result<apply::ApplyOutcome, apply::Error>) -> WatchApply {
    match result {
        Ok(apply::ApplyOutcome::Activated { .. }) => WatchApply::Applied,
        Err(error @ apply::Error::ValidationRejected { .. }) => {
            WatchApply::WaitForChange(io::Error::other(error))
        }
        Err(error) => WatchApply::Retry(io::Error::other(error)),
    }
}

/// Return the Envoy bootstrap path beneath the shared ingress data root.
#[must_use]
pub(crate) fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ingress").join("envoy").join(CONFIG_FILE)
}

/// Concatenate the live xDS files the operator-visible config RPC returns.
///
/// # Errors
///
/// Returns when any live xDS file cannot be read.
pub(crate) fn read_generated_config(data_dir: &Path) -> io::Result<String> {
    let config_file = config_path(data_dir);
    let directory = config_file.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Envoy bootstrap path has no parent",
        )
    })?;
    let lds = fs::read_to_string(directory.join(LDS_FILE))?;
    let rds = fs::read_to_string(directory.join(RDS_FILE))?;
    let cds = fs::read_to_string(directory.join(CDS_FILE))?;
    Ok(format!("{lds}\n---\n{rds}\n---\n{cds}"))
}

/// Install the bootstrap and empty xDS required before the first Envoy process starts.
///
/// Existing bootstrap is authoritative and is only replaced by the validated apply path.
///
/// # Errors
///
/// Returns when the empty local projection cannot be rendered or durably written.
pub(crate) fn write_initial_config(machine: &Machine, config_file: &Path) -> Result<(), Error> {
    if config_file.try_exists()? {
        return Ok(());
    }
    let projection = IngressProjection::derive(machine, &[], &BTreeMap::new());
    let rendered = render(&projection)?;
    let parent = config_file
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config file has no parent"))?;
    fs::create_dir_all(parent)?;
    write_xds(parent, &rendered)?;
    atomic_write(config_file, BOOTSTRAP.as_bytes(), 0o644)?;
    Ok(())
}

fn write_xds(directory: &Path, rendered: &RenderedConfig) -> io::Result<()> {
    atomic_write(&directory.join(LDS_FILE), rendered.lds.as_bytes(), 0o644)?;
    atomic_write(&directory.join(RDS_FILE), rendered.rds.as_bytes(), 0o644)?;
    atomic_write(&directory.join(CDS_FILE), rendered.cds.as_bytes(), 0o644)
}

/// Rendered Envoy xDS tied to one projection digest.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RenderedConfig {
    lds: String,
    rds: String,
    cds: String,
    digest: String,
}

impl RenderedConfig {
    #[must_use]
    pub(crate) fn lds(&self) -> &str {
        &self.lds
    }

    #[must_use]
    pub(crate) fn rds(&self) -> &str {
        &self.rds
    }

    #[must_use]
    pub(crate) fn cds(&self) -> &str {
        &self.cds
    }

    #[must_use]
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
}

/// Failure while rendering Envoy configuration.
#[derive(Debug, Error)]
pub(crate) enum Error {
    /// A tagged fragment belongs to another concrete backend.
    #[error("cannot render caddy Ingress Proxy Fragment with envoy")]
    BackendMismatch,
    /// A bootstrap or xDS file operation failed.
    #[error("Envoy ingress filesystem operation failed: {0}")]
    Filesystem(#[from] io::Error),
}

/// Render one already-derived projection as file-based xDS.
///
/// # Errors
///
/// Returns [`Error::BackendMismatch`] when the projection contains a fragment
/// for another concrete backend.
pub(crate) fn render(projection: &IngressProjection) -> Result<RenderedConfig, Error> {
    if projection.global_fragment.is_some() || !projection.service_fragments.is_empty() {
        return Err(Error::BackendMismatch);
    }
    let digest = projection_digest(projection);
    Ok(RenderedConfig {
        lds: render_lds(&digest),
        rds: render_rds(projection, &digest),
        cds: render_cds(projection, &digest),
        digest,
    })
}

fn projection_digest(projection: &IngressProjection) -> String {
    let canonical = serde_json::to_vec(projection)
        .expect("Ingress Projection contains only serializable value types");
    hex::encode(Sha256::digest(canonical))
}

fn render_lds(digest: &str) -> String {
    format!(
        "\
version_info: \"{digest}\"
resources:
- \"@type\": {LISTENER_TYPE}
  name: ployz-http
  address:
    socket_address:
      address: 0.0.0.0
      port_value: {HTTP_CONTAINER_PORT}
  metadata:
    filter_metadata:
      com.ployz:
        projection_digest: {digest}
  filter_chains:
  - filters:
    - name: envoy.filters.network.http_connection_manager
      typed_config:
        \"@type\": {HCM_TYPE}
        stat_prefix: ingress_http
        codec_type: AUTO
        rds:
          route_config_name: http
          config_source:
            resource_api_version: V3
            path_config_source:
              path: /config/rds.yaml
              watched_directory:
                path: /config
        http_filters:
        - name: envoy.filters.http.router
          typed_config:
            \"@type\": {ROUTER_TYPE}
"
    )
}

fn render_rds(projection: &IngressProjection, digest: &str) -> String {
    let mut yaml = format!(
        "\
version_info: \"{digest}\"
resources:
- \"@type\": {ROUTE_TYPE}
  name: http
  metadata:
    filter_metadata:
      com.ployz:
        projection_digest: {digest}
  virtual_hosts:
"
    );
    for site in http_sites(projection) {
        let id = route_id(&site.hostname);
        let _ = writeln!(yaml, "  - name: {id}");
        let _ = writeln!(yaml, "    domains:");
        let _ = writeln!(yaml, "    - {}", site.hostname);
        let _ = writeln!(yaml, "    routes:");
        let _ = writeln!(yaml, "    - match:");
        let _ = writeln!(yaml, "        prefix: /");
        let _ = writeln!(yaml, "      route:");
        let _ = writeln!(yaml, "        cluster: {id}");
        let _ = writeln!(yaml, "        timeout: {ROUTE_TIMEOUT}");
        let _ = writeln!(yaml, "        idle_timeout: {ROUTE_IDLE_TIMEOUT}");
    }
    let _ = writeln!(yaml, "  - name: ployz-verify");
    let _ = writeln!(yaml, "    domains:");
    let _ = writeln!(yaml, "    - \"*\"");
    let _ = writeln!(yaml, "    routes:");
    let _ = writeln!(yaml, "    - match:");
    let _ = writeln!(yaml, "        path: {INGRESS_VERIFY_PATH}");
    let _ = writeln!(yaml, "      direct_response:");
    let _ = writeln!(yaml, "        status: 200");
    let _ = writeln!(yaml, "        body:");
    let _ = writeln!(yaml, "          inline_string: {}", projection.machine.id);
    yaml
}

fn render_cds(projection: &IngressProjection, digest: &str) -> String {
    let sites: Vec<_> = http_sites(projection).collect();
    if sites.is_empty() {
        return format!("version_info: \"{digest}\"\nresources: []\n");
    }
    let mut yaml = format!("version_info: \"{digest}\"\nresources:\n");
    for site in sites {
        let id = route_id(&site.hostname);
        let endpoints = site
            .route(HttpProtocol::Http)
            .expect("http_sites only yields HTTP publications");
        let _ = write!(
            yaml,
            concat!(
                "- \"@type\": {cluster_type}\n",
                "  name: {id}\n",
                "  type: STATIC\n",
                "  connect_timeout: {connect}\n",
                "  lb_policy: ROUND_ROBIN\n",
                "  metadata:\n",
                "    filter_metadata:\n",
                "      com.ployz:\n",
                "        projection_digest: {digest}\n",
                "  load_assignment:\n",
                "    cluster_name: {id}\n",
                "    endpoints:\n",
                "    - lb_endpoints:\n",
            ),
            cluster_type = CLUSTER_TYPE,
            id = id,
            connect = CONNECT_TIMEOUT,
            digest = digest,
        );
        if endpoints.is_empty() {
            yaml.push_str(&socket_endpoint("127.0.0.1", EMPTY_UPSTREAM_PORT));
        } else {
            for endpoint in endpoints {
                yaml.push_str(&socket_endpoint(endpoint.address.0, endpoint.port.get()));
            }
        }
    }
    yaml
}

fn socket_endpoint(address: impl std::fmt::Display, port: u16) -> String {
    format!(
        concat!(
            "      - endpoint:\n",
            "          address:\n",
            "            socket_address:\n",
            "              address: {address}\n",
            "              port_value: {port}\n",
        ),
        address = address,
        port = port,
    )
}

fn http_sites(projection: &IngressProjection) -> impl Iterator<Item = &IngressSite> {
    projection
        .sites
        .iter()
        .filter(|site| site.route(HttpProtocol::Http).is_some())
}

fn route_id(hostname: &IngressHost) -> String {
    format!("ployz-http-{hostname}")
}

#[cfg(test)]
mod watch_tests {
    use super::apply::{ApplyOutcome, Error as ApplyError};
    use super::*;

    #[test]
    fn rejection_waits_for_changed_input() {
        assert!(matches!(
            classify_apply(Err(ApplyError::ValidationRejected {
                digest: "digest".into(),
                reason: "invalid".into(),
            })),
            WatchApply::WaitForChange(_)
        ));
    }

    #[test]
    fn transient_apply_error_retries() {
        assert!(matches!(
            classify_apply(Err(ApplyError::Filesystem(io::Error::other("temporary")))),
            WatchApply::Retry(_)
        ));
    }

    #[test]
    fn activated_apply_is_terminal_for_this_input() {
        assert!(matches!(
            classify_apply(Ok(ApplyOutcome::Activated {
                digest: "digest".into(),
            })),
            WatchApply::Applied
        ));
    }
}

#[cfg(test)]
#[path = "../envoy_tests.rs"]
mod tests;
