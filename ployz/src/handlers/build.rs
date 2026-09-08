use clap::ArgMatches;

use crate::compose::{BuildOptions, LoadOptions, execute_build, load_project, plan_build};

use super::{Error, connect_client, leaf_matches, runtime, string_values};

pub(super) fn run(matches: &ArgMatches) -> Result<(), Error> {
    let leaf = leaf_matches(matches);
    let load = LoadOptions {
        command: "build".into(),
        files: string_values(leaf, "file")
            .into_iter()
            .map(Into::into)
            .collect(),
        profiles: string_values(leaf, "profile"),
        ..Default::default()
    };
    let options = BuildOptions {
        build_args: string_values(leaf, "build-arg"),
        check: leaf.get_flag("check"),
        deps: leaf.get_flag("deps"),
        no_cache: leaf.get_flag("no-cache"),
        pull: leaf.get_flag("pull"),
        push_registry: leaf.get_flag("push-registry"),
        services: string_values(leaf, "service"),
    };
    let project = load_project(&load)?;
    for warning in &project.warnings {
        eprintln!("WARNING: {warning}");
    }
    let plan = plan_build(&project, &options)?;
    if plan.is_empty() {
        println!("No buildable services selected.");
        return Ok(());
    }
    execute_build(&plan, &options, &load, &project)?;
    if options.check || !leaf.get_flag("push") {
        return Ok(());
    }

    let explicit = string_values(leaf, "machine");
    let context = project.selected_context(
        leaf.get_one::<String>("context").map(String::as_str),
        matches.get_one::<String>("connect").map(String::as_str),
    );
    let runtime = runtime()?;
    let failures = runtime.block_on(async {
        let mut client = connect_client(matches, context).await?;
        let mut failures = crate::failure::Failures::default();
        for service in &plan {
            let targets = push_targets(&explicit, &service.machines);
            match crate::image::push(&mut client, &service.image, None, &targets).await {
                Ok(result) => report_push(&mut failures, &service.image, result),
                Err(error) => push_failure(&mut failures, &service.image, error)?,
            }
        }
        Ok::<_, Error>(failures)
    })?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.into_failure(str::to_owned))
    }
}

pub(super) fn report_push(
    failures: &mut crate::failure::Failures,
    image: &str,
    result: ployz_core::PartialResult<(), crate::image::PushError>,
) {
    for success in result.successes {
        println!("Pushed {image} to {}", success.machine_id);
    }
    for failure in result.failures {
        failures.record(format!("{image} on {}", failure.machine_id), &failure.error);
    }
    for machine in result.omissions {
        failures.note(format!("{image} on {machine}"), "no terminal response");
    }
}

pub(super) fn push_targets(
    explicit: &[String],
    configured: &[ployz_core::MachineTarget],
) -> Vec<String> {
    if explicit.is_empty() {
        configured.iter().map(ToString::to_string).collect()
    } else {
        explicit.to_vec()
    }
}

fn push_failure(
    failures: &mut crate::failure::Failures,
    image: &str,
    error: crate::image::PushError,
) -> Result<(), Error> {
    if error.is_cancellation() {
        return Err(Error::context(format!("{image}: {error}"), error));
    }
    failures.record(image, &error);
    Ok(())
}

#[cfg(test)]
mod tests {
    use ployz_core::MachineTarget;

    use super::*;

    #[test]
    fn explicit_push_targets_override_service_targets_and_empty_means_all() {
        let configured = [MachineTarget::parse("service-machine").unwrap()];
        assert_eq!(
            push_targets(&["explicit-machine".into()], &configured),
            ["explicit-machine"]
        );
        assert_eq!(push_targets(&[], &configured), ["service-machine"]);
        assert!(push_targets(&[], &[]).is_empty());
    }

    #[test]
    fn internal_push_failures_stay_bugs_through_the_aggregate() {
        let mut failures = crate::failure::Failures::default();
        push_failure(
            &mut failures,
            "example.test/api",
            crate::image::PushError::ImageIngest(ployz_core::RpcError {
                code: ployz_core::RpcErrorCode::Internal,
                message: "boom".into(),
                details: serde_json::Value::Null,
            }),
        )
        .unwrap();
        let framed = failures.into_failure(str::to_owned).to_string();
        assert!(framed.contains("boom"), "{framed}");
        assert!(framed.contains("bug"), "{framed}");
        assert!(framed.contains("ployz version"), "{framed}");
    }

    #[test]
    fn cancellation_is_terminal_while_other_push_errors_accumulate() {
        let mut failures = crate::failure::Failures::default();
        assert!(
            push_failure(
                &mut failures,
                "example.test/api",
                crate::image::PushError::Cancelled
            )
            .is_err()
        );
        push_failure(
            &mut failures,
            "example.test/api",
            crate::image::PushError::ImageNotFound("example.test/api".into()),
        )
        .unwrap();
        let accumulated = failures.into_failure(str::to_owned).to_string();
        assert!(
            accumulated.contains("example.test/api: image 'example.test/api' not found locally"),
            "{accumulated}"
        );
    }
}
