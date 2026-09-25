use std::{collections::BTreeMap, fs, process::Command};

use ployz::context::{Config, Connection, Context};

#[test]
fn context_commands_list_show_and_persist_an_explicit_selection() {
    let root = std::env::temp_dir().join(format!("ployz-context-cli-{}", std::process::id()));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([
            (
                "dev".into(),
                Context {
                    connections: vec![Connection::unix("/tmp/dev.sock").unwrap()],
                },
            ),
            (
                "prod".into(),
                Context {
                    connections: vec![Connection::unix("/tmp/prod.sock").unwrap()],
                },
            ),
        ]),
    )
    .save()
    .unwrap();

    let selected = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "--ployz-config",
            path.to_str().unwrap(),
            "ctx",
            "use",
            "dev",
        ])
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(Config::load(&path).unwrap().current_context(), Some("dev"));

    let listed = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args(["ctx", "ls", "--ployz-config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(listed.contains("dev"));
    assert!(listed.contains("prod"));

    let shown = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args(["ctx", "show", "--ployz-config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(String::from_utf8(shown.stdout).unwrap().trim(), "dev");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ctx_connection_selects_and_persists_across_invocations() {
    let root = std::env::temp_dir().join(format!(
        "ployz-ctx-connection-select-{}",
        std::process::id()
    ));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([(
            "prod".into(),
            Context {
                connections: vec![
                    Connection::unix("/tmp/prod-a.sock").unwrap(),
                    Connection::unix("/tmp/prod-b.sock").unwrap(),
                ],
            },
        )]),
    )
    .save()
    .unwrap();

    let selected = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "ctx",
            "connection",
            "unix:///tmp/prod-b.sock",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );

    let shown = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "ctx",
            "connection",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(
        String::from_utf8(shown.stdout).unwrap().trim(),
        "unix:///tmp/prod-b.sock"
    );
    assert_eq!(
        Config::load(&path)
            .unwrap()
            .contexts
            .get("prod")
            .unwrap()
            .connections
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["unix:///tmp/prod-b.sock", "unix:///tmp/prod-a.sock"]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_config_beats_environment_and_interactive_errors_do_not_mutate() {
    let root = std::env::temp_dir().join(format!("ployz-context-env-{}", std::process::id()));
    let flag_path = root.join("flag.yaml");
    let env_path = root.join("env.yaml");
    let _ = fs::remove_dir_all(&root);
    let contexts = BTreeMap::from([
        (
            "dev".into(),
            Context {
                connections: vec![Connection::unix("/tmp/dev.sock").unwrap()],
            },
        ),
        (
            "prod".into(),
            Context {
                connections: vec![Connection::unix("/tmp/prod.sock").unwrap()],
            },
        ),
    ]);
    Config::new(&flag_path, Some("prod".into()), contexts.clone())
        .save()
        .unwrap();
    Config::new(&env_path, Some("prod".into()), contexts)
        .save()
        .unwrap();

    let selected = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "--ployz-config",
            flag_path.to_str().unwrap(),
            "ctx",
            "use",
            "dev",
        ])
        .env("PLOYZ_CONFIG", &env_path)
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(
        Config::load(&flag_path).unwrap().current_context(),
        Some("dev")
    );
    assert_eq!(
        Config::load(&env_path).unwrap().current_context(),
        Some("prod")
    );

    let before = Config::load(&flag_path).unwrap();
    for args in [vec!["ctx"], vec!["ctx", "use"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
            .args(args)
            .args(["--ployz-config", flag_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(Config::load(&flag_path).unwrap(), before);
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_filename_only_config_override_saves_in_the_current_directory() {
    let root = std::env::temp_dir().join(format!("ployz-relative-config-{}", std::process::id()));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([
            ("dev".into(), Context::default()),
            ("prod".into(), Context::default()),
        ]),
    )
    .save()
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .current_dir(&root)
        .args(["--ployz-config", "config.yaml", "ctx", "use", "dev"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(Config::load(&path).unwrap().current_context(), Some("dev"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ctx_rm_keeps_current_for_another_context_and_unsets_it_for_the_current_one() {
    let root = std::env::temp_dir().join(format!("ployz-ctx-rm-current-{}", std::process::id()));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([
            (
                "default".into(),
                Context {
                    connections: vec![Connection::unix("/tmp/default.sock").unwrap()],
                },
            ),
            (
                "dev".into(),
                Context {
                    connections: vec![Connection::unix("/tmp/dev.sock").unwrap()],
                },
            ),
            (
                "prod".into(),
                Context {
                    connections: vec![Connection::unix("/tmp/prod.sock").unwrap()],
                },
            ),
        ]),
    )
    .save()
    .unwrap();

    let removed = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "ctx",
            "rm",
            "default",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert_eq!(
        String::from_utf8(removed.stdout).unwrap().trim(),
        "Removed context default."
    );
    let config = Config::load(&path).unwrap();
    assert_eq!(config.current_context(), Some("prod"));
    assert!(!config.contexts.contains_key("default"));
    let listed = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args(["ctx", "ls", "--ployz-config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(!listed.contains("default"), "{listed}");
    assert!(listed.contains("prod"), "{listed}");

    let removed = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "ctx",
            "rm",
            "prod",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    let stdout = String::from_utf8(removed.stdout).unwrap();
    assert!(stdout.contains("Removed context prod."), "{stdout}");
    assert!(stdout.contains("Current context is now unset."), "{stdout}");

    let shown = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args(["ctx", "show", "--ployz-config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(String::from_utf8(shown.stdout).unwrap().trim(), "");

    let config = Config::load(&path).unwrap();
    assert_eq!(config.current_context(), None);
    assert!(!config.contexts.contains_key("prod"));
    let yaml = fs::read_to_string(&path).unwrap();
    assert!(
        !yaml.contains("current_context:"),
        "dangling current context was stored: {yaml}"
    );

    let selected = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "ctx",
            "use",
            "dev",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_eq!(Config::load(&path).unwrap().current_context(), Some("dev"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ctx_rm_of_an_unknown_name_fails_without_mutating() {
    let root = std::env::temp_dir().join(format!("ployz-ctx-rm-unknown-{}", std::process::id()));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    let before = Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([(
            "prod".into(),
            Context {
                connections: vec![Connection::unix("/tmp/prod.sock").unwrap()],
            },
        )]),
    );
    before.save().unwrap();

    for action in ["use", "rm"] {
        for name in ["gone", "gone\n\u{1b}[2J"] {
            let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
                .args([
                    "ctx",
                    action,
                    name,
                    "--ployz-config",
                    path.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains(&format!("context {} not found", name.escape_debug())),
                "{error}"
            );
            assert!(!error.trim().chars().any(char::is_control), "{error}");
            assert_eq!(Config::load(&path).unwrap(), before);
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ctx_rm_rejects_a_direct_connection() {
    let root = std::env::temp_dir().join(format!("ployz-ctx-rm-connect-{}", std::process::id()));
    let path = root.join("config.yaml");
    let _ = fs::remove_dir_all(&root);
    let before = Config::new(
        &path,
        Some("prod".into()),
        BTreeMap::from([(
            "prod".into(),
            Context {
                connections: vec![Connection::unix("/tmp/prod.sock").unwrap()],
            },
        )]),
    );
    before.save().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .args([
            "--connect",
            "tcp://127.0.0.1:1",
            "ctx",
            "rm",
            "prod",
            "--ployz-config",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "context management is unavailable with a direct connection"
    );
    assert_eq!(Config::load(&path).unwrap(), before);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn management_context_selection_and_listing_never_print_capabilities() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("config.yaml");
    let secret = ployz_core::ManagementCapability::new(
        ployz_core::ManagementIdentity::from_bytes([1; 32]),
        [2; 32],
    )
    .to_secret_string();
    Config::new(
        &path,
        Some("private".into()),
        BTreeMap::from([(
            "private".into(),
            Context {
                connections: vec![Connection::management(&secret).unwrap()],
            },
        )]),
    )
    .save()
    .unwrap();
    for args in [
        vec!["ctx", "ls"],
        vec!["ctx", "show"],
        vec!["ctx", "connection"],
        vec!["ctx", "connection", "management:[redacted]"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
            .arg("--ployz-config")
            .arg(&path)
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(&secret));
        if args.get(1) == Some(&"connection") {
            assert!(String::from_utf8_lossy(&output.stdout).contains("management:[redacted]"));
        }
    }
}

#[test]
fn management_selection_uses_machine_labels_or_ordered_indices_in_a_mixed_context() {
    use ployz::context::SshDestination;
    use ployz_core::MachineId;
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("config.yaml");
    let ssh = Connection::ssh(SshDestination::parse("root@example.com").unwrap());
    let first_secret = ployz_core::ManagementCapability::new(
        ployz_core::ManagementIdentity::from_bytes([3; 32]),
        [4; 32],
    )
    .to_secret_string();
    let second_secret = ployz_core::ManagementCapability::new(
        ployz_core::ManagementIdentity::from_bytes([5; 32]),
        [6; 32],
    )
    .to_secret_string();
    let first = Connection::management(&first_secret)
        .unwrap()
        .with_machine_id(MachineId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap());
    let second = Connection::management(&second_secret)
        .unwrap()
        .with_machine_id(MachineId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap());
    Config::new(
        &path,
        Some("mixed".into()),
        BTreeMap::from([(
            "mixed".into(),
            Context {
                connections: vec![ssh.clone(), first.clone(), second.clone()],
            },
        )]),
    )
    .save()
    .unwrap();
    // Selecting moves only that entry to the front; the rest keep their order.
    for (selector, expected) in [
        (
            "2".to_owned(),
            vec![first.clone(), ssh.clone(), second.clone()],
        ),
        (
            second.to_string(),
            vec![second.clone(), first.clone(), ssh.clone()],
        ),
        (
            ssh.to_string(),
            vec![ssh.clone(), second.clone(), first.clone()],
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
            .arg("--ployz-config")
            .arg(&path)
            .args(["ctx", "connection", &selector])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!rendered.contains(&first_secret));
        assert!(!rendered.contains(&second_secret));
        assert_eq!(
            Config::load(&path)
                .unwrap()
                .contexts
                .get("mixed")
                .unwrap()
                .connections,
            expected
        );
    }
}

#[test]
fn ambiguous_management_labels_fail_without_mutation_and_index_selects_the_second() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = root.path().join("config.yaml");
    let first = Connection::management(
        ployz_core::ManagementCapability::new(
            ployz_core::ManagementIdentity::from_bytes([1; 32]),
            [2; 32],
        )
        .to_secret_string(),
    )
    .unwrap();
    let second = Connection::management(
        ployz_core::ManagementCapability::new(
            ployz_core::ManagementIdentity::from_bytes([3; 32]),
            [4; 32],
        )
        .to_secret_string(),
    )
    .unwrap();
    let config = Config::new(
        &path,
        Some("private".into()),
        BTreeMap::from([(
            "private".into(),
            Context {
                connections: vec![first.clone(), second.clone()],
            },
        )]),
    );
    config.save().unwrap();
    // (selector, stderr, whether stderr is the whole message)
    for (selector, message, exact) in [
        ("management:[redacted]", "ambiguous", false),
        ("0", "out of range", false),
        ("3", "out of range", false),
        (
            "unix:///tmp/missing.sock",
            r#"connection "unix:///tmp/missing.sock" not found"#,
            true,
        ),
        (
            "unix:///tmp/missing socket\n\u{1b}[2J",
            r#"connection "unix:///tmp/missing socket\n\u{1b}[2J" not found"#,
            true,
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ployz"))
            .arg("--ployz-config")
            .arg(&path)
            .args(["ctx", "connection", selector])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        if exact {
            assert_eq!(stderr.trim(), message);
        } else {
            assert!(stderr.contains(message), "{stderr}");
        }
        assert_eq!(Config::load(&path).unwrap(), config);
    }
    let selected = Command::new(env!("CARGO_BIN_EXE_ployz"))
        .arg("--ployz-config")
        .arg(&path)
        .args(["ctx", "connection", "2"])
        .output()
        .unwrap();
    assert!(selected.status.success());
    assert_eq!(
        Config::load(&path)
            .unwrap()
            .contexts
            .get("private")
            .unwrap()
            .connections,
        vec![second, first]
    );
}
