//! `unipi-kanboard` entry point — parse, dispatch, print.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the binary's output layer: JSON/text on stdout, errors on stderr"
)]

use clap::Parser;
use clap::error::ErrorKind;

use unipi_kanboard::cli::Cli;
use unipi_kanboard::error::Error;
use unipi_kanboard::run;

fn main() {
    // clap-level failures (an unknown subcommand, a missing argument) keep the
    // §5.5 contract under `--json`: one `{ok: false, …, kind: "usage"}` line on
    // stderr, exit 2 — the same shape `report` prints for `Error::Usage`.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            err.exit()
        }
        Err(err) => {
            if std::env::args().any(|a| a == "--json") {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err.to_string().trim().to_owned(),
                    "kind": "usage",
                });
                eprintln!("{payload}");
                std::process::exit(2);
            }
            err.exit()
        }
    };
    let code = match run::dispatch(&cli) {
        Ok(payload) => {
            if cli.json {
                match serde_json::to_string_pretty(&payload) {
                    Ok(text) => println!("{text}"),
                    Err(err) => {
                        eprintln!("unipi-kanboard: cannot serialise output: {err}");
                        std::process::exit(1);
                    }
                }
            } else {
                println!("{}", run::human(&cli, &payload));
            }
            run::exit_code(&cli, &payload)
        }
        Err(err) => {
            report(&cli, &err);
            err.exit_code()
        }
    };
    std::process::exit(code);
}

fn report(cli: &Cli, err: &Error) {
    if cli.json {
        let payload = serde_json::json!({
            "ok": false,
            "error": err.to_string(),
            "kind": match err {
                Error::Usage(_) => "usage",
                Error::NotFound(_) => "not_found",
                Error::Rule(_) => "rule",
                _ => "io",
            },
        });
        eprintln!("{payload}");
    } else {
        eprintln!("unipi-kanboard: {err}");
    }
}
