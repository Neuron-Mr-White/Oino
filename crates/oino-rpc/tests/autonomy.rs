//! Real JSONL approval capability negotiation and correlation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use futures::{StreamExt, future::BoxFuture, stream::BoxStream};
use oino_ai::JsonValue;
use oino_client::{ClientError, RpcCommand, RpcResponseData, SessionBackend};
use oino_core::{
    events::ClientEvent,
    session::{
        AgentSession,
        permission::{ApprovalResponse, Level},
    },
    testing::{FauxProvider, TestSession},
    ui::UiResponse,
};
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
struct Backend(AgentSession);
impl SessionBackend for Backend {
    fn send(&self, _: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        Box::pin(async { Ok(None) })
    }
    fn events(&self) -> BoxStream<'static, ClientEvent> {
        self.0.subscribe().boxed()
    }
    fn ui_respond(&self, _: UiResponse) {}
    fn set_approval_ui(&self, v: bool) {
        self.0.set_approval_ui(v);
    }
    fn set_autonomy(&self, v: Level) -> Result<(), ClientError> {
        self.0.set_autonomy(v);
        Ok(())
    }
    fn approval_response(&self, r: ApprovalResponse) -> Result<(), ClientError> {
        self.0.approval_response(r).map_err(ClientError::Command)
    }
    fn session_id(&self) -> String {
        self.0.session_id()
    }
    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
#[tokio::test]
async fn rpc_approval_capability_and_single_use_response() {
    let session = TestSession::default()
        .build(FauxProvider::new(vec![]))
        .await;
    let backend: Arc<dyn SessionBackend> = Arc::new(Backend(session.clone()));
    let (mut input, server_in) = tokio::io::duplex(65536);
    let (server_out, output) = tokio::io::duplex(65536);
    let server = tokio::spawn(async move {
        oino_rpc::RpcServer::new(backend)
            .run(server_in, server_out)
            .await
            .unwrap()
    });
    let mut lines = BufReader::new(output).lines();
    input
        .write_all(b"{\"id\":1,\"type\":\"_oino/hello\",\"client\":{\"name\":\"plain\"}}\n")
        .await
        .unwrap();
    lines.next_line().await.unwrap().unwrap();
    let args = json!({"command":"git push","summary":"x","riskLevel":"low"})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(
        session.check_permission("bash", &args).await.unwrap_err(),
        "needs Autonomy High"
    );
    input.write_all(b"{\"id\":2,\"type\":\"_oino/hello\",\"client\":{\"name\":\"approval-ui\"},\"ui\":{\"approvals\":true}}\n").await.unwrap();
    lines.next_line().await.unwrap().unwrap();
    let run = {
        let s = session.clone();
        let args = args.clone();
        tokio::spawn(async move { s.check_permission("bash", &args).await })
    };
    let event: JsonValue = serde_json::from_str(
        &tokio::time::timeout(std::time::Duration::from_secs(2), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(event["type"], "approval_request");
    let response = json!({"id":3,"type":"approval_response","requestId":event["requestId"],"decision":"allow_once"});
    input
        .write_all(format!("{response}\n").as_bytes())
        .await
        .unwrap();
    let reply: JsonValue =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["success"], true);
    run.await.unwrap().unwrap();
    input
        .write_all(format!("{response}\n").as_bytes())
        .await
        .unwrap();
    let reply: JsonValue =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["success"], false);
    drop(input);
    server.await.unwrap();
    assert_eq!(
        session.check_permission("bash", &args).await.unwrap_err(),
        "needs Autonomy High"
    );
}
