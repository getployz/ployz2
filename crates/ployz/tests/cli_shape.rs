#[cfg(unix)]
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::process::{Command as ProcessCommand, Stdio};

use clap_complete::{Shell, generate};

#[test]
fn listing_json_output_accepts_only_json_in_long_and_short_forms() {
    let paths: &[&[&str]] = &[
        &["changes"],
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
    }
}

#[cfg(unix)]
#[test]
fn completion_exits_on_sigpipe_when_the_reader_closes_after_one_line() {
    use std::os::unix::process::ExitStatusExt;

    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_ployz"))
        .args(["completion", "bash"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut first_line = String::new();
    output.read_line(&mut first_line).unwrap();
    drop(output);

    assert!(!first_line.is_empty());
    assert_eq!(child.wait().unwrap().signal(), Some(13));
}

#[test]
fn remote_build_target_requires_equals_and_preserves_positional_service() {
    for (args, target, service) in [
        (
            vec!["ployz", "build", "--remote=tower", "api"],
            "tower",
            "api",
        ),
        (vec!["ployz", "build", "--remote", "api"], "", "api"),
    ] {
        let matches = ployz::cli::command().try_get_matches_from(args).unwrap();
        let build = matches.subcommand_matches("build").unwrap();
        assert_eq!(
            build.get_one::<String>("remote").map(String::as_str),
            Some(target)
        );
        assert_eq!(
            build
                .get_many::<String>("service")
                .unwrap()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [service]
        );
    }
    assert!(
        ployz::cli::command()
            .try_get_matches_from(["ployz", "build", "--local", "--remote=tower"])
            .is_err()
    );
}

#[test]
fn machine_policy_flags_are_independent_boolean_values_and_legacy_ingress_is_rejected() {
    for path in [
        vec!["machine", "update", "node"],
        vec!["machine", "init"],
        vec!["machine", "add", "root@node"],
        vec!["cloud", "enroll", "pmet_test"],
    ] {
        let mut args = vec!["ployz"];
        args.extend(path);
        let mut valid = args.clone();
        valid.extend([
            "--accepts-builds=true",
            "--accepts-services=false",
            "--accepts-ingress=true",
            "--label-add",
            "region=west",
            "--label-add",
            "disk=ssd",
            "--label-rm",
            "retired",
        ]);
        let matches = ployz::cli::command().try_get_matches_from(valid).unwrap();
        let mut leaf = &matches;
        while let Some((_, child)) = leaf.subcommand() {
            leaf = child;
        }
        assert_eq!(leaf.get_one::<bool>("accepts-builds"), Some(&true));
        assert_eq!(leaf.get_one::<bool>("accepts-services"), Some(&false));
        assert_eq!(leaf.get_one::<bool>("accepts-ingress"), Some(&true));
        assert_eq!(leaf.get_many::<String>("label-add").unwrap().count(), 2);
        for invalid in [
            "--no-ingress",
            "--accepts-services",
            "--accepts-builds=maybe",
        ] {
            let mut invalid_args = args.clone();
            invalid_args.push(invalid);
            assert!(
                ployz::cli::command()
                    .try_get_matches_from(invalid_args)
                    .is_err()
            );
        }
    }
}

#[test]
fn ingress_deploy_accepts_repeated_constraints_and_rejects_legacy_machine_selection() {
    let matches = ployz::cli::command()
        .try_get_matches_from([
            "ployz",
            "ingress",
            "deploy",
            "--constraint",
            "node.labels.region==west",
            "--constraint",
            "node.labels.retired!=true",
        ])
        .unwrap();
    let deploy = matches
        .subcommand_matches("ingress")
        .unwrap()
        .subcommand_matches("deploy")
        .unwrap();
    assert_eq!(
        deploy
            .get_many::<String>("constraint")
            .unwrap()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["node.labels.region==west", "node.labels.retired!=true"]
    );
    assert!(
        ployz::cli::command()
            .try_get_matches_from(["ployz", "ingress", "deploy", "--machine", "edge",])
            .is_err()
    );
}
