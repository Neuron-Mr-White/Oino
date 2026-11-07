use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Status,
    BuildIndex {
        root: PathBuf,
    },
    Query {
        root: PathBuf,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        args: Option<serde_json::Map<String, Value>>,
    },
    Shutdown,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent_dir = std::env::var_os("OINO_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".oino")))
        .ok_or("Cannot find Oino home")?;
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut output = tokio::io::stdout();
    while let Ok(Ok(Some(line))) =
        tokio::time::timeout(Duration::from_secs(300), input.next_line()).await
    {
        let request = serde_json::from_str::<Request>(&line);
        let shutdown = matches!(request, Ok(Request::Shutdown));
        let result: Result<Value, String> = match request {
            Ok(Request::Status) => {
                Ok(json!({"loadedModels": oino_tools::zvec::loaded_models().await}))
            }
            Ok(Request::BuildIndex { root }) => {
                oino_tools::zvec::build_index(&root, &agent_dir, true)
                    .await
                    .map(|()| json!({}))
            }
            Ok(Request::Query {
                root,
                text,
                limit,
                args,
            }) => {
                let args = args.map(Value::Object).unwrap_or_else(
                    || json!({"pattern": text, "search_mode": "semantic", "head_limit": limit}),
                );
                match args.as_object() {
                    Some(args) => {
                        oino_tools::zvec::query(&root, &agent_dir, args, CancellationToken::new())
                            .await
                            .map(|value| json!(value))
                            .map_err(|error| error.to_string())
                    }
                    None => Err("Invalid query".into()),
                }
            }
            Ok(Request::Shutdown) => Ok(json!({})),
            Err(error) => Err(error.to_string()),
        };
        let response = match result {
            Ok(value) => json!({"ok": true, "result": value}),
            Err(error) => json!({"ok": false, "error": error}),
        };
        output
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
        if shutdown {
            break;
        }
    }
    Ok(())
}
