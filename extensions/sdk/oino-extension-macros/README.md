<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-extension-macros — proc macros for the Oino guest SDK

Companion crate of [`oino-extension`](https://crates.io/crates/oino-extension): attribute
macros that turn plain `async fn`s on an extension type into Oino extension declarations.
You normally depend on it indirectly through `oino-extension` (which re-exports the macros);
add a direct dependency only when configuring macro wiring yourself.

## `#[tool]`

On an `async fn name(&'static self, args: A, cx: ToolCtx) -> Result<ToolResult>` inside an
`impl` block it generates a sibling `fn name_tool() -> Tool<Self>` whose description is the
doc comment and whose parameters come from `A: JsonSchema`. Optional arguments:
`name = "..."` (default: fn name), `label = "..."`, `kind = Read|Edit|…`.

```rust,ignore
use oino_extension::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct NotifyArgs {
    /// Message body.
    message: String,
}

impl MyExtension {
    /// Send a notification to the user.
    #[oino_extension::tool(label = "Notify user", kind = Other)]
    async fn notify_user(
        &'static self,
        args: NotifyArgs,
        cx: ToolCtx,
    ) -> Result<ToolResult> {
        Ok(ToolResult::text(args.message))
    }
}

impl Extension for MyExtension {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.tool(Self::notify_user_tool());
        Ok(())
    }
}
```
