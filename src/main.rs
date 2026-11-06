use std::process::ExitCode;

use capcheck::{Options, Verdict, check};
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Markdown,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FailOn {
    /// False claims.
    Fail,
    /// False claims, and behavior the statement does not declare.
    Undeclared,
    /// Nothing: the exit code only says the run was done.
    Never,
}

/// Checks a FHIR server's CapabilityStatement against what the server really does.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// The FHIR base URL.
    base: String,
    /// A bearer token. CAPCHECK_TOKEN works too.
    #[arg(long, env = "CAPCHECK_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Only this resource type; can be repeated.
    #[arg(long = "type", value_name = "TYPE")]
    types: Vec<String>,
    /// Requests under way at the same time.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// Also check writes on Patient. This creates and deletes resources: not for production.
    #[arg(long)]
    write: bool,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// What makes the exit code non-zero.
    #[arg(long, value_enum, default_value_t = FailOn::Fail)]
    fail_on: FailOn,
    /// List what could not be checked too.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = Options {
        base: cli.base,
        token: cli.token,
        types: cli.types,
        concurrency: cli.concurrency.max(1),
        write: cli.write,
    };
    let report = match check(&options).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    match cli.format {
        Format::Text => print!("{}", report.text(cli.verbose)),
        Format::Markdown => print!("{}", report.markdown()),
        Format::Json => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(2);
            }
        },
    }
    let failed = report.count(Verdict::Fail) > 0;
    let undeclared = report.count(Verdict::Undeclared) > 0;
    match (cli.fail_on, failed, undeclared) {
        (FailOn::Never, ..) | (FailOn::Fail, false, _) | (FailOn::Undeclared, false, false) => {
            ExitCode::SUCCESS
        }
        _ => ExitCode::FAILURE,
    }
}
