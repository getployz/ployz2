//! Static Envoy bootstrap for a healthy reserved Ingress Proxy process.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::filesystem::atomic_write;

pub(crate) const CONFIG_FILE: &str = "bootstrap.yaml";

/// Minimal static bootstrap: admin omitted, listeners serve nothing yet.
pub(crate) const BOOTSTRAP: &str = "\
node:
  id: ingress
  cluster: ployz
static_resources:
  listeners: []
  clusters: []
";

/// Return the Envoy bootstrap path beneath the shared ingress data root.
#[must_use]
pub(crate) fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ingress").join("envoy").join(CONFIG_FILE)
}

/// Install the static bootstrap required before the first Envoy process starts.
///
/// Existing configuration is authoritative and is only replaced by later apply.
///
/// # Errors
///
/// Returns when the bootstrap cannot be durably written.
pub(crate) fn write_initial_config(config_file: &Path) -> io::Result<()> {
    if config_file.try_exists()? {
        return Ok(());
    }
    let parent = config_file
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config file has no parent"))?;
    fs::create_dir_all(parent)?;
    atomic_write(config_file, BOOTSTRAP.as_bytes(), 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_initial_config_is_idempotent_and_leaves_existing_bytes() {
        let root = std::env::temp_dir().join(format!(
            "ployz-envoy-bootstrap-{}",
            ployz_core::MachineId::random()
        ));
        let config_file = root.join("ingress/envoy/bootstrap.yaml");
        write_initial_config(&config_file).unwrap();
        assert_eq!(fs::read_to_string(&config_file).unwrap(), BOOTSTRAP);
        assert!(!BOOTSTRAP.contains("admin:"));
        fs::write(&config_file, "authoritative\n").unwrap();
        write_initial_config(&config_file).unwrap();
        assert_eq!(fs::read_to_string(&config_file).unwrap(), "authoritative\n");
        let _ = fs::remove_dir_all(root);
    }
}
