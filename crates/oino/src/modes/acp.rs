//! `--mode acp`: an Agent Client Protocol agent on stdio ([`oino_acp::serve_stdio`]).
//!
//! Every ACP session gets the same extension set as the other modes (bundled, installed and
//! `-e` packages, see [`crate::loader`]); services are shared per working directory. Logs and
//! diagnostics go to stderr because stdout carries the protocol.

use std::path::PathBuf;
use std::sync::Arc;

use oino_core::extensions::api::ExtensionMode;
use oino_core::services::{ExtensionLoader, ServicesOptions};

use crate::cli::args::Args;
use crate::loader::{CliLoader, LoaderContext};
use crate::modes::signals;
use crate::startup::{Environment, acp_model_choice, docs_root, resource_flags, tool_selection};

/// Serves ACP until the client disconnects (exit 0) or a signal arrives.
pub async fn run(args: &Args, env: &Environment) -> i32 {
    let agent_dir = env.agent_dir.clone();
    let docs = docs_root(&agent_dir);
    let sources = args.extensions.clone().unwrap_or_default();
    let flags = resource_flags(args, &env.cwd);
    let values = args.unknown_flags.clone();
    let approve = args.project_trust_override;
    let no_extensions = args.no_extensions;
    let model_args = Arc::new(args.clone());
    let factory = oino_acp::CoreSessionFactory::new(
        move |cwd| {
            let loader = CliLoader::new(
                sources.clone(),
                &LoaderContext {
                    agent_dir: agent_dir.clone(),
                    cwd: cwd.to_path_buf(),
                    no_extensions,
                },
            );
            for line in crate::report_lines(loader.diagnostics()) {
                crate::eprintln_line(&line);
            }
            ServicesOptions {
                cwd: cwd.to_path_buf(),
                agent_dir: agent_dir.clone(),
                docs_root: docs.clone(),
                mode: ExtensionMode::Acp,
                has_ui: true,
                flags: flags.clone(),
                extension_flag_values: values.clone(),
                loader: Arc::new(loader) as Arc<dyn ExtensionLoader>,
                approve,
                models: None,
                settings: None,
            }
        },
        oino_acp::SessionDefaults {
            session_dir: args.session_dir.as_ref().map(PathBuf::from),
            model: None,
            thinking_level: args.thinking,
            tools: tool_selection(args),
            exclude_tools: args.exclude_tools.clone().unwrap_or_default(),
            model_resolver: Some(oino_acp::ModelResolver(Arc::new(move |services| {
                acp_model_choice(&model_args, services)
            }))),
        },
    );
    let info = oino_acp::AgentInfo {
        version: crate::VERSION.to_owned(),
        ..oino_acp::AgentInfo::default()
    };
    tokio::select! {
        result = oino_acp::serve_stdio(Arc::new(factory), info, env.cwd.clone()) => match result {
            Ok(()) => 0,
            Err(error) => {
                crate::eprintln_line(&format!("Error: {error}"));
                1
            }
        },
        signal = signals::next() => signal.exit_code(),
    }
}
