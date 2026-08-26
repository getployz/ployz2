//! CLI shape rules that stand on their own.
//!
//! The frozen upstream command-page oracle was removed with `evidence/`; the
//! clap tree is no longer diffed against the 58 reference pages.

use clap_complete::{Shell, generate};

#[test]
fn listing_json_output_accepts_only_json_in_long_and_short_forms() {
    let paths: &[&[&str]] = &[
        &["ls"],
        &["ps"],
        &["service", "ls"],
        &["volume", "ls"],
        &["project", "ls"],
    ];
    for path in paths {
        for flag in ["--output", "-o"] {
            let mut args = vec!["ployz"];
            args.extend_from_slice(path);
            args.extend([flag, "json"]);
            let matches = ployz::cli::command().try_get_matches_from(args).unwrap();
            let mut leaf = &matches;
            while let Some((_, child)) = leaf.subcommand() {
                leaf = child;
            }
            assert_eq!(
                leaf.get_one::<String>("output").map(String::as_str),
                Some("json"),
                "{} {flag}",
                path.join(" ")
            );
        }

        let mut args = vec!["ployz"];
        args.extend_from_slice(path);
        args.extend(["--output", "yaml"]);
        assert!(
            ployz::cli::command().try_get_matches_from(args).is_err(),
            "{} accepted a non-JSON output format",
            path.join(" ")
        );
    }
}

#[test]
fn ployz_owned_environment_surface_is_frozen() {
    assert_eq!(
        ployz::cli::env::ALL,
        [
            "PLOYZ_AUTO_CONFIRM",
            "COMPOSE_DISABLE_ENV_FILE",
            "COMPOSE_FILE",
            "COMPOSE_PROJECT_NAME",
            "PLOYZ_CONFIG",
            "PLOYZ_CONNECT",
            "PLOYZ_CONTEXT",
            "PLOYZ_DAEMON_VERSION",
            "DEBUG",
            "PLOYZ_FAILED_CONTAINER_LOGS_TAIL",
            "PLOYZ_HEALTH_MONITOR_PERIOD",
            "PLOYZ_SSH_CONTROL_PERSIST",
        ]
    );
}

#[test]
fn native_completion_is_generated_for_every_supported_shell() {
    for shell in [
        Shell::Bash,
        Shell::Elvish,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Zsh,
    ] {
        let mut output = Vec::new();
        generate(shell, &mut ployz::cli::command(), "ployz", &mut output);
        let output = String::from_utf8(output).unwrap();
        assert!(!output.is_empty(), "empty {shell:?} completion");
        assert!(output.contains("ployz"), "unnamed {shell:?} completion");
        assert!(
            !output.contains("UNCLOUD"),
            "legacy name in {shell:?} completion"
        );
    }
}
