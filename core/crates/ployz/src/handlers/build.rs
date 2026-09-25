//! `ployz build`: check out one Git Service's commit, refuse unless its build inputs
//! match the expected fingerprint, build it with local Buildx, and push it into one
//! Machine with a Build Grant. It knows nothing of the CI system running it.

use std::{collections::BTreeMap, path::Path, process::Command};

use clap::ArgMatches;
use ployz_core::{
    BuildGrant, RETAINED_DIGEST_TAG_PREFIX, ServiceName,
    config::{ServiceSource, parse_service_config},
};
use serde_json::{Value, json};

use super::{Error, leaf_matches, required, runtime};
use crate::{
    build::LocalImage,
    connect::{ManagementRelay, open_grant_registry},
    sdk::{PreparationInput, preparation::capture},
};

pub(super) fn build(root: &ArgMatches) -> Result<(), Error> {
    let matches = leaf_matches(root);
    let grant = BuildGrant::parse(required(matches, "grant")?)?;
    let expected = required(matches, "fingerprint")?;
    let commit = required(matches, "commit")?;
    let source = Path::new(
        matches
            .get_one::<String>("source")
            .expect("source has a default"),
    )
    .canonicalize()?;
    let deployment: Value =
        serde_json::from_slice(&std::fs::read(required(matches, "deployment")?)?)?;
    let service = git_service(&deployment)?;
    if !(commit.len() == 40
        && commit
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    {
        return Err(Error::usage("--commit must be a full lowercase Git SHA"));
    }
    check_out(&source, &commit)?;
    let captured = capture(PreparationInput {
        deployment,
        sources: BTreeMap::from([(service.clone(), source)]),
        source_commits: BTreeMap::from([(service.clone(), commit)]),
        build_receipts: BTreeMap::new(),
        build_index: 0,
    })?;
    if captured.fingerprints.get(&service) != Some(&expected) {
        return Err(Error::usage(format!(
            "the checkout's build inputs do not match the expected fingerprint; refusing to build. \
             The fingerprint covers this ployz version ({}), so install the version that computed it",
            env!("CARGO_PKG_VERSION")
        )));
    }
    runtime()?.block_on(async {
        let cancellation = crate::cancellation::on_ctrl_c();
        let build = captured.build;
        let built = tokio::task::spawn_blocking(move || build.execute_local(&cancellation))
            .await
            .map_err(std::io::Error::other)?
            .map_err(|error| Error::usage(error.to_string()))?;
        push(&grant, &built).await
    })
}

/// The deployment's single Git-sourced Service: the one this command builds.
fn git_service(deployment: &Value) -> Result<ServiceName, Error> {
    let mut services = deployment
        .get("snapshots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|snapshot| parse_service_config(snapshot.get("config")?.clone()).ok())
        .filter(|config| matches!(config.settings.source, ServiceSource::Git { .. }))
        .map(|config| config.settings.private_dns);
    match (services.next(), services.next()) {
        (Some(service), None) => Ok(service),
        _ => Err(Error::usage(
            "--deployment must hold exactly one Git-sourced Service",
        )),
    }
}

/// Check out `commit`, fetching it from `origin` when absent, and require a clean tree.
fn check_out(source: &Path, commit: &str) -> Result<(), Error> {
    if git(source, &["rev-parse", "HEAD"])? != commit {
        git(source, &["fetch", "--quiet", "--depth=1", "origin", commit])?;
        git(source, &["checkout", "--quiet", "--detach", commit])?;
    }
    if git(source, &["rev-parse", "HEAD"])? != commit {
        return Err(Error::usage(format!("the checkout is not at {commit}")));
    }
    if !git(source, &["status", "--porcelain"])?.is_empty() {
        return Err(Error::usage(
            "the checkout has changes outside its commit; refusing to build",
        ));
    }
    Ok(())
}

fn git(source: &Path, args: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(Error::usage(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Push through the grant with `docker push`, which sends only the layers the Machine
/// lacks, and print what the Machine verified it received.
async fn push(grant: &BuildGrant, built: &LocalImage) -> Result<(), Error> {
    let digest = &built.image.reference;
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| Error::usage("the built image is not identified by a SHA-256 digest"))?;
    let tag = format!("{}:{RETAINED_DIGEST_TAG_PREFIX}{hex}", built.repository);
    let registry = open_grant_registry(grant, &ManagementRelay::default()).await?;
    let local = format!("{}/{tag}", registry.address());
    docker(&["tag", digest, &local]).await?;
    let pushed = docker(&["push", "--quiet", &local]).await;
    // The loopback name is meaningless once this command exits.
    let _ = docker(&["image", "rm", &local]).await;
    if let Err(error) = pushed {
        return Err(registry.refusal().map_or(error, Error::usage));
    }
    // The Machine stores a tagged manifest only when its bytes hash to the tag's
    // digest, so a completed push is the Machine's confirmation of `digest`.
    println!(
        "{}",
        json!({"digest": digest, "tag": tag, "platforms": built.image.platforms})
    );
    Ok(())
}

async fn docker(args: &[&str]) -> Result<(), Error> {
    let output = tokio::process::Command::new("docker")
        .args(args)
        .output()
        .await?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error::usage(format!(
        "docker {} failed: {}",
        args.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
