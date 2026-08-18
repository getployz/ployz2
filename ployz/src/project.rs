//! Resolve a Cluster-side Project name from Compose project-name precedence.

use std::{
    fmt, fs,
    path::{Component, Path, PathBuf},
};

use clap::{ArgMatches, parser::ValueSource};
use ployz_core::{ProjectName, ValueError};
use serde::Deserialize;
use thiserror::Error;

use crate::compose::ComposeProject;

/// Why a resolved Project name was chosen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectNameSource {
    CommandLine,
    ComposeProjectName,
    ComposeName,
    ComposeProjectDirectory,
    CurrentDirectory,
    Default,
}

impl fmt::Display for ProjectNameSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CommandLine => "command-line project name",
            Self::ComposeProjectName => "COMPOSE_PROJECT_NAME",
            Self::ComposeName => "top-level Compose name",
            Self::ComposeProjectDirectory => "Compose project directory",
            Self::CurrentDirectory => "current directory",
            Self::Default => "default",
        })
    }
}

/// One Project name plus the precedence level that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProject {
    pub name: ProjectName,
    pub source: ProjectNameSource,
}

/// Inputs for Compose project-name precedence. Absent sources are skipped.
#[derive(Clone, Debug, Default)]
pub struct ProjectNameInput<'a> {
    pub command_line: Option<&'a str>,
    pub compose_project_name: Option<&'a str>,
    pub compose_name: Option<&'a str>,
    pub compose_project_directory: Option<&'a Path>,
    pub current_directory: Option<&'a Path>,
    pub implicit_default: bool,
}

/// A Project name that cannot be used, or a directory with no usable basename.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProjectError {
    #[error(transparent)]
    InvalidName(#[from] ValueError),
    #[error("Project '{name}' is reserved for Ployz infrastructure")]
    Reserved { name: ProjectName },
    #[error("no usable Project name in directory '{}'", .0.display())]
    UnusableDirectory(PathBuf),
}

/// Resolve one Project name. The first present source wins; invalid values are
/// not normalised and do not fall through.
///
/// # Errors
///
/// Returns when a present source is not a valid Project Name, or a directory
/// source has no usable basename.
pub fn resolve_project_name(input: &ProjectNameInput<'_>) -> Result<ResolvedProject, ProjectError> {
    if let Some(value) = input.command_line {
        return parsed(value, ProjectNameSource::CommandLine);
    }
    if let Some(value) = input.compose_project_name {
        return parsed(value, ProjectNameSource::ComposeProjectName);
    }
    if let Some(value) = input.compose_name {
        return parsed(value, ProjectNameSource::ComposeName);
    }
    if let Some(directory) = input.compose_project_directory {
        return from_directory(directory, ProjectNameSource::ComposeProjectDirectory);
    }
    if let Some(directory) = input.current_directory {
        return from_directory(directory, ProjectNameSource::CurrentDirectory);
    }
    if input.implicit_default {
        return parsed("default", ProjectNameSource::Default);
    }
    Err(ProjectError::UnusableDirectory(PathBuf::from(".")))
}

/// Refuse a reserved Project name on deployment and removal commands.
///
/// # Errors
///
/// Returns [`ProjectError::Reserved`] when `name` is `ployz-system`.
pub fn refuse_reserved(name: &ProjectName) -> Result<(), ProjectError> {
    if name.is_reserved() {
        Err(ProjectError::Reserved { name: name.clone() })
    } else {
        Ok(())
    }
}

/// Resolve a Project for `ployz run` / `scale`: CLI, `COMPOSE_PROJECT_NAME`, then `default`.
///
/// # Errors
///
/// Returns when a present source is not a valid Project Name.
pub fn resolve_for_run(matches: &ArgMatches) -> Result<ResolvedProject, ProjectError> {
    let (command_line, compose_project_name) = cli_sources(matches);
    resolve_project_name(&ProjectNameInput {
        command_line,
        compose_project_name,
        implicit_default: true,
        ..ProjectNameInput::default()
    })
}

/// Resolve a Project for Compose `deploy` using full precedence.
///
/// # Errors
///
/// Returns when a present source is not a valid Project Name, or the Compose
/// project directory has no usable basename.
pub fn resolve_for_deploy(
    matches: &ArgMatches,
    project: &ComposeProject,
    files: &[PathBuf],
) -> Result<ResolvedProject, ProjectError> {
    let (command_line, compose_project_name) = cli_sources(matches);
    let compose_name = top_level_compose_name(files, &project.working_dir);
    resolve_project_name(&ProjectNameInput {
        command_line,
        compose_project_name,
        compose_name: compose_name.as_deref(),
        compose_project_directory: Some(&project.working_dir),
        current_directory: None,
        implicit_default: false,
    })
}

/// Resolve a Project only when CLI or `COMPOSE_PROJECT_NAME` named one.
///
/// # Errors
///
/// Returns when a present source is not a valid Project Name.
pub fn resolve_explicit(matches: &ArgMatches) -> Result<Option<ResolvedProject>, ProjectError> {
    let (command_line, compose_project_name) = cli_sources(matches);
    if command_line.is_none() && compose_project_name.is_none() {
        return Ok(None);
    }
    resolve_project_name(&ProjectNameInput {
        command_line,
        compose_project_name,
        ..ProjectNameInput::default()
    })
    .map(Some)
}

/// Last top-level Compose `name` among the files Compose would load.
#[must_use]
pub fn top_level_compose_name(explicit_files: &[PathBuf], working_dir: &Path) -> Option<String> {
    let files = compose_files_to_scan(explicit_files, working_dir);
    let mut found = None;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let Ok(parsed) = serde_norway::from_str::<NamedCompose>(&text) else {
            continue;
        };
        if let Some(name) = parsed.name {
            found = Some(name);
        }
    }
    found
}

fn cli_sources(matches: &ArgMatches) -> (Option<&str>, Option<&str>) {
    let Ok(Some(value)) = matches.try_get_one::<String>("project-name") else {
        return (None, None);
    };
    match matches.value_source("project-name") {
        Some(ValueSource::CommandLine) => (Some(value.as_str()), None),
        Some(ValueSource::EnvVariable) => (None, Some(value.as_str())),
        _ => (None, None),
    }
}

fn parsed(value: &str, source: ProjectNameSource) -> Result<ResolvedProject, ProjectError> {
    Ok(ResolvedProject {
        name: ProjectName::parse(value)?,
        source,
    })
}

fn from_directory(
    directory: &Path,
    source: ProjectNameSource,
) -> Result<ResolvedProject, ProjectError> {
    let Some(name) = directory_basename(directory) else {
        return Err(ProjectError::UnusableDirectory(directory.to_owned()));
    };
    parsed(name, source)
}

fn directory_basename(path: &Path) -> Option<&str> {
    path.components()
        .rev()
        .find_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => None,
        })
}

fn compose_files_to_scan(explicit_files: &[PathBuf], working_dir: &Path) -> Vec<PathBuf> {
    if !explicit_files.is_empty() {
        return explicit_files.to_vec();
    }
    let from_env = compose_files_from_environment();
    if !from_env.is_empty() {
        return from_env;
    }
    discovered_compose_files(working_dir)
}

fn compose_files_from_environment() -> Vec<PathBuf> {
    let Some(files) = std::env::var_os("COMPOSE_FILE") else {
        return Vec::new();
    };
    let separator = std::env::var_os("COMPOSE_PATH_SEPARATOR")
        .filter(|separator| !separator.is_empty())
        .unwrap_or_else(|| ":".into());
    files
        .to_string_lossy()
        .split(separator.to_string_lossy().as_ref())
        .filter(|file| !file.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn discovered_compose_files(working_dir: &Path) -> Vec<PathBuf> {
    for name in [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ] {
        let path = working_dir.join(name);
        if path.is_file() {
            return vec![path];
        }
    }
    Vec::new()
}

#[derive(Deserialize)]
struct NamedCompose {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn resolved(name: &str, source: ProjectNameSource) -> ResolvedProject {
        ResolvedProject {
            name: ProjectName::parse(name).unwrap(),
            source,
        }
    }

    #[test]
    fn precedence_picks_the_first_present_source() {
        let directory = Path::new("/tmp/from-dir");
        let cwd = Path::new("/tmp/from-cwd");
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                command_line: Some("from-cli"),
                compose_project_name: Some("from-env"),
                compose_name: Some("from-compose"),
                compose_project_directory: Some(directory),
                current_directory: Some(cwd),
                implicit_default: true,
            })
            .unwrap(),
            resolved("from-cli", ProjectNameSource::CommandLine)
        );
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                compose_project_name: Some("from-env"),
                compose_name: Some("from-compose"),
                compose_project_directory: Some(directory),
                current_directory: Some(cwd),
                implicit_default: true,
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("from-env", ProjectNameSource::ComposeProjectName)
        );
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                compose_name: Some("from-compose"),
                compose_project_directory: Some(directory),
                current_directory: Some(cwd),
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("from-compose", ProjectNameSource::ComposeName)
        );
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                compose_project_directory: Some(directory),
                current_directory: Some(cwd),
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("from-dir", ProjectNameSource::ComposeProjectDirectory)
        );
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                current_directory: Some(cwd),
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("from-cwd", ProjectNameSource::CurrentDirectory)
        );
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                implicit_default: true,
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("default", ProjectNameSource::Default)
        );
    }

    #[test]
    fn present_invalid_names_are_rejected_without_falling_through() {
        let error = resolve_project_name(&ProjectNameInput {
            command_line: Some("My_App"),
            compose_name: Some("shop"),
            ..ProjectNameInput::default()
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid Project Name \"My_App\": a 1-63 character lowercase DNS label; underscores and uppercase are not accepted"
        );
        let env = resolve_project_name(&ProjectNameInput {
            compose_project_name: Some("SHOP"),
            compose_name: Some("shop"),
            ..ProjectNameInput::default()
        })
        .unwrap_err();
        assert!(env.to_string().contains("SHOP"), "{env}");
        assert!(!env.to_string().contains("shop\""), "{env}");
    }

    #[test]
    fn directory_names_are_not_normalised() {
        let error = resolve_project_name(&ProjectNameInput {
            compose_project_directory: Some(Path::new("/tmp/My_App")),
            current_directory: Some(Path::new("/tmp/shop")),
            ..ProjectNameInput::default()
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid Project Name \"My_App\": a 1-63 character lowercase DNS label; underscores and uppercase are not accepted"
        );
    }

    #[test]
    fn run_without_a_project_uses_default() {
        assert_eq!(
            resolve_project_name(&ProjectNameInput {
                implicit_default: true,
                ..ProjectNameInput::default()
            })
            .unwrap(),
            resolved("default", ProjectNameSource::Default)
        );
    }

    #[test]
    fn reserved_names_parse_and_are_refused() {
        let name = ProjectName::parse("ployz-system").unwrap();
        assert!(name.is_reserved());
        assert_eq!(
            refuse_reserved(&name).unwrap_err().to_string(),
            "Project 'ployz-system' is reserved for Ployz infrastructure"
        );
        refuse_reserved(&ProjectName::parse("shop").unwrap()).unwrap();
    }

    #[test]
    fn source_labels_match_the_plan_header() {
        assert_eq!(
            ProjectNameSource::CommandLine.to_string(),
            "command-line project name"
        );
        assert_eq!(
            ProjectNameSource::ComposeProjectName.to_string(),
            "COMPOSE_PROJECT_NAME"
        );
        assert_eq!(
            ProjectNameSource::ComposeName.to_string(),
            "top-level Compose name"
        );
        assert_eq!(
            ProjectNameSource::ComposeProjectDirectory.to_string(),
            "Compose project directory"
        );
        assert_eq!(
            ProjectNameSource::CurrentDirectory.to_string(),
            "current directory"
        );
        assert_eq!(ProjectNameSource::Default.to_string(), "default");
    }

    #[test]
    fn top_level_compose_name_uses_the_last_file() {
        let root = unique_dir();
        fs::write(root.join("a.yaml"), "name: first\nservices: {}\n").unwrap();
        fs::write(root.join("b.yaml"), "name: second\nservices: {}\n").unwrap();
        fs::write(root.join("c.yaml"), "services: {}\n").unwrap();
        assert_eq!(
            top_level_compose_name(&[root.join("a.yaml"), root.join("b.yaml")], &root).as_deref(),
            Some("second")
        );
        assert_eq!(
            top_level_compose_name(&[root.join("a.yaml"), root.join("c.yaml")], &root).as_deref(),
            Some("first")
        );
        fs::write(
            root.join("compose.yaml"),
            "name: discovered\nservices: {}\n",
        )
        .unwrap();
        assert_eq!(
            top_level_compose_name(&[], &root).as_deref(),
            Some("discovered")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relative_dot_directory_has_no_basename_of_its_own() {
        assert!(directory_basename(Path::new(".")).is_none());
        assert_eq!(directory_basename(Path::new("/tmp/shop/.")), Some("shop"));
        assert_eq!(directory_basename(Path::new("/")), None);
    }

    fn unique_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ployz-project-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
