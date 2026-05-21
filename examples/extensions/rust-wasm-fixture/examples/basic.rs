use oino_extension_sdk::WasmSdk;
use serde_json::json;

fn main() {
    let output = WasmSdk::tool_success(json!({ "message": "hello" }));
    println!("{}", output.output);
}
