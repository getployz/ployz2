//! Throwaway CLI look spike. Not a product surface.
//!
//! One View, four adapters: iocraft, tabled/cliclack, compact text, JSON.

mod classic_look;
mod error;
mod iocraft_look;
mod view;

use std::io::{self, IsTerminal};

use clap::{Parser, Subcommand, ValueEnum};

use error::{SpikeError, ambiguous_api, error_view};

#[derive(Parser)]
#[command(
    name = "ployz-cli-ui-spike",
    about = "Throwaway CLI look spike. Not a product surface."
)]
struct Cli {
    #[arg(long, value_enum, default_value_t = clap::ColorChoice::Auto, global = true)]
    color: clap::ColorChoice,
    /// Visual variant. JSON ignores this.
    #[arg(long, value_enum, default_value_t = Look::Iocraft, global = true)]
    look: Look,
    #[arg(short, long, value_enum, default_value_t = Output::Text, global = true)]
    output: Output,
    /// Skip confirm on deploy.
    #[arg(long, global = true)]
    yes: bool,
    /// Shrink sleeps so captures finish immediately.
    #[arg(long, global = true)]
    instant: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Look {
    #[default]
    Iocraft,
    Classic,
    Compact,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Output {
    #[default]
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// `ployz ls` shape.
    Ls,
    /// `ployz images` shape.
    Images,
    /// Plan + confirm + progress. Stub events, no Session.
    Deploy,
    /// Ambiguous-service failure through miette.
    Error,
    /// ls + images + plan + error, no prompts.
    Catalog,
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    apply_color(cli.color);
    run(cli)?;
    Ok(())
}

fn apply_color(color: clap::ColorChoice) {
    let choice = match color {
        clap::ColorChoice::Auto => anstream::ColorChoice::Auto,
        clap::ColorChoice::Always => anstream::ColorChoice::Always,
        clap::ColorChoice::Never => anstream::ColorChoice::Never,
    };
    choice.write_global();
}

fn run(cli: Cli) -> Result<(), SpikeError> {
    match cli.output {
        Output::Json => json(&cli.command),
        Output::Text => text(cli),
    }
}

fn json(command: &Command) -> Result<(), SpikeError> {
    let payload = match command {
        Command::Ls => serde_json::to_string_pretty(&view::services())?,
        Command::Images => serde_json::to_string_pretty(&view::images())?,
        Command::Deploy => serde_json::to_string_pretty(&view::deploy_view())?,
        Command::Error => serde_json::to_string_pretty(&error_view())?,
        Command::Catalog => serde_json::to_string_pretty(&view::catalog())?,
    };
    println!("{payload}");
    Ok(())
}

fn text(cli: Cli) -> Result<(), SpikeError> {
    match cli.command {
        Command::Ls => ls(cli.look),
        Command::Images => images(cli.look),
        Command::Deploy => deploy(cli.look, cli.yes, cli.instant),
        Command::Error => Err(ambiguous_api()),
        Command::Catalog => catalog(cli.look),
    }
}

fn ls(look: Look) -> Result<(), SpikeError> {
    let rows = view::services();
    match look {
        Look::Iocraft => iocraft_look::services(rows),
        Look::Classic => classic_look::services(rows, false),
        Look::Compact => classic_look::services(rows, true),
    }
}

fn images(look: Look) -> Result<(), SpikeError> {
    let rows = view::images();
    match look {
        Look::Iocraft => iocraft_look::images(rows),
        Look::Classic => classic_look::images(rows, false),
        Look::Compact => classic_look::images(rows, true),
    }
}

fn deploy(look: Look, yes: bool, instant: bool) -> Result<(), SpikeError> {
    let plan = view::plan();
    match look {
        Look::Iocraft => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()?;
            runtime.block_on(iocraft_look::deploy(yes, instant))
        }
        Look::Classic => classic_look::deploy(&plan, false, yes, instant),
        Look::Compact => classic_look::deploy(&plan, true, yes, instant),
    }
}

fn catalog(look: Look) -> Result<(), SpikeError> {
    ls(look)?;
    images(look)?;
    match look {
        Look::Iocraft => iocraft_look::plan_view(view::plan())?,
        Look::Classic => classic_look::plan(&view::plan(), false)?,
        Look::Compact => classic_look::plan(&view::plan(), true)?,
    }
    let report = miette::Report::new(ambiguous_api());
    if io::stderr().is_terminal() {
        eprintln!("{report:?}");
    } else {
        eprintln!("{report}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_ls_is_an_array_of_service_rows() {
        let json = serde_json::to_value(view::services()).expect("serialize");
        assert_eq!(
            json.pointer("/0/service"),
            Some(&serde_json::json!("web/api"))
        );
        assert_eq!(json.pointer("/0/containers"), Some(&serde_json::json!(2)));
    }

    #[test]
    fn json_error_uses_a_type_tag() {
        let json = serde_json::to_value(error_view()).expect("serialize");
        assert_eq!(
            json.pointer("/type"),
            Some(&serde_json::json!("ambiguous_service"))
        );
        assert_eq!(json.pointer("/name"), Some(&serde_json::json!("api")));
        assert_eq!(
            json.pointer("/matches")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn json_deploy_includes_plan_and_progress() {
        let json = serde_json::to_value(view::deploy_view()).expect("serialize");
        assert_eq!(
            json.pointer("/plan/project"),
            Some(&serde_json::json!("web"))
        );
        assert_eq!(
            json.pointer("/progress")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }
}
