#![forbid(unsafe_code)]

use oino_extension_sdk::{
    validate_extension_manifest_json, validate_package_dir, validate_package_manifest_json,
    validate_parity_matrix, AuthoringError, ExampleExtensionTemplate,
};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("oino-extension-devkit: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), AuthoringError> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };
    match command.as_str() {
        "template-extension" => {
            let template = ExampleExtensionTemplate::complete_example()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&template.extension_manifest()?)?
            );
        }
        "template-package" => {
            let template = ExampleExtensionTemplate::complete_example()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&template.package_manifest()?)?
            );
        }
        "validate-extension" => {
            let Some(path) = args.next() else {
                return Err(AuthoringError::Validation(
                    "validate-extension requires a manifest path".into(),
                ));
            };
            let manifest = validate_extension_manifest_json(&fs::read_to_string(path)?)?;
            println!(
                "extension manifest ok: {} {}",
                manifest.id, manifest.version
            );
        }
        "validate-package" => {
            let Some(path) = args.next() else {
                return Err(AuthoringError::Validation(
                    "validate-package requires a package manifest path or package directory".into(),
                ));
            };
            let path = std::path::PathBuf::from(path);
            if path.is_dir() {
                let report = validate_package_dir(&path)?;
                println!(
                    "package directory ok: {} {} ({} extension(s))",
                    report.package.id,
                    report.package.version,
                    report.extensions.len()
                );
            } else {
                let manifest = validate_package_manifest_json(&fs::read_to_string(path)?)?;
                println!("package manifest ok: {} {}", manifest.id, manifest.version);
            }
        }
        "parity-check" => {
            let Some(path) = args.next() else {
                return Err(AuthoringError::Validation(
                    "parity-check requires a parity matrix markdown path".into(),
                ));
            };
            let report = validate_parity_matrix(&fs::read_to_string(path)?);
            if report.is_ok() {
                println!("parity matrix ok");
            } else {
                return Err(AuthoringError::Validation(format!(
                    "parity matrix gaps: {report:?}"
                )));
            }
        }
        "help" | "--help" | "-h" => print_usage(),
        other => {
            return Err(AuthoringError::Validation(format!(
                "unknown devkit command `{other}`"
            )));
        }
    }
    Ok(())
}

fn print_usage() {
    println!(
        "oino-extension-devkit commands:\n  template-extension\n  template-package\n  validate-extension <oino.extension.json>\n  validate-package <oino.package.json|package-dir>\n  parity-check <parity-matrix.md>"
    );
}
