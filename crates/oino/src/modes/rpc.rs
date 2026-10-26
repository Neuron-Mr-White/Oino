//! pi `rpc-mode.ts` on stdio: [`oino_rpc::RpcServer`] over an in-process backend.

use std::sync::Arc;

use oino_client::{InProcessBackend, SessionBackend};
use oino_core::runtime::AgentSessionRuntime;
use oino_rpc::RpcServer;

use crate::modes::signals;

/// Serves RPC on stdin/stdout until stdin ends (exit 0) or a signal arrives.
pub async fn run(runtime: Arc<AgentSessionRuntime>) -> i32 {
    let backend = Arc::new(InProcessBackend::new(runtime));
    let server = RpcServer::new(Arc::clone(&backend) as Arc<dyn SessionBackend>);
    tokio::select! {
        result = server.run(tokio::io::stdin(), tokio::io::stdout()) => match result {
            Ok(()) => 0,
            Err(error) => {
                crate::eprintln_line(&format!("Error: {error}"));
                1
            }
        },
        signal = signals::next() => {
            backend.shutdown().await;
            signal.exit_code()
        }
    }
}
