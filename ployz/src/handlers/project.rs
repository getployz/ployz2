use std::io::{self, IsTerminal, Write};

use clap::ArgMatches;
use ployz_core::{ProjectName, derive_projects};

use crate::{
    deploy::{DeployPreview, DeploySnapshot, VolumeFate, execute_confirmed, render},
    project::refuse_reserved,
};

use super::{Error, leaf_matches, required, with_client};

pub(super) fn list(root: &ArgMatches) -> Result<(), Error> {
    with_client(root, |client| {
        Box::pin(async move {
            let machines = client.machines().await?;
            let snapshot = client.deploy_snapshot(machines).await?;
            for line in observer_listing_warnings(&snapshot) {
                eprintln!("{line}");
            }
            println!("PROJECT\tSERVICES\tVOLUMES");
            for project in derive_projects(
                &snapshot.containers,
                snapshot
                    .volumes
                    .iter()
                    .map(|volume| (&volume.id, &volume.labels)),
            ) {
                println!(
                    "{}\t{}\t{}",
                    project.name,
                    project.services.len(),
                    project.volumes.len()
                );
            }
            Ok(())
        })
    })
}

pub(super) fn remove(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let name = ProjectName::parse(required(matches, "project")?)?;
    refuse_reserved(&name)?;
    let volumes = if matches.get_flag("volumes") {
        VolumeFate::Destroy
    } else {
        VolumeFate::Preserve
    };
    let yes = matches.get_flag("yes");
    let context = matches
        .get_one::<String>("context")
        .cloned()
        .unwrap_or_else(|| "default".into());
    with_client(root, move |client| {
        Box::pin(async move {
            let preview = client.preview_project_removal(&name, volumes).await?;
            for warning in &preview.warnings {
                eprintln!("WARNING: {warning}");
            }
            if let Some(reason) = preview.prune_refusal {
                print!("{}", render::removal_plan_text(&preview, &context));
                return Err(Error::usage(reason.to_string()));
            }
            if project_not_found(&preview) {
                return Err(Error::usage(format!("Project '{name}' was not found")));
            }
            print!("{}", render::removal_plan_text(&preview, &context));
            if preview.operations.is_empty() {
                return Ok(());
            }
            if !yes && !confirm_removal(&name, &context)? {
                println!("No changes were made.");
                return Ok(());
            }
            execute_confirmed(
                client,
                &preview,
                format!("Removing Project {name} from {context}"),
            )
            .await
        })
    })
}

fn confirm_removal(project: &ProjectName, context: &str) -> Result<bool, Error> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::usage(
            "confirmation requires a terminal; pass --yes to continue",
        ));
    }
    print!("{}", render::confirm_removal_prompt(project, context));
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y" | "yes" | "YES"))
}

fn project_not_found(preview: &DeployPreview) -> bool {
    preview.prune_refusal.is_none()
        && preview.operations.is_empty()
        && preview.preserved_volumes.is_empty()
        && preview.would_remove.is_empty()
}

fn observer_listing_warnings(snapshot: &DeploySnapshot) -> Vec<String> {
    let mut lines =
        vec!["WARNING: Live Observation is observer-relative and not globally complete".into()];
    lines.extend(snapshot.container_failures.iter().map(|failure| {
        format!(
            "WARNING: Machine {} failed: {}",
            failure.machine_id, failure.error.message
        )
    }));
    lines.extend(
        snapshot
            .container_omissions
            .iter()
            .map(|machine_id| format!("WARNING: Machine {machine_id} was omitted")),
    );
    lines.extend(snapshot.volume_failures.iter().map(|failure| {
        format!(
            "WARNING: Machine {} failed listing volumes: {}",
            failure.machine_id, failure.error.message
        )
    }));
    lines.extend(
        snapshot
            .volume_omissions
            .iter()
            .map(|machine_id| format!("WARNING: Machine {machine_id} was omitted listing volumes")),
    );
    lines
}

#[cfg(test)]
mod tests {
    use ployz_core::{
        MachineFailure, MachineId, ProjectName, PruneRefusal, RpcError, RpcErrorCode,
    };

    use super::*;

    #[test]
    fn listing_warnings_are_observer_relative_and_include_volume_gaps() {
        let machine = MachineId::parse("1".repeat(32)).unwrap();
        let snapshot = DeploySnapshot {
            container_failures: vec![MachineFailure {
                machine_id: machine,
                error: RpcError {
                    code: RpcErrorCode::Unavailable,
                    message: "down".into(),
                    details: serde_json::Value::Null,
                },
            }],
            volume_omissions: vec![machine],
            ..Default::default()
        };
        assert_eq!(
            observer_listing_warnings(&snapshot),
            [
                "WARNING: Live Observation is observer-relative and not globally complete".into(),
                format!("WARNING: Machine {machine} failed: down"),
                format!("WARNING: Machine {machine} was omitted listing volumes"),
            ]
        );
    }

    #[test]
    fn incomplete_empty_view_is_not_reported_as_missing() {
        let mut preview =
            DeployPreview::new(Vec::new(), Vec::new(), ProjectName::parse("shop").unwrap());
        assert!(project_not_found(&preview));
        preview.prune_refusal = Some(PruneRefusal::IncompleteSnapshot);
        assert!(!project_not_found(&preview));
    }
}
