//! Saved context mutation and credential persistence contracts.

#[test]
fn rotation_updates_latest_contexts_and_preserves_other_connections() {
    use super::*;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let old =
        ManagementCapability::new(ployz_core::ManagementIdentity::from_bytes([1; 32]), [2; 32]);
    let new = ManagementCapability::new(*old.machine(), [3; 32]);
    let connection = Connection::management(old.to_secret_string()).unwrap();
    let config = Config::new(
        dir.path().join("config.yaml"),
        Some("first".into()),
        BTreeMap::from([(
            "first".into(),
            Context {
                connections: vec![connection.clone()],
            },
        )]),
    );
    config.save().unwrap();
    // Another writer adds a context while enrollment is in flight.
    let mut latest = Config::load(config.path()).unwrap();
    let unix = Connection::unix("/tmp/ployz.sock").unwrap();
    latest.contexts.insert(
        "second".into(),
        Context {
            connections: vec![unix.clone(), connection],
        },
    );
    latest.save().unwrap();
    config.save_management_capability(&new).unwrap();
    let saved = Config::load(config.path()).unwrap();
    let replacement = Connection::management(new.to_secret_string()).unwrap();
    assert_eq!(
        saved.contexts.get("first").unwrap().connections,
        vec![replacement.clone()]
    );
    assert_eq!(
        saved.contexts.get("second").unwrap().connections,
        vec![unix, replacement]
    );
    assert_eq!(saved.current_context(), Some("first"));
    let unrelated =
        ManagementCapability::new(ployz_core::ManagementIdentity::from_bytes([4; 32]), [5; 32]);
    assert!(matches!(
        config.save_management_capability(&unrelated),
        Err(ConfigError::ManagementConnectionMissing)
    ));
    assert_eq!(Config::load(config.path()).unwrap(), saved);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        config.save_management_capability(&old),
        Err(ConfigError::PrivatePermissions(_))
    ));
}

use std::{collections::BTreeMap, fs};

use super::{Config, Context};

#[test]
fn a_dangling_current_name_is_dropped_on_construction_and_load() {
    let config = Config::new(
        "/tmp/config.yaml",
        Some("gone".into()),
        BTreeMap::from([("prod".into(), Context::default())]),
    );
    assert_eq!(config.current_context(), None);
    assert!(config.contexts.contains_key("prod"));

    let root = std::env::temp_dir().join(format!(
        "ployz-dangling-current-yaml-{}",
        std::process::id()
    ));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        &path,
        "current_context: gone\ncontexts:\n  prod:\n    connections: []\n",
    )
    .unwrap();

    let config = Config::load(&path).unwrap();
    assert_eq!(config.current_context(), None);
    assert!(config.contexts.contains_key("prod"));

    fs::remove_dir_all(root).unwrap();
}
